use crate::config::{Correction, Track};
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug)]
pub struct Fps {
    pub num: i64,
    pub den: i64,
}
impl Fps {
    pub fn parse(s: &str) -> Result<Self> {
        let (n, d) = s.split_once('/').unwrap_or((s, "1"));
        let result = Self {
            num: n.parse()?,
            den: d.parse()?,
        };
        ensure!(
            result.num > 0 && result.den > 0 && result.num <= 1_000_000 && result.den <= 1_000_000,
            "Invalid FPS: {s}"
        );
        Ok(result)
    }
    pub fn frame(&self, ms: i64, end: bool) -> i64 {
        // MKV PTS are rounded to milliseconds before timestamp lookup.
        let n = (2 * ms as i128 - 1) * self.num as i128;
        let d = 2 * self.den as i128 * 1000;
        if end {
            ((n + d - 1).div_euclid(d) - 1).max(0) as i64
        } else {
            ((n + d - 1).div_euclid(d)).max(0) as i64
        }
    }
    pub fn millis(&self, frame: i64) -> i64 {
        ((frame as i128 * self.den as i128 * 1000 + self.num as i128 / 2) / self.num as i128) as i64
    }
    // Aegisub start: midpoint before frame; end: midpoint after frame.
    pub fn subtitle_ms(&self, frame: i64, end: bool) -> i64 {
        if frame == 0 && !end {
            return 0;
        }
        let other = if end { frame + 1 } else { frame - 1 };
        ((self.millis(frame) as i128 + self.millis(other) as i128 + 10) / 20 * 10).max(0) as i64
    }
}
pub fn parse_time(s: &str) -> Result<i64> {
    let parts: Vec<_> = s.trim().split(':').collect();
    ensure!(parts.len() == 3, "Invalid timestamp: {s}");
    let (sec, frac) = parts[2]
        .split_once(['.', ','])
        .context("Timestamp has no fractional part")?;
    ensure!(
        !frac.is_empty() && frac.len() <= 9 && frac.bytes().all(|c| c.is_ascii_digit()),
        "Invalid fractional part: {s}"
    );
    let fraction = format!("{frac:0<3}");
    let h: i64 = parts[0].parse()?;
    let m: i64 = parts[1].parse()?;
    let sec: i64 = sec.parse()?;
    ensure!(
        h >= 0 && (0..60).contains(&m) && (0..60).contains(&sec),
        "Invalid timestamp: {s}"
    );
    Ok((h * 3600 + m * 60 + sec) * 1000 + fraction[..3].parse::<i64>()?)
}
pub fn time(ms: i64, ass: bool) -> String {
    let ms = ms.max(0);
    let s = ms / 1000;
    if ass {
        format!(
            "{}:{:02}:{:02}.{:02}",
            s / 3600,
            s / 60 % 60,
            s % 60,
            ms % 1000 / 10
        )
    } else {
        format!(
            "{:02}:{:02}:{:02},{:03}",
            s / 3600,
            s / 60 % 60,
            s % 60,
            ms % 1000
        )
    }
}
#[derive(Debug, Clone)]
pub struct Cue {
    pub start: i64,
    pub end: i64,
    pub text: String,
}
pub fn parse_srt(s: &str) -> Result<Vec<Cue>> {
    let normalized = s.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let split = regex::Regex::new(r"\n[ \t]*\n+")?;
    let mut cues = Vec::new();
    for block in split
        .split(normalized.trim())
        .filter(|s| !s.trim().is_empty())
    {
        let mut lines = block.lines();
        let first = lines.next().unwrap();
        let stamp = if first.contains("-->") {
            first
        } else {
            first.trim().parse::<u64>().context("Invalid SRT index")?;
            lines.next().context("SRT has no timestamp")?
        };
        let (start, end) = stamp.split_once("-->").context("SRT has no -->")?;
        let start = parse_time(start)?;
        let end = parse_time(end.trim())?;
        ensure!(end >= start, "SRT ends before it starts");
        let text = lines.collect::<Vec<_>>().join("\n");
        ensure!(!text.is_empty(), "Empty SRT block");
        cues.push(Cue { start, end, text });
    }
    Ok(cues)
}
pub fn dump_srt(cues: &[Cue]) -> String {
    cues.iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                time(c.start, false),
                time(c.end, false),
                c.text
            )
        })
        .collect()
}
fn move_cue(c: &mut Cue, frames: i64, fps: Fps) {
    if frames == 0 {
        return;
    }
    let a = fps.frame(c.start, false) + frames;
    let b = fps.frame(c.end, true) + frames;
    if a < 0 || b < 0 {
        c.start = 0;
        c.end = 0;
        return;
    }
    c.start = fps.subtitle_ms(a, false);
    c.end = fps.subtitle_ms(b, true);
}
fn correct(c: &mut Cue, correction: &Correction, extra: i64, fps: Fps) {
    let frame = fps.frame(c.start, false);
    let shift = extra
        + correction.shift
        + correction
            .ranges
            .iter()
            .filter(|r| r.contains(frame))
            .map(|r| r.shift)
            .sum::<i64>();
    move_cue(c, shift, fps);
}
fn targets(eps: &[String], ep: &str) -> bool {
    eps.is_empty() || eps.iter().any(|e| e == ep)
}
pub fn process_srt(
    input: &str,
    t: &Track,
    correction: &Correction,
    ep: &str,
    fps: Fps,
    cutoff: Option<(i64, bool)>,
) -> Result<String> {
    let mut cues = Vec::new();
    for mut c in parse_srt(input)? {
        correct(&mut c, correction, t.shift_frames, fps);
        cues.push(c);
    }
    for deletion in &t.deletes {
        if targets(&deletion.episodes, ep) {
            cues.retain(|c| {
                let f = fps.frame(c.start, false);
                f < deletion.start || deletion.end.is_some_and(|end| f >= end)
            });
        }
    }
    for addition in &t.additions {
        if targets(&addition.episodes, ep) {
            for line in &addition.lines {
                cues.extend(parse_srt(line)?);
            }
            cues.sort_by_key(|c| c.start);
        }
    }
    if let Some((end, trimmed)) = cutoff {
        cues.retain_mut(|c| {
            if trimmed {
                if c.start >= end {
                    return false;
                }
                c.end = c.end.min(end);
            } else if c.end > end {
                return false;
            }
            c.end > c.start && c.end > 0
        });
    }
    Ok(dump_srt(&cues))
}

#[derive(Debug, Clone)]
pub struct Ass {
    pub lines: Vec<String>,
    pub style_format: Vec<String>,
    pub event_format: Vec<String>,
}
fn fields(s: &str) -> Vec<String> {
    s.split(',').map(|s| s.trim().to_owned()).collect()
}
fn index(fmt: &[String], key: &str) -> Result<usize> {
    fmt.iter()
        .position(|f| f.eq_ignore_ascii_case(key))
        .with_context(|| format!("Missing ASS field: {key}"))
}
impl Ass {
    pub fn parse(s: &str) -> Result<Self> {
        let lines: Vec<_> = s
            .trim_start_matches('\u{feff}')
            .lines()
            .map(|s| s.trim_end_matches('\r').to_owned())
            .collect();
        let mut style_format = Vec::new();
        let mut event_format = Vec::new();
        let mut section = "";
        for line in &lines {
            if line.starts_with('[') {
                section = line.trim();
            }
            if let Some(f) = line.strip_prefix("Format:") {
                if section == "[V4+ Styles]" {
                    style_format = fields(f);
                }
                if section == "[Events]" {
                    event_format = fields(f);
                }
            }
        }
        ensure!(
            !style_format.is_empty() && !event_format.is_empty(),
            "ASS requires V4+ Styles and Events with Format"
        );
        ensure!(
            event_format
                .last()
                .is_some_and(|s| s.eq_ignore_ascii_case("Text")),
            "Text must be the last ASS field"
        );
        for field in ["Name", "Fontname", "Bold", "Italic"] {
            index(&style_format, field)?;
        }
        for field in ["Start", "End", "Style", "Text"] {
            index(&event_format, field)?;
        }
        Ok(Self {
            lines,
            style_format,
            event_format,
        })
    }
    pub fn event(&self, line: &str) -> Result<Vec<String>> {
        let raw = line
            .split_once(':')
            .context("Invalid ASS event")?
            .1
            .trim_start();
        let fields: Vec<_> = raw
            .splitn(self.event_format.len(), ',')
            .map(str::to_owned)
            .collect();
        ensure!(
            fields.len() == self.event_format.len(),
            "Incomplete ASS event: {line}"
        );
        Ok(fields)
    }
    pub fn styles(&self) -> Result<Vec<BTreeMap<String, String>>> {
        let mut result = Vec::new();
        for line in &self.lines {
            if let Some(raw) = line.strip_prefix("Style:") {
                let f = fields(raw);
                ensure!(
                    f.len() == self.style_format.len(),
                    "Incomplete ASS style: {line}"
                );
                result.push(self.style_format.iter().cloned().zip(f).collect());
            }
        }
        Ok(result)
    }
    pub fn dump(&self) -> String {
        self.lines.join("\n") + "\n"
    }
    pub fn set_headers(&mut self, headers: &BTreeMap<String, String>) -> Result<()> {
        let start = self
            .lines
            .iter()
            .position(|s| s == "[Script Info]")
            .context("ASS has no Script Info")?;
        for (key, value) in headers {
            ensure!(
                !key.contains(['\n', '\r', ':']) && !value.contains(['\n', '\r']),
                "Invalid header"
            );
            let end = self
                .lines
                .iter()
                .enumerate()
                .skip(start + 1)
                .find(|(_, s)| s.starts_with('['))
                .map(|(i, _)| i)
                .unwrap_or(self.lines.len());
            if let Some(i) = (start + 1..end).find(|&i| {
                self.lines[i]
                    .split_once(':')
                    .is_some_and(|(k, _)| k.trim() == key)
            }) {
                self.lines[i] = format!("{key}: {value}");
            } else {
                self.lines.insert(end, format!("{key}: {value}"));
            }
        }
        Ok(())
    }
}
pub fn process_ass(
    input: &str,
    t: &Track,
    correction: &Correction,
    ep: &str,
    fps: Fps,
    cutoff: Option<(i64, bool)>,
) -> Result<Ass> {
    let mut doc = Ass::parse(input)?;
    doc.set_headers(&t.headers)?;
    let a = index(&doc.event_format, "Start")?;
    let b = index(&doc.event_format, "End")?;
    let txt = index(&doc.event_format, "Text")?;
    let mut lines = Vec::new();
    for line in &doc.lines {
        if line.starts_with("Dialogue:") || line.starts_with("Comment:") {
            let mut f = doc.event(line)?;
            let mut cue = Cue {
                start: parse_time(&f[a])?,
                end: parse_time(&f[b])?,
                text: f[txt].clone(),
            };
            ensure!(cue.end >= cue.start, "ASS ends before it starts");
            correct(&mut cue, correction, t.shift_frames, fps);
            f[a] = time(cue.start, true);
            f[b] = time(cue.end, true);
            lines.push(format!(
                "{}: {}",
                line.split_once(':').unwrap().0,
                f.join(",")
            ));
        } else {
            lines.push(line.clone());
        }
    }
    doc.lines = lines;
    for deletion in &t.deletes {
        if !targets(&deletion.episodes, ep) {
            continue;
        }
        let mut updated = Vec::new();
        for line in &doc.lines {
            if line.starts_with("Dialogue:") || line.starts_with("Comment:") {
                let fields = doc.event(line)?;
                let frame = fps.frame(parse_time(&fields[a])?, false);
                if frame >= deletion.start && deletion.end.is_none_or(|end| frame < end) {
                    continue;
                }
            }
            updated.push(line.clone());
        }
        doc.lines = updated;
    }
    for addition in &t.additions {
        if !targets(&addition.episodes, ep) {
            continue;
        }
        let start = doc
            .lines
            .iter()
            .position(|line| line == "[Events]")
            .context("ASS has no Events section")?;
        let end = doc
            .lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, line)| line.starts_with('['))
            .map(|(index, _)| index)
            .unwrap_or(doc.lines.len());
        for line in &addition.lines {
            ensure!(
                line.starts_with("Dialogue:") && !line.contains(['\n', '\r']),
                "Added ASS entry must be a Dialogue line"
            );
            let fields = doc.event(line)?;
            ensure!(
                parse_time(&fields[b])? >= parse_time(&fields[a])?,
                "Added ASS line has an invalid timestamp"
            );
        }
        doc.lines.splice(end..end, addition.lines.clone());
        let mut events = doc
            .lines
            .iter()
            .filter(|line| line.starts_with("Dialogue:") || line.starts_with("Comment:"))
            .map(|line| Ok((parse_time(&doc.event(line)?[a])?, line.clone())))
            .collect::<Result<Vec<_>>>()?;
        events.sort_by_key(|(start, _)| *start);
        let mut sorted = events.into_iter();
        for line in &mut doc.lines {
            if line.starts_with("Dialogue:") || line.starts_with("Comment:") {
                *line = sorted.next().unwrap().1;
            }
        }
    }
    if let Some((end, trimmed)) = cutoff {
        // ASS stores event timestamps in centiseconds, so compare against the
        // last representable endpoint at or before the video end.
        let end = end / 10 * 10;
        let mut kept = Vec::new();
        for line in &doc.lines {
            if line.starts_with("Dialogue:") || line.starts_with("Comment:") {
                let mut f = doc.event(line)?;
                let start = parse_time(&f[a])?;
                let mut finish = parse_time(&f[b])?;
                if trimmed {
                    if start >= end {
                        continue;
                    }
                    finish = finish.min(end);
                    if finish <= start || finish <= 0 {
                        continue;
                    }
                    f[b] = time(finish, true);
                    kept.push(format!(
                        "{}: {}",
                        line.split_once(':').unwrap().0,
                        f.join(",")
                    ));
                } else if finish <= end && finish > start && finish > 0 {
                    kept.push(line.clone());
                }
            } else {
                kept.push(line.clone());
            }
        }
        doc.lines = kept;
    }
    let mut used = BTreeSet::new();
    let sty = index(&doc.event_format, "Style")?;
    let resets = regex::Regex::new(r"\\r([^\\}]+)")?;
    for line in &doc.lines {
        if line.starts_with("Dialogue:") {
            let f = doc.event(line)?;
            used.insert(f[sty].trim_start_matches([' ', '\t', '*']).to_owned());
            for c in resets.captures_iter(&f[txt]) {
                used.insert(c[1].to_owned());
            }
        }
    }
    let mut section = String::new();
    doc.lines.retain(|line| {
        if line.starts_with('[') {
            section = line.clone();
        }
        if t.clean.iter().any(|s| s == "garbage") && section == "[Aegisub Project Garbage]" {
            return false;
        }
        if t.clean.iter().any(|s| s == "extradata") && section == "[Aegisub Extradata]" {
            return false;
        }
        if t.clean.iter().any(|s| s == "comments") && line.starts_with("Comment:") {
            return false;
        }
        if t.clean.iter().any(|s| s == "styles") && line.starts_with("Style:") {
            let values = fields(line.split_once(':').unwrap().1);
            return values
                .get(index(&doc.style_format, "Name").unwrap())
                .is_some_and(|s| used.contains(s));
        }
        true
    });
    Ok(doc)
}
