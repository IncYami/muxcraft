use console::Style;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::IsTerminal;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static VERBOSE: OnceLock<bool> = OnceLock::new();

pub fn initialize(verbose: bool) {
    let _ = VERBOSE.set(verbose);
    let _ = tracing_subscriber::fmt()
        .with_max_level(if verbose {
            tracing::Level::DEBUG
        } else {
            tracing::Level::WARN
        })
        .with_writer(std::io::stderr)
        .without_time()
        .try_init();
}

pub fn verbose() -> bool {
    *VERBOSE.get().unwrap_or(&false)
}

fn terminal() -> bool {
    std::io::stderr().is_terminal()
}

pub fn batch(mode: &str, count: usize) {
    let noun = if count == 1 { "episode" } else { "episodes" };
    if terminal() {
        eprintln!(
            "{}  {} {}",
            Style::new().cyan().bold().apply_to("muxcraft"),
            count,
            noun
        );
        eprintln!("  {}", Style::new().dim().apply_to(mode));
    } else {
        eprintln!("muxcraft: {mode} ({count} {noun})");
    }
}

pub fn episode(id: &str, index: usize, total: usize, title: &str) {
    if terminal() {
        eprintln!();
        eprintln!(
            "{}  {}  {}",
            Style::new().cyan().bold().apply_to(format!("Episode {id}")),
            Style::new().dim().apply_to(format!("{index}/{total}")),
            title
        );
    } else {
        eprintln!("Episode {id} ({index}/{total}): {title}");
    }
}

pub fn detail(message: &str) {
    eprintln!("    {message}");
}

pub fn complete(id: &str, output: &std::path::Path, elapsed: Duration) {
    let label = Style::new().green().bold().apply_to("Complete");
    eprintln!(
        "{label}  Episode {id} in {}  → {}",
        duration(elapsed),
        output.display()
    );
}

pub fn checked(count: usize, elapsed: Duration) {
    let label = Style::new().green().bold().apply_to("Checked");
    let noun = if count == 1 { "episode" } else { "episodes" };
    eprintln!("{label}  {count} {noun} in {}", duration(elapsed));
}

pub fn batch_complete(count: usize, elapsed: Duration) {
    let label = Style::new().green().bold().apply_to("Batch complete");
    let noun = if count == 1 { "episode" } else { "episodes" };
    eprintln!("{label}  {count} {noun} in {}", duration(elapsed));
}

fn duration(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{:.1}s", elapsed.as_secs_f64())
    }
}

pub struct Step {
    label: String,
    started: Instant,
    finished: bool,
}

impl Step {
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        if terminal() {
            eprintln!("  {} {label}", Style::new().cyan().apply_to("›"));
        } else {
            eprintln!("  Starting: {label}");
        }
        Self {
            label,
            started: Instant::now(),
            finished: false,
        }
    }

    pub fn done(mut self) {
        self.finished = true;
        if terminal() {
            eprintln!(
                "  {} {}  {}",
                Style::new().green().apply_to("✓"),
                self.label,
                Style::new()
                    .dim()
                    .apply_to(duration(self.started.elapsed()))
            );
        } else {
            eprintln!(
                "  Done: {} ({})",
                self.label,
                duration(self.started.elapsed())
            );
        }
    }
}

impl Drop for Step {
    fn drop(&mut self) {
        if !self.finished {
            if terminal() {
                eprintln!(
                    "  {} {}  {}",
                    Style::new().red().apply_to("✗"),
                    self.label,
                    Style::new()
                        .dim()
                        .apply_to(duration(self.started.elapsed()))
                );
            } else {
                eprintln!(
                    "  Failed: {} ({})",
                    self.label,
                    duration(self.started.elapsed())
                );
            }
        }
    }
}

pub fn command(program: &str, args: &[String]) {
    if verbose() {
        tracing::debug!(program, args = ?args, "running command");
    }
}

#[derive(Clone)]
pub struct Progress(ProgressBar);

impl Progress {
    pub fn new(label: &str, total: Option<u64>) -> Self {
        let bar = match total {
            Some(total) if std::io::stderr().is_terminal() => {
                let bar = ProgressBar::new(total);
                bar.set_style(
                    ProgressStyle::with_template(
                        "{prefix:.bold.cyan} [{bar:32.cyan/blue}] {pos}/{len}",
                    )
                    .unwrap(),
                );
                bar.set_prefix(label.to_owned());
                bar
            }
            _ if std::io::stderr().is_terminal() => {
                let bar = ProgressBar::new_spinner();
                bar.set_style(
                    ProgressStyle::with_template("{prefix:.bold.cyan} {spinner}").unwrap(),
                );
                bar.set_prefix(label.to_owned());
                bar.enable_steady_tick(std::time::Duration::from_millis(100));
                bar
            }
            _ => ProgressBar::hidden(),
        };
        Self(bar)
    }

    pub fn update(&self, current: u64) {
        self.0.set_position(current);
    }
    pub fn update_percent(&self, current: u8) {
        self.0.set_length(100);
        self.0.set_position(u64::from(current));
    }
    pub fn finish(&self) {
        self.0.finish_and_clear();
    }
}

pub fn diagnostic(text: &str) {
    if verbose() && !text.trim().is_empty() {
        eprint!("{text}");
        if !text.ends_with('\n') {
            eprintln!();
        }
    }
}

pub fn error(text: &str) {
    let label = Style::new().red().bold().apply_to("Error:");
    eprintln!("{label} {text}");
}

pub fn warning(text: &str) {
    let label = Style::new().yellow().bold().apply_to("Warning:");
    eprintln!("{label} {text}");
}

pub fn tool_output_line(line: &str) {
    if verbose() {
        diagnostic(line);
    } else if line.to_ascii_lowercase().contains("warning")
        || line.to_ascii_lowercase().contains("error")
    {
        if line.to_ascii_lowercase().starts_with("warning")
            || line.to_ascii_lowercase().starts_with("error")
        {
            eprintln!("{line}");
        } else if line.to_ascii_lowercase().contains("error") {
            error(line);
        } else {
            warning(line);
        }
    }
}

/// Extract an FFmpeg progress update from its key/value protocol.
pub fn ffmpeg_progress_line(line: &str) -> Option<(String, Option<u64>)> {
    let (key, value) = line.trim().split_once('=')?;
    match key {
        "frame" => value.parse().ok().map(|n| ("frame".into(), Some(n))),
        "out_time_ms" => value.parse().ok().map(|n| ("time".into(), Some(n))),
        "progress" if value == "end" => Some(("done".into(), None)),
        _ => None,
    }
}

/// Parse a mkvmerge --gui-mode progress line (`#GUI#progress 37%`).
pub fn mkvmerge_progress_line(line: &str) -> Option<u8> {
    let percent = line
        .trim()
        .strip_prefix("#GUI#progress ")?
        .strip_suffix('%')?
        .parse::<u8>()
        .ok()?;
    (percent <= 100).then_some(percent)
}

pub fn run_mkvmerge(program: &str, args: &[String]) -> anyhow::Result<std::process::Output> {
    use std::process::Stdio;
    let mut actual = args.to_vec();
    if !actual.iter().any(|arg| arg == "--gui-mode") {
        actual.insert(0, "--gui-mode".into());
    }
    let progress = Progress::new("mkvmerge", Some(100));
    let mut child = std::process::Command::new(program)
        .args(&actual)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Missing mkvmerge stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("Missing mkvmerge stderr"))?;
    let out_bar = progress.clone();
    let out_thread = std::thread::spawn(move || collect_gui_lines(stdout, out_bar));
    let err_bar = progress.clone();
    let err_thread = std::thread::spawn(move || collect_gui_lines(stderr, err_bar));
    let status = child.wait()?;
    let stdout = out_thread
        .join()
        .map_err(|_| anyhow::anyhow!("mkvmerge stdout reader panicked"))??;
    let stderr = err_thread
        .join()
        .map_err(|_| anyhow::anyhow!("mkvmerge stderr reader panicked"))??;
    progress.finish();
    let output = std::process::Output {
        status,
        stdout,
        stderr,
    };
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for line in all.lines().filter(|line| !line.starts_with("#GUI#")) {
        tool_output_line(line);
    }
    Ok(output)
}

pub fn run_tool(program: &str, args: &[String]) -> anyhow::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;
    let progress = Progress::new(program, None);
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Missing tool stdout"))?;
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("Missing tool stderr"))?;
    let mut stderr_bytes = Vec::new();
    stderr.read_to_end(&mut stderr_bytes)?;
    let stdout_bytes = stdout_thread
        .join()
        .map_err(|_| anyhow::anyhow!("Tool stdout reader panicked"))??;
    let status = child.wait()?;
    progress.finish();
    for line in String::from_utf8_lossy(&stdout_bytes)
        .lines()
        .chain(String::from_utf8_lossy(&stderr_bytes).lines())
    {
        tool_output_line(line);
    }
    Ok(std::process::Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    })
}

fn collect_gui_lines<R: std::io::Read + Send + 'static>(
    stream: R,
    bar: Progress,
) -> std::io::Result<Vec<u8>> {
    use std::io::{BufRead, BufReader};
    let mut bytes = Vec::new();
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if let Some(percent) = mkvmerge_progress_line(&line) {
            bar.update_percent(percent);
        }
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    Ok(bytes)
}

pub fn collect_ffmpeg_progress<R: std::io::Read>(
    stream: R,
    bar: Progress,
) -> std::io::Result<Vec<u8>> {
    use std::io::{BufRead, BufReader};
    let mut bytes = Vec::new();
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if let Some((_, Some(value))) = ffmpeg_progress_line(&line) {
            bar.update(value);
        }
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{ffmpeg_progress_line, mkvmerge_progress_line};

    #[test]
    fn parses_ffmpeg_progress_protocol() {
        assert_eq!(
            ffmpeg_progress_line("frame=42"),
            Some(("frame".into(), Some(42)))
        );
        assert_eq!(
            ffmpeg_progress_line("progress=end"),
            Some(("done".into(), None))
        );
        assert_eq!(ffmpeg_progress_line("fps=24.0"), None);
    }

    #[test]
    fn parses_only_valid_mkvmerge_percentages() {
        assert_eq!(mkvmerge_progress_line("#GUI#progress 37%"), Some(37));
        assert_eq!(mkvmerge_progress_line("#GUI#progress 137%"), None);
        assert_eq!(mkvmerge_progress_line("warning"), None);
    }
}
