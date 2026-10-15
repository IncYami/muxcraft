use crate::{
    chapters,
    config::{Config, EpisodePlan},
    subtitle::Fps,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{path::Path, process::Command};

#[derive(Debug, Clone)]
pub struct ResolvedCut {
    pub playlist: std::path::PathBuf,
    pub input_ticks: u32,
    pub source_start_seconds: f64,
    pub start_chapter: usize,
    pub end_chapter: Option<usize>,
    pub start_ticks: u32,
    pub end_ticks: u32,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub video_frames: u64,
    pub start_deviation_ms: f64,
    pub end_deviation_ms: Option<f64>,
    pub chapter_ticks: Vec<u32>,
}

#[derive(Deserialize)]
struct PacketList {
    packets: Option<Vec<Packet>>,
}
#[derive(Deserialize)]
struct Packet {
    pts_time: Option<String>,
    flags: Option<String>,
}

fn nearest_keyframe(
    program: &str,
    source: &Path,
    track: usize,
    target: f64,
    frame: f64,
) -> Result<f64> {
    let begin = (target - 2.0).max(0.0);
    let interval = format!("{begin:.9}%+4");
    let output = Command::new(program)
        .args([
            "-v",
            "error",
            "-select_streams",
            &format!("v:{track}"),
            "-read_intervals",
            &interval,
            "-show_packets",
            "-show_entries",
            "packet=pts_time,flags",
            "-of",
            "json",
        ])
        .arg(source)
        .output()
        .with_context(|| format!("Running {program}"))?;
    ensure!(
        output.status.success(),
        "ffprobe keyframes failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let packets: PacketList = serde_json::from_slice(&output.stdout)?;
    let closest = packets
        .packets
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.flags.as_deref().is_some_and(|s| s.contains('K')))
        .filter_map(|p| p.pts_time?.parse::<f64>().ok())
        .min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()))
        .with_context(|| format!("No keyframe near {target:.9} s in {}", source.display()))?;
    ensure!(
        (closest - target).abs() <= frame + 0.000_001,
        "Playlist mark {target:.9} s is {:.3} ms from nearest keyframe ({closest:.9} s); maximum is one frame",
        (closest - target).abs() * 1000.0
    );
    Ok(closest)
}

pub fn resolve(cfg: &Config, plan: &EpisodePlan) -> Result<Option<ResolvedCut>> {
    let Some(spec) = &plan.cut else {
        return Ok(None);
    };
    let clip = plan
        .source
        .file_stem()
        .context("M2TS has no clip name")?
        .to_string_lossy();
    let playlist = spec
        .playlist
        .as_ref()
        .cloned()
        .map_or_else(|| chapters::discover_cut_playlist(&plan.source), Ok)?;
    let marks = chapters::cut_marks(&std::fs::read(&playlist)?, &clip)?;
    ensure!(
        spec.start_chapter <= marks.marks.len(),
        "Cut {} start chapter is outside the playlist",
        plan.episode
    );
    if let Some(end) = spec.end_chapter {
        ensure!(
            end <= marks.marks.len(),
            "Cut {} end chapter is outside the playlist",
            plan.episode
        );
    }
    let start_ticks = marks.marks[spec.start_chapter - 1];
    let end_ticks = spec
        .end_chapter
        .map_or(marks.output_ticks, |end| marks.marks[end - 1]);
    ensure!(
        end_ticks > start_ticks,
        "Cut {} has an empty interval",
        plan.episode
    );
    let fps = Fps::parse(&cfg.project.fps)?;
    let frame = fps.den as f64 / fps.num as f64;
    let video = &plan.tracks[0];
    let origin = Command::new(&cfg.tools.ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=start_time",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(&plan.source)
        .output()?;
    ensure!(origin.status.success(), "ffprobe source start time failed");
    let source_start_seconds: f64 = String::from_utf8(origin.stdout)?
        .trim()
        .parse()
        .context("Source has no start time")?;
    let codec = Command::new(&cfg.tools.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            &format!("v:{}", video.track),
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(&plan.source)
        .output()?;
    let codecs = String::from_utf8_lossy(&codec.stdout);
    ensure!(
        codec.status.success()
            && !codecs.trim().is_empty()
            && codecs.lines().all(|s| s.trim() == "h264"),
        "Cuts currently require H.264 video"
    );
    let requested_start = start_ticks as f64 / 45000.0;
    let requested_end = end_ticks as f64 / 45000.0;
    let start_seconds = nearest_keyframe(
        &cfg.tools.ffprobe,
        &plan.source,
        video.track,
        requested_start,
        frame,
    )?;
    let end_seconds = if spec.end_chapter.is_some() {
        nearest_keyframe(
            &cfg.tools.ffprobe,
            &plan.source,
            video.track,
            requested_end,
            frame,
        )?
    } else {
        requested_end
    };
    ensure!(
        end_seconds > start_seconds,
        "Cut {} keyframes are not ascending",
        plan.episode
    );
    let video_frames = ((end_seconds - start_seconds) / frame).round() as u64;
    ensure!(
        video_frames > 0,
        "Cut {} contains no video frames",
        plan.episode
    );
    ensure!(
        start_seconds + 0.000001 >= source_start_seconds,
        "Cut {} starts before the M2TS timeline",
        plan.episode
    );
    let chapter_ticks = marks
        .marks
        .iter()
        .copied()
        .filter(|tick| *tick >= start_ticks && *tick < end_ticks)
        .collect();
    Ok(Some(ResolvedCut {
        playlist,
        input_ticks: marks.input_ticks,
        source_start_seconds,
        start_chapter: spec.start_chapter,
        end_chapter: spec.end_chapter,
        start_ticks,
        end_ticks,
        start_seconds,
        end_seconds,
        video_frames,
        start_deviation_ms: (start_seconds - requested_start) * 1000.0,
        end_deviation_ms: spec
            .end_chapter
            .map(|_| (end_seconds - requested_end) * 1000.0),
        chapter_ticks,
    }))
}
