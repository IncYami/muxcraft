use crate::subtitle::Fps;
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn xml(chapters: &[(i64, String)], language: &str) -> Result<String> {
    ensure!(!chapters.is_empty(), "No chapters found");
    let mut result =
        String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Chapters><EditionEntry>");
    let mut last = -1;
    for (ms, name) in chapters {
        ensure!(
            *ms >= 0 && *ms > last,
            "Chapters must be ascending and nonnegative"
        );
        last = *ms;
        let seconds = ms / 1000;
        let time = format!(
            "{:02}:{:02}:{:02}.{:03}000000",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
            ms % 1000
        );
        result.push_str(&format!("<ChapterAtom><ChapterTimeStart>{time}</ChapterTimeStart><ChapterDisplay><ChapterString>{}</ChapterString><ChapterLanguage>{}</ChapterLanguage></ChapterDisplay></ChapterAtom>",escape(name),escape(language)));
    }
    result.push_str("</EditionEntry></Chapters>\n");
    Ok(result)
}
pub fn ogm(chapters: &[(i64, String)]) -> Result<String> {
    ensure!(!chapters.is_empty(), "No chapters found");
    let mut output = String::new();
    let mut last = -1;
    for (i, (ms, name)) in chapters.iter().enumerate() {
        ensure!(
            *ms >= 0 && *ms > last,
            "Chapters must be ascending and nonnegative"
        );
        ensure!(
            !name.contains(['\n', '\r']),
            "Chapter names cannot contain line breaks"
        );
        last = *ms;
        let seconds = ms / 1000;
        output.push_str(&format!(
            "CHAPTER{:02}={:02}:{:02}:{:02}.{:03}\nCHAPTER{:02}NAME={}\n",
            i + 1,
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
            ms % 1000,
            i + 1,
            name
        ));
    }
    Ok(output)
}
fn u16_at(b: &[u8], p: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(
        b.get(p..p + 2)
            .context("Truncated MPLS (u16)")?
            .try_into()?,
    ))
}
fn u32_at(b: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        b.get(p..p + 4)
            .context("Truncated MPLS (u32)")?
            .try_into()?,
    ))
}
#[derive(Debug, Clone)]
pub struct PlaylistCutMarks {
    pub input_ticks: u32,
    pub output_ticks: u32,
    pub marks: Vec<u32>,
}

pub fn discover_cut_playlist(source: &Path) -> Result<PathBuf> {
    let stream = source.parent().context("M2TS has no STREAM directory")?;
    ensure!(
        stream.file_name().is_some_and(|name| name == "STREAM"),
        "Automatic cut playlist lookup requires BDMV/STREAM: {}",
        source.display()
    );
    let directory = stream
        .parent()
        .context("STREAM has no BDMV directory")?
        .join("PLAYLIST");
    let clip = source
        .file_stem()
        .context("M2TS has no clip name")?
        .to_string_lossy();
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(&directory)
        .with_context(|| format!("PLAYLIST directory not found: {}", directory.display()))?
    {
        let path = entry?.path();
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mpls"))
        {
            continue;
        }
        if let Ok(marks) = cut_marks(&std::fs::read(&path)?, &clip) {
            candidates.push((path, marks));
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    let (selected, reference) = candidates.first().with_context(|| {
        format!(
            "No single-clip MPLS playlist references {}",
            source.display()
        )
    })?;
    ensure!(
        candidates.iter().all(|(_, marks)| {
            marks.input_ticks == reference.input_ticks
                && marks.output_ticks == reference.output_ticks
                && marks.marks == reference.marks
        }),
        "Several playlists reference {} with different chapter timings: {:?}",
        source.display(),
        candidates.iter().map(|(path, _)| path).collect::<Vec<_>>()
    );
    Ok(selected.clone())
}

pub fn cut_marks(b: &[u8], clip: &str) -> Result<PlaylistCutMarks> {
    ensure!(b.get(..4) == Some(b"MPLS"), "Invalid MPLS signature");
    let playlist = u32_at(b, 8)? as usize;
    let marks_offset = u32_at(b, 12)? as usize;
    let count = u16_at(b, playlist + 6)? as usize;
    ensure!(
        count == 1,
        "Cuts require a playlist with exactly one PlayItem"
    );
    let item = playlist + 10;
    let len = u16_at(b, item)? as usize;
    ensure!(
        len >= 20 && item + 2 + len <= b.len(),
        "Invalid MPLS PlayItem"
    );
    let start = item + 2;
    ensure!(
        b.get(start..start + 5) == Some(clip.as_bytes()),
        "Playlist does not reference clip {clip}"
    );
    ensure!(
        b.get(start + 5..start + 9) == Some(b"M2TS"),
        "Playlist item is not M2TS"
    );
    let input_ticks = u32_at(b, start + 12)?;
    let output_ticks = u32_at(b, start + 16)?;
    ensure!(output_ticks > input_ticks, "Invalid MPLS PlayItem interval");
    let mark_count = u16_at(b, marks_offset + 4)? as usize;
    let mut marks = Vec::new();
    for i in 0..mark_count {
        let pos = marks_offset + 6 + i * 14;
        if b.get(pos + 1) == Some(&1) && u16_at(b, pos + 2)? == 0 {
            let tick = u32_at(b, pos + 4)?;
            if tick >= input_ticks && tick < output_ticks {
                marks.push(tick);
            }
        }
    }
    marks.sort_unstable();
    marks.dedup();
    ensure!(!marks.is_empty(), "Playlist has no entry marks");
    Ok(PlaylistCutMarks {
        input_ticks,
        output_ticks,
        marks,
    })
}
#[derive(Debug, PartialEq, Eq)]
pub struct PlaylistChapters {
    pub times: Vec<i64>,
    pub duration_ms: i64,
    pub duration_frames: i64,
    pub in_ms: i64,
}
pub fn parse_mpls(
    b: &[u8],
    clip: &str,
    fps: Fps,
    video_frames: Option<i64>,
) -> Result<Option<PlaylistChapters>> {
    ensure!(b.get(..4) == Some(b"MPLS"), "Invalid MPLS signature");
    let p = u32_at(b, 8)? as usize;
    let marks = u32_at(b, 12)? as usize;
    let count = u16_at(b, p + 6)? as usize;
    let mut pos = p + 10;
    let mut items = Vec::new();
    for i in 0..count {
        let len = u16_at(b, pos)? as usize;
        ensure!(len >= 20, "MPLS PlayItem is too short");
        let start = pos + 2;
        let name = std::str::from_utf8(b.get(start..start + 5).context("Truncated MPLS clip")?)?;
        let codec = b
            .get(start + 5..start + 9)
            .context("Truncated MPLS codec")?;
        let input = u32_at(b, start + 12)?;
        let output = u32_at(b, start + 16)?;
        if name == clip && codec == b"M2TS" {
            ensure!(output >= input, "MPLS out < in");
            items.push((i, input, output));
        }
        pos = pos.checked_add(len + 2).context("MPLS overflow")?;
        ensure!(pos <= b.len(), "Truncated MPLS");
    }
    if items.is_empty() {
        return Ok(None);
    }
    ensure!(
        items.len() == 1,
        "Clip repeats in playlist; specify chapters explicitly"
    );
    let (item, input, output) = items[0];
    let count = u16_at(b, marks + 4)? as usize;
    let mut timestamps = Vec::new();
    for i in 0..count {
        let p = marks + 6 + i * 14;
        let kind = *b.get(p + 1).context("Truncated MPLS mark")?;
        if kind == 1 && u16_at(b, p + 2)? as usize == item {
            timestamps.push(u32_at(b, p + 4)?);
        }
    }
    if timestamps.is_empty() {
        return Ok(None);
    }
    timestamps.sort_unstable();
    timestamps.dedup();
    let offset = input.min(timestamps[0]);
    let playlist_frames =
        ((output - input) as i128 * fps.num as i128 / (45000 * fps.den as i128)) as i64;
    let cutoff_frames = video_frames.unwrap_or(playlist_frames);
    ensure!(cutoff_frames > 0, "Invalid chapter cutoff frame count");
    let mut times = Vec::new();
    for mark in timestamps {
        let elapsed_ticks = (mark - offset) as i128;
        let frame = (elapsed_ticks * fps.num as i128 / (45000 * fps.den as i128)) as i64;
        if frame > cutoff_frames - 50 {
            continue;
        }
        // Keep the MPLS mark time instead of snapping it to a video frame.
        let ms = ((elapsed_ticks * 1000 + 22500) / 45000) as i64;
        if times.last() != Some(&ms) {
            times.push(ms);
        }
    }
    Ok(Some(PlaylistChapters {
        times,
        duration_ms: (((output - input) as i128 * 1000 + 22500) / 45000) as i64,
        duration_frames: playlist_frames,
        in_ms: input as i64 * 1000 / 45000,
    }))
}
pub fn bdmv(
    source: &Path,
    playlist: Option<&Path>,
    fps: Fps,
    video_frames: Option<i64>,
) -> Result<(PathBuf, PlaylistChapters)> {
    let stem = source
        .file_stem()
        .context("Source has no file stem")?
        .to_string_lossy();
    let mut candidates = Vec::new();
    if let Some(p) = playlist {
        candidates.push(p.to_owned());
    } else {
        let stream = source.parent().context("Source has no parent directory")?;
        ensure!(
            stream
                .file_name()
                .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("STREAM")),
            "Source is not in BDMV/STREAM"
        );
        let dir = stream.parent().unwrap().join("PLAYLIST");
        for entry in std::fs::read_dir(&dir)
            .with_context(|| format!("PLAYLIST not found: {}", dir.display()))?
        {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("mpls"))
            {
                candidates.push(path);
            }
        }
    }
    candidates.sort();
    let mut matches = Vec::new();
    for p in candidates {
        if let Some(ch) = parse_mpls(&std::fs::read(&p)?, &stem, fps, video_frames)
            .with_context(|| format!("Playlist {}", p.display()))?
        {
            matches.push((p, ch));
        }
    }
    ensure!(
        !matches.is_empty(),
        "No playlist contains chapters for clip {stem}"
    );
    if matches.iter().skip(1).any(|(_, c)| c != &matches[0].1) {
        bail!(
            "Playlists have different chapters for {stem}; specify chapters.playlist: {:?}",
            matches.iter().map(|(p, _)| p).collect::<Vec<_>>()
        );
    }
    Ok(matches.remove(0))
}

/// Find a playlist whose PlayItems exactly match the ordered BDMV clips.
/// The same source clips may appear in other playlists in a different order.
pub fn bdmv_joined(sources: &[PathBuf], fps: Fps) -> Result<Option<(PathBuf, PlaylistChapters)>> {
    if sources.len() < 2 {
        return Ok(None);
    }
    let stream = sources[0]
        .parent()
        .context("Source has no parent directory")?;
    if !stream.file_name().is_some_and(|name| name == "STREAM")
        || sources.iter().any(|source| {
            source.parent() != Some(stream)
                || !source
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("m2ts"))
        })
    {
        return Ok(None);
    }
    let clips = sources
        .iter()
        .map(|source| {
            source
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .context("Source has no clip name")
        })
        .collect::<Result<Vec<_>>>()?;
    let directory = stream
        .parent()
        .context("STREAM has no BDMV directory")?
        .join("PLAYLIST");
    if !directory.is_dir() {
        return Ok(None);
    }
    let mut matches = Vec::new();
    for entry in std::fs::read_dir(&directory)? {
        let path = entry?.path();
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mpls"))
        {
            continue;
        }
        if let Some(chapters) = parse_joined_mpls(&std::fs::read(&path)?, &clips, fps)
            .with_context(|| format!("Playlist {}", path.display()))?
        {
            matches.push((path, chapters));
        }
    }
    matches.sort_by(|a, b| a.0.cmp(&b.0));
    if matches.is_empty() {
        return Ok(None);
    }
    if matches
        .iter()
        .skip(1)
        .any(|(_, chapters)| chapters != &matches[0].1)
    {
        bail!(
            "Playlists have different chapters for joined clips: {:?}; configure [chapters] explicitly",
            matches.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );
    }
    Ok(Some(matches.remove(0)))
}

fn parse_joined_mpls(b: &[u8], clips: &[String], fps: Fps) -> Result<Option<PlaylistChapters>> {
    ensure!(b.get(..4) == Some(b"MPLS"), "Invalid MPLS signature");
    let playlist = u32_at(b, 8)? as usize;
    let marks_offset = u32_at(b, 12)? as usize;
    let count = u16_at(b, playlist + 6)? as usize;
    if count != clips.len() {
        return Ok(None);
    }
    let mut pos = playlist + 10;
    let mut items = Vec::with_capacity(count);
    let mut elapsed = 0_i128;
    for clip in clips {
        let len = u16_at(b, pos)? as usize;
        ensure!(len >= 20, "MPLS PlayItem is too short");
        let start = pos + 2;
        ensure!(start + len <= b.len(), "Truncated MPLS PlayItem");
        if b.get(start..start + 5) != Some(clip.as_bytes())
            || b.get(start + 5..start + 9) != Some(b"M2TS")
        {
            return Ok(None);
        }
        let input = u32_at(b, start + 12)?;
        let output = u32_at(b, start + 16)?;
        ensure!(output > input, "Invalid MPLS PlayItem interval");
        items.push((input, output, elapsed));
        elapsed += (output - input) as i128;
        pos += len + 2;
    }
    let count = u16_at(b, marks_offset + 4)? as usize;
    let mut ticks = Vec::new();
    for i in 0..count {
        let pos = marks_offset + 6 + i * 14;
        if b.get(pos + 1) != Some(&1) {
            continue;
        }
        let item = u16_at(b, pos + 2)? as usize;
        let (input, output, prior) = *items.get(item).context("MPLS mark has invalid PlayItem")?;
        let tick = u32_at(b, pos + 4)?;
        if tick >= input && tick < output {
            ticks.push(prior + (tick - input) as i128);
        }
    }
    ticks.sort_unstable();
    ticks.dedup();
    if ticks.is_empty() {
        return Ok(None);
    }
    let duration_frames = (elapsed * fps.num as i128 / (45000 * fps.den as i128)) as i64;
    let terminal_window = 50.min((duration_frames / 4).max(1));
    let times = ticks
        .into_iter()
        .filter(|tick| {
            let frame = (*tick * fps.num as i128 / (45000 * fps.den as i128)) as i64;
            frame <= duration_frames - terminal_window
        })
        .map(|tick| ((tick * 1000 + 22500) / 45000) as i64)
        .collect::<Vec<_>>();
    if times.is_empty() {
        return Ok(None);
    }
    Ok(Some(PlaylistChapters {
        times,
        duration_ms: ((elapsed * 1000 + 22500) / 45000) as i64,
        duration_frames,
        in_ms: 0,
    }))
}
pub fn name_times(times: Vec<i64>, names: &[String]) -> Result<Vec<(i64, String)>> {
    ensure!(
        names.is_empty() || names.len() == times.len(),
        "{} names for {} chapters",
        names.len(),
        times.len()
    );
    Ok(times
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            (
                t,
                names
                    .get(i)
                    .cloned()
                    .unwrap_or(format!("Chapter {:02}", i + 1)),
            )
        })
        .collect())
}
