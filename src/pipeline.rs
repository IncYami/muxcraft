use crate::{
    chapters,
    config::{ChapterSpec, Config, Episode, EpisodePlan, Track, TrackPlan, absolute, expand},
    cuts::{self, ResolvedCut},
    fonts::{FontIndex, requests},
    pgs,
    subtitle::{self, Ass, Fps},
    tmdb,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Deserialize)]
struct Identification {
    #[serde(default)]
    tracks: Vec<IdentTrack>,
    #[serde(default)]
    errors: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct IdentTrack {
    id: u64,
    codec: String,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Debug, Serialize)]
struct CommandRecord {
    program: String,
    args: Vec<String>,
    status: Option<i32>,
}
#[derive(Debug, Serialize)]
struct Audit {
    plan: EpisodePlan,
    configuration: Config,
    state: String,
    commands: Vec<CommandRecord>,
    fonts: Vec<crate::fonts::CollectedFont>,
    error: Option<String>,
    chapter_source: Option<PathBuf>,
    chapter_timing: Option<ChapterTiming>,
    cut: Option<CutAudit>,
}
#[derive(Debug, Serialize)]
struct CutAudit {
    resolved: ResolvedCut,
    source_video_sha256: Option<String>,
    copied_video_sha256: Option<String>,
    final_video_sha256: Option<String>,
    source_video_packets: Option<u64>,
    copied_video_packets: Option<u64>,
    final_video_packets: Option<u64>,
    source_audio_sha256: BTreeMap<String, String>,
    cut_audio_sha256: BTreeMap<String, String>,
    final_audio_sha256: BTreeMap<String, String>,
    source_audio_pcm_bytes: BTreeMap<String, u64>,
    cut_audio_pcm_bytes: BTreeMap<String, u64>,
    final_audio_pcm_bytes: BTreeMap<String, u64>,
    audio_sample_ranges: BTreeMap<String, AudioSampleRange>,
    pgs_sha256: BTreeMap<String, String>,
    first_video_ms: Option<f64>,
    first_audio_ms: BTreeMap<String, f64>,
}
#[derive(Debug, Clone, Serialize)]
struct AudioSampleRange {
    first_source_sample_pts_seconds: f64,
    sample_rate: u32,
    start_sample: u64,
    end_sample: u64,
    decoded_end_sample: u64,
}
#[derive(Debug, Clone, Serialize)]
struct ChapterTiming {
    playlist_duration_ms: i64,
    video_duration_ms: Option<i64>,
    video_duration_source: Option<String>,
    difference_ms: Option<i64>,
    cutoff_frames: i64,
    cutoff_source: String,
}
struct ChapterData {
    content: Option<String>,
    source: PathBuf,
    timing: Option<ChapterTiming>,
}
struct VideoDuration {
    duration_ms: i64,
    frames: i64,
    source: &'static str,
}
fn identify(program: &str, path: &Path) -> Result<Identification> {
    let out = Command::new(program)
        .args(["-J"])
        .arg(path)
        .output()
        .with_context(|| format!("Running {program}"))?;
    ensure!(
        out.status.code().is_some_and(|c| c <= 1),
        "Failed to identify {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let ident: Identification =
        serde_json::from_slice(&out.stdout).context("Invalid mkvmerge identification")?;
    ensure!(ident.errors.is_empty(), "mkvmerge: {:?}", ident.errors);
    Ok(ident)
}
fn track_id(ident: &Identification, category: &str, relative: usize) -> Result<u64> {
    let kind = if category == "subtitle" {
        "subtitles"
    } else {
        category
    };
    ident
        .tracks
        .iter()
        .filter(|t| t.kind == kind)
        .nth(relative)
        .map(|t| t.id)
        .with_context(|| format!("Relative track {relative} of {category} not found"))
}

fn pgs_track(ident: &Identification, relative: usize) -> Result<(u64, usize)> {
    ident
        .tracks
        .iter()
        .filter(|track| track.kind == "subtitles")
        .enumerate()
        .filter(|(_, track)| track.codec.contains("PGS"))
        .nth(relative)
        .map(|(subtitle_index, track)| (track.id, subtitle_index))
        .with_context(|| format!("PGS track {relative} not found"))
}

#[derive(Debug, Deserialize)]
struct AudioStream {
    codec_name: String,
    sample_rate: String,
    channels: u8,
    time_base: String,
    bits_per_raw_sample: Option<String>,
    bits_per_sample: Option<u8>,
    sample_fmt: Option<String>,
}

fn audio_stream(program: &str, path: &Path, track: usize) -> Result<AudioStream> {
    let out = Command::new(program)
        .args([
            "-v",
            "error",
            "-select_streams",
            &format!("a:{track}"),
            "-show_entries",
            "stream=codec_name,sample_rate,channels,time_base,bits_per_raw_sample,bits_per_sample,sample_fmt",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .with_context(|| format!("Running {program}"))?;
    ensure!(
        out.status.success(),
        "ffprobe failed for {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    #[derive(Deserialize)]
    struct Streams {
        streams: Vec<AudioStream>,
    }
    let mut streams: Streams =
        serde_json::from_slice(&out.stdout).context("Invalid ffprobe audio stream data")?;
    ensure!(
        !streams.streams.is_empty(),
        "Audio track {track} not found in {}",
        path.display()
    );
    let stream = streams.streams.remove(0);
    ensure!(stream.channels > 0, "Audio track {track} has no channels");
    Ok(stream)
}

fn source_audio_sample_range(
    cfg: &Config,
    source: &Path,
    track: usize,
    stream: &AudioStream,
    cut: &ResolvedCut,
) -> Result<AudioSampleRange> {
    let output = Command::new(&cfg.tools.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            &format!("a:{track}"),
            "-read_intervals",
            "%+#1",
            "-show_frames",
            "-show_entries",
            "frame=pts",
            "-of",
            "json",
        ])
        .arg(source)
        .output()?;
    ensure!(
        output.status.success(),
        "ffprobe failed to locate the first audio sample in {}: {}",
        source.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    #[derive(Deserialize)]
    struct Frame {
        pts: i64,
    }
    #[derive(Deserialize)]
    struct Frames {
        frames: Vec<Frame>,
    }
    let frames: Frames = serde_json::from_slice(&output.stdout)?;
    let first_pts_ticks = frames
        .frames
        .first()
        .context("Source has no decoded audio frames")?
        .pts;
    let (time_base_num, time_base_den) = stream
        .time_base
        .split_once('/')
        .context("Invalid audio time base")?;
    let time_base_num: u32 = time_base_num.parse()?;
    let time_base_den: u32 = time_base_den.parse()?;
    ensure!(
        time_base_num > 0 && time_base_den > 0,
        "Invalid audio time base"
    );
    let first_pts = first_pts_ticks as f64 * f64::from(time_base_num) / f64::from(time_base_den);
    let sample_rate: u32 = stream.sample_rate.parse()?;
    ensure!(sample_rate > 0, "Invalid audio sample rate");
    let index = |seconds: f64| -> Result<u64> {
        let samples = ((seconds - first_pts) * f64::from(sample_rate))
            .round()
            .max(0.0);
        ensure!(
            samples.is_finite() && samples < u64::MAX as f64,
            "Audio sample index is outside the supported range"
        );
        Ok(samples as u64)
    };
    let start_sample = index(cut.start_seconds)?;
    let end_sample = index(cut.end_seconds)?;
    ensure!(end_sample > start_sample, "Cut contains no audio samples");
    Ok(AudioSampleRange {
        first_source_sample_pts_seconds: first_pts,
        sample_rate,
        start_sample,
        end_sample,
        decoded_end_sample: end_sample,
    })
}

fn pcm_codec(stream: &AudioStream) -> &'static str {
    let bits = stream
        .bits_per_raw_sample
        .as_deref()
        .and_then(|s| s.parse::<u8>().ok())
        .or(stream.bits_per_sample)
        .or_else(|| {
            stream
                .sample_fmt
                .as_deref()
                .map(|s| if s.contains("s16") { 16 } else { 24 })
        })
        .unwrap_or(16);
    if bits <= 16 { "pcm_s16le" } else { "pcm_s24le" }
}

fn flac_version(program: &str) -> Option<String> {
    let output = Command::new(program).arg("--version").output().ok()?;
    let version = String::from_utf8_lossy(&output.stdout);
    let version = version
        .split_whitespace()
        .find(|part| part.starts_with("1."))?;
    Some(version.to_owned())
}

fn flac_threads(version: &str) -> Option<usize> {
    let minor = version.split('.').nth(1)?.parse::<u8>().ok()?;
    (minor >= 5).then(|| {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8)
    })
}

struct PreparedAudio {
    path: PathBuf,
    tags: BTreeMap<String, String>,
    source_pcm: Option<PathBuf>,
    source_pcm_hash: Option<(String, u64)>,
    source_sample_range: Option<AudioSampleRange>,
}

fn cut_relative(cut: &ResolvedCut) -> f64 {
    cut.start_seconds - cut.source_start_seconds
}

// This is only a demux read limit. The atrim filters below retain their exact
// timestamp/sample boundaries and the margin covers packet duration/reordering.
fn source_read_bound(cut: &ResolvedCut) -> f64 {
    (cut.end_seconds - cut.source_start_seconds + 2.0).max(0.0)
}

fn cut_video(
    cfg: &Config,
    audit: &mut Audit,
    source: &Path,
    track: usize,
    cut: &ResolvedCut,
    work: &Path,
) -> Result<(PathBuf, (String, u64))> {
    let output = work
        .join(source.file_name().context("Source has no filename")?)
        .with_extension("mkv");
    let tee_output_path = if output.is_absolute() {
        output.clone()
    } else {
        std::env::current_dir()?.join(&output)
    };
    let tee_path = tee_output_path
        .to_string_lossy()
        .chars()
        .flat_map(|c| {
            if "\\|[]".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect::<String>();
    let tee_output = format!(
        "[f=matroska]{tee_path}|[f=hash:hash=SHA256:bsfs/v=h264_mp4toannexb\\,filter_units=pass_types=1-5]pipe:1"
    );
    let args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:2".into(),
        "-n".into(),
        "-ss".into(),
        format!("{:.9}", cut_relative(cut)),
        "-i".into(),
        source.to_string_lossy().into(),
        "-map".into(),
        format!("0:v:{track}"),
        "-frames:v".into(),
        cut.video_frames.to_string(),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-map_chapters".into(),
        "-1".into(),
        "-c:v".into(),
        "copy".into(),
        // The tee muxer preserves the MPEG-TS codec tag, which Matroska rejects.
        "-tag:v".into(),
        "avc1".into(),
        "-f".into(),
        "tee".into(),
        tee_output,
    ];
    let result = Command::new(&cfg.tools.ffmpeg).args(&args).output()?;
    audit.commands.push(CommandRecord {
        program: cfg.tools.ffmpeg.clone(),
        args,
        status: result.status.code(),
    });
    audit.save()?;
    ensure!(
        result.status.success(),
        "Cut video and source hash failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    ensure!(output.is_file(), "FFmpeg did not create the cut video");
    let line = String::from_utf8(result.stdout)?;
    let hash = line
        .trim()
        .strip_prefix("SHA256=")
        .context("FFmpeg tee muxer did not return source SHA256")?;
    ensure!(hash.len() == 64, "Invalid source video SHA256");
    let packets = String::from_utf8_lossy(&result.stderr)
        .lines()
        .filter_map(|line| {
            line.strip_prefix("frame=")
                .and_then(|number| number.parse::<u64>().ok())
        })
        .next_back()
        .context("FFmpeg did not report copied video packets")?;
    Ok((output, (hash.to_owned(), packets)))
}

fn video_sha256(
    cfg: &Config,
    audit: &mut Audit,
    source: &Path,
    track: usize,
    cut: Option<&ResolvedCut>,
) -> Result<(String, u64)> {
    let mut args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:2".into(),
    ];
    if let Some(cut) = cut {
        args.extend(["-ss".into(), format!("{:.9}", cut_relative(cut))]);
    }
    args.extend(["-i".into(), source.to_string_lossy().into()]);
    args.extend([
        "-map".into(),
        format!("0:v:{track}"),
        "-c:v".into(),
        "copy".into(),
        "-bsf:v".into(),
        "h264_mp4toannexb,filter_units=pass_types=1-5".into(),
    ]);
    if let Some(cut) = cut {
        args.extend(["-frames:v".into(), cut.video_frames.to_string()]);
    }
    args.extend([
        "-f".into(),
        "hash".into(),
        "-hash".into(),
        "SHA256".into(),
        "-".into(),
    ]);
    let output = Command::new(&cfg.tools.ffmpeg).args(&args).output()?;
    audit.commands.push(CommandRecord {
        program: cfg.tools.ffmpeg.clone(),
        args,
        status: output.status.code(),
    });
    audit.save()?;
    ensure!(
        output.status.success(),
        "Video hash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line = String::from_utf8(output.stdout)?;
    let hash = line
        .trim()
        .strip_prefix("SHA256=")
        .context("FFmpeg did not return SHA256")?;
    ensure!(hash.len() == 64, "Invalid video SHA256");
    let packets = String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| {
            line.strip_prefix("frame=")
                .and_then(|number| number.parse::<u64>().ok())
        })
        .next_back()
        .context("FFmpeg did not report copied video packets")?;
    Ok((hash.into(), packets))
}

fn pcm_sha256(
    cfg: &Config,
    audit: &mut Audit,
    source: &Path,
    track: usize,
    codec: &str,
    sample_range: Option<[u64; 2]>,
) -> Result<(String, u64)> {
    let mut args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
    ];
    args.extend(["-i".into(), source.to_string_lossy().into()]);
    if let Some([start, end]) = sample_range {
        args.extend([
            "-af".into(),
            format!("atrim=start_sample={start}:end_sample={end}"),
        ]);
    }
    let format = if codec == "pcm_s16le" {
        "s16le"
    } else {
        "s24le"
    };
    args.extend([
        "-map".into(),
        format!("0:a:{track}"),
        "-c:a".into(),
        codec.into(),
        "-f".into(),
        format.into(),
        "-".into(),
    ]);
    let mut child = Command::new(&cfg.tools.ffmpeg)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let mut output = child.stdout.take().context("Missing FFmpeg PCM output")?;
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut chunk = [0_u8; 65536];
    loop {
        let n = output.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        digest.update(&chunk[..n]);
        bytes += n as u64;
    }
    let result = child.wait_with_output()?;
    audit.commands.push(CommandRecord {
        program: cfg.tools.ffmpeg.clone(),
        args,
        status: result.status.code(),
    });
    audit.save()?;
    ensure!(
        result.status.success(),
        "PCM hash failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok((
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        bytes,
    ))
}

fn first_packet_ms(program: &str, source: &Path, kind: &str, track: usize) -> Result<f64> {
    let result = Command::new(program)
        .args([
            "-v",
            "error",
            "-select_streams",
            &format!("{kind}:{track}"),
            "-read_intervals",
            "%+#1",
            "-show_packets",
            "-show_entries",
            "packet=pts_time",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(source)
        .output()?;
    ensure!(
        result.status.success(),
        "ffprobe failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout)?;
    let first = text
        .lines()
        .find_map(|line| line.trim().parse::<f64>().ok())
        .context("Track has no timestamped packets")?;
    Ok(first * 1000.0)
}

fn pgs_sha256(cfg: &Config, audit: &mut Audit, source: &Path, track: usize) -> Result<String> {
    let args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        source.to_string_lossy().into(),
        "-map".into(),
        format!("0:s:{track}"),
        "-c:s".into(),
        "copy".into(),
        "-f".into(),
        "hash".into(),
        "-hash".into(),
        "SHA256".into(),
        "-".into(),
    ];
    let result = Command::new(&cfg.tools.ffmpeg).args(&args).output()?;
    audit.commands.push(CommandRecord {
        program: cfg.tools.ffmpeg.clone(),
        args,
        status: result.status.code(),
    });
    audit.save()?;
    ensure!(
        result.status.success(),
        "PGS hash failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout)?;
    let hash = output
        .trim()
        .strip_prefix("SHA256=")
        .context("Missing PGS SHA256")?;
    ensure!(hash.len() == 64, "Invalid PGS SHA256");
    Ok(hash.into())
}

fn extract_audio(
    cfg: &Config,
    audit: &mut Audit,
    track: &Track,
    source: &Path,
    work: &Path,
    stream: &AudioStream,
    cut: Option<&ResolvedCut>,
    source_range: Option<&AudioSampleRange>,
) -> Result<PreparedAudio> {
    let extension = if cut.is_some() {
        "wav"
    } else {
        match stream.codec_name.as_str() {
            "flac" => "flac",
            "truehd" => "thd",
            "dts" => "dts",
            "ac3" => "ac3",
            "eac3" => "eac3",
            "aac" => "aac",
            "mp3" => "mp3",
            "opus" => "opus",
            "vorbis" => "ogg",
            name if name.starts_with("pcm_") => "wav",
            _ => "wav",
        }
    };
    let extracted = work.join(format!("{}.extracted.{extension}", track.id));
    let mut source_sample_range = source_range.cloned();
    let source_pcm_hash = if let (Some(cut), Some(range)) = (cut, source_range)
        && track.codec == "flac"
    {
        let codec = pcm_codec(stream);
        let format = if codec == "pcm_s16le" {
            "s16le"
        } else {
            "s24le"
        };
        let filter = format!(
            "[0:a:{}]asplit=2[cut_in][source_in];[cut_in]atrim=start={:.9}:end={:.9},asetpts=PTS-STARTPTS[cut_out];[source_in]atrim=start_sample={}:end_sample={}[source_out]",
            track.track,
            cut_relative(cut),
            cut.end_seconds - cut.source_start_seconds,
            range.start_sample,
            range.end_sample
        );
        let args = vec![
            "-nostdin".into(),
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-n".into(),
            "-t".into(),
            format!("{:.9}", source_read_bound(cut)),
            "-i".into(),
            source.to_string_lossy().into(),
            "-filter_complex".into(),
            filter,
            "-map".into(),
            "[cut_out]".into(),
            "-c:a".into(),
            codec.into(),
            "-rf64".into(),
            "auto".into(),
            extracted.to_string_lossy().into(),
            "-map".into(),
            "[source_out]".into(),
            "-c:a".into(),
            codec.into(),
            "-f".into(),
            format.into(),
            "-".into(),
        ];
        let mut child = Command::new(&cfg.tools.ffmpeg)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let mut output = child.stdout.take().context("Missing FFmpeg PCM output")?;
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        let mut chunk = [0_u8; 65536];
        loop {
            let n = output.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            digest.update(&chunk[..n]);
            bytes += n as u64;
        }
        let result = child.wait_with_output()?;
        audit.commands.push(CommandRecord {
            program: cfg.tools.ffmpeg.clone(),
            args,
            status: result.status.code(),
        });
        audit.save()?;
        ensure!(
            result.status.success(),
            "Cut audio extraction and source PCM hash failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let bytes_per_sample = if codec == "pcm_s16le" { 2_u64 } else { 3_u64 };
        let bytes_per_frame = u64::from(stream.channels)
            .checked_mul(bytes_per_sample)
            .context("PCM sample-frame byte count overflowed")?;
        ensure!(
            bytes % bytes_per_frame == 0,
            "Sample-count PCM branch returned a partial interleaved sample frame"
        );
        let actual_samples = bytes / bytes_per_frame;
        let requested_samples = range.end_sample - range.start_sample;
        ensure!(
            actual_samples <= requested_samples,
            "Sample-count PCM branch exceeded the requested source sample range"
        );
        ensure!(
            actual_samples == requested_samples || cut.end_chapter.is_none(),
            "Source audio reached EOF before the requested internal chapter boundary"
        );
        ensure!(
            actual_samples > 0,
            "Cut contains no available audio samples"
        );
        if let Some(range) = &mut source_sample_range {
            range.decoded_end_sample = range
                .start_sample
                .checked_add(actual_samples)
                .context("Decoded source sample range overflowed")?;
        }
        Some((
            digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            bytes,
        ))
    } else {
        let mut args = vec!["-nostdin".into(), "-hide_banner".into(), "-n".into()];
        if let Some(cut) = cut {
            args.extend(["-t".into(), format!("{:.9}", source_read_bound(cut))]);
        }
        args.extend([
            "-i".into(),
            source.to_string_lossy().into(),
            "-map_chapters".into(),
            "-1".into(),
            "-map".into(),
            format!("0:a:{}", track.track),
            "-vn".into(),
            "-sn".into(),
            "-dn".into(),
        ]);
        if let Some(cut) = cut {
            // FFmpeg resets input timestamps to the container start before atrim runs.
            args.extend([
                "-af".into(),
                format!(
                    "atrim=start={:.9}:end={:.9},asetpts=PTS-STARTPTS",
                    cut_relative(cut),
                    cut.end_seconds - cut.source_start_seconds
                ),
            ]);
        }
        if extension == "wav" {
            args.extend([
                "-c:a".into(),
                pcm_codec(stream).into(),
                "-rf64".into(),
                "auto".into(),
            ]);
        } else {
            args.extend(["-c:a".into(), "copy".into()]);
            if extension == "dts" {
                args.extend(["-f".into(), "dts".into()]);
            }
        }
        args.push(extracted.to_string_lossy().into());
        audit.command(&cfg.tools.ffmpeg, args, false)?;
        None
    };
    if track.codec == "copy" {
        return Ok(PreparedAudio {
            path: extracted,
            tags: BTreeMap::new(),
            source_pcm: None,
            source_pcm_hash: None,
            source_sample_range: None,
        });
    }

    let input = if extension == "flac" || extension == "wav" {
        extracted.clone()
    } else {
        let pcm = work.join(format!("{}.prepared.w64", track.id));
        let args = vec![
            "-nostdin".into(),
            "-hide_banner".into(),
            "-n".into(),
            "-i".into(),
            extracted.to_string_lossy().into(),
            "-map".into(),
            "0:a:0".into(),
            "-c:a".into(),
            pcm_codec(stream).into(),
            pcm.to_string_lossy().into(),
        ];
        audit.command(&cfg.tools.ffmpeg, args, false)?;
        pcm
    };
    let output = work.join(format!("{}.flac", track.id));
    let mut args = vec![
        "-8".into(),
        "-o".into(),
        output.to_string_lossy().into(),
        "--verify".into(),
    ];
    let version = flac_version(&cfg.tools.flac);
    let mut settings = vec!["-8".to_owned(), "--verify".to_owned()];
    if let Some(threads) = version.as_deref().and_then(flac_threads) {
        args.push(format!("--threads={threads}"));
        settings.push(format!("--threads={threads}"));
    }
    args.push(input.to_string_lossy().into());
    audit.command(&cfg.tools.flac, args, false)?;
    Ok(PreparedAudio {
        path: output,
        source_pcm: cut.map(|_| extracted),
        source_pcm_hash,
        source_sample_range,
        tags: BTreeMap::from([
            (
                "ENCODER".to_owned(),
                version.map_or_else(|| "FLAC".to_owned(), |version| format!("FLAC {version}")),
            ),
            ("ENCODER_SETTINGS".to_owned(), settings.join(" ")),
        ]),
    })
}
fn config_track(cfg: &Config, id: &str, ep: &str) -> Track {
    cfg.video
        .iter()
        .chain(&cfg.audio)
        .chain(&cfg.subtitles)
        .chain(&cfg.pgs)
        .find(|t| t.id == id)
        .unwrap()
        .effective(ep)
}
fn matrix_from_metadata(space: &str, video_range: &str) -> String {
    let matrix = match space {
        "bt709" => "709",
        "smpte170m" | "bt470bg" => "601",
        "smpte240m" => "240M",
        "fcc" => "FCC",
        unknown => {
            eprintln!(
                "Warning: video matrix '{unknown}' cannot be represented in ASS; using None."
            );
            return "None".into();
        }
    };
    let range = match video_range {
        "pc" => "PC",
        "tv" => "TV",
        unknown => {
            eprintln!("Warning: video range '{unknown}' is unspecified; assuming limited range.");
            "TV"
        }
    };
    format!("{range}.{matrix}")
}
fn video_matrix(cfg: &Config, video: &TrackPlan) -> Result<String> {
    let out = Command::new(&cfg.tools.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            &format!("v:{}", video.track),
            "-show_entries",
            "stream=color_space,color_range",
            "-of",
            "json",
        ])
        .arg(&video.source)
        .output()?;
    ensure!(
        out.status.success(),
        "ffprobe failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let stream = &metadata["streams"][0];
    Ok(matrix_from_metadata(
        stream["color_space"].as_str().unwrap_or(""),
        stream["color_range"].as_str().unwrap_or(""),
    ))
}
fn load_sub(
    cfg: &Config,
    t: &Track,
    path: &Path,
    ep: &str,
    video: &TrackPlan,
) -> Result<(String, Option<Ass>)> {
    let input = std::fs::read_to_string(path)
        .with_context(|| format!("UTF-8 subtitle: {}", path.display()))?;
    let default = crate::config::Correction::default();
    let correction = if let Some(table) = t.shift_table.as_ref() {
        table.episodes.get(ep).unwrap_or(&table.default)
    } else {
        &default
    };
    let fps = Fps::parse(&cfg.project.fps)?;
    let kind = if t.kind.is_empty() {
        path.extension().and_then(|s| s.to_str()).unwrap_or("")
    } else {
        &t.kind
    };
    match kind {
        "srt" => {
            ensure!(
                t.headers.is_empty() && t.clean.is_empty(),
                "Subtitle {}: ASS headers and cleaning are not valid for SRT",
                t.id
            );
            Ok((subtitle::process_srt(&input, t, correction, ep, fps)?, None))
        }
        "ass" => {
            let mut effective = t.clone();
            if let Some(matrix) = &t.ycbcr_matrix {
                ensure!(
                    !t.headers.contains_key("YCbCr Matrix"),
                    "Subtitle {} sets YCbCr Matrix twice",
                    t.id
                );
                let value = if matrix == "auto" {
                    video_matrix(cfg, video)?
                } else {
                    matrix.clone()
                };
                effective.headers.insert("YCbCr Matrix".into(), value);
            }
            let ass = subtitle::process_ass(&input, &effective, correction, ep, fps)?;
            Ok((ass.dump(), Some(ass)))
        }
        _ => bail!("Subtitle {}: specify ass, srt, or pgs", t.id),
    }
}
type FontCache = BTreeMap<(Vec<PathBuf>, bool), FontIndex>;
fn font_index<'a>(
    cfg: &Config,
    base: &Path,
    plan: &EpisodePlan,
    cache: &'a mut FontCache,
) -> Result<&'a FontIndex> {
    if !cfg.fonts.enabled {
        return Ok(cache.entry((Vec::new(), false)).or_default());
    }
    let mut dirs = Vec::new();
    for dir in &cfg.fonts.dirs {
        let path = absolute(base, &expand(dir, &plan.vars)?);
        ensure!(
            path.is_dir(),
            "Font directory is missing: {}",
            path.display()
        );
        dirs.push(path);
    }
    if cfg.fonts.search_current_dir {
        dirs.push(base.to_owned());
    }
    if cfg.fonts.search_subtitle_dirs {
        for track in &plan.tracks {
            if track.category != "subtitle" {
                continue;
            }
            let spec = config_track(cfg, &track.id, &plan.episode);
            let is_ass = spec.kind == "ass"
                || (spec.kind.is_empty()
                    && track
                        .source
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("ass")));
            if is_ass && let Some(parent) = track.source.parent() {
                dirs.push(parent.to_owned());
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    let mut roots: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if !roots.iter().any(|root| dir.starts_with(root)) {
            roots.push(dir);
        }
    }
    let key = (roots, cfg.fonts.use_system_fonts);
    if let std::collections::btree_map::Entry::Vacant(entry) = cache.entry(key.clone()) {
        entry.insert(FontIndex::scan(&key.0, key.1)?);
    }
    Ok(&cache[&key])
}
fn video_duration_from_probe(data: &[u8], fps: Fps) -> Option<VideoDuration> {
    let metadata: serde_json::Value = serde_json::from_slice(data).ok()?;
    let stream = metadata["streams"].as_array()?.first()?;
    for (value, source) in [
        (&stream["duration"], "video_stream"),
        (&metadata["format"]["duration"], "container"),
    ] {
        let Some(seconds) = value.as_str().and_then(|text| text.parse::<f64>().ok()) else {
            continue;
        };
        let frames = (seconds * fps.num as f64 / fps.den as f64).round();
        let millis = (seconds * 1000.0).round();
        if seconds.is_finite()
            && frames >= 1.0
            && frames <= i64::MAX as f64
            && millis <= i64::MAX as f64
        {
            return Some(VideoDuration {
                duration_ms: millis as i64,
                frames: frames as i64,
                source,
            });
        }
    }
    None
}
fn probe_video_duration(
    program: &str,
    path: &Path,
    track: usize,
    fps: Fps,
) -> Option<VideoDuration> {
    let output = Command::new(program)
        .args([
            "-v",
            "error",
            "-select_streams",
            &format!("v:{track}"),
            "-show_entries",
            "stream=duration:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| video_duration_from_probe(&output.stdout, fps))
        .flatten()
}
fn chapter_data(
    cfg: &Config,
    base: &Path,
    ep: &Episode,
    plan: &EpisodePlan,
    resolved_cut: Option<&ResolvedCut>,
) -> Result<Option<ChapterData>> {
    let Some(c) = &ep.chapters else {
        return Ok(None);
    };
    let fps = Fps::parse(&cfg.project.fps)?;
    match c {
        ChapterSpec::File(path) => {
            let path = absolute(base, &expand(path, &plan.vars)?);
            ensure!(path.is_file(), "Chapter file not found: {}", path.display());
            Ok(Some(ChapterData {
                content: None,
                source: path,
                timing: None,
            }))
        }
        ChapterSpec::Frames(items) => {
            let entries = items
                .iter()
                .map(|e| {
                    ensure!(e.frame >= 0, "Negative chapter frame");
                    Ok((fps.millis(e.frame), e.name.clone()))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Some(ChapterData {
                content: Some(chapters::ogm(&entries)?),
                source: plan.source.clone(),
                timing: None,
            }))
        }
        ChapterSpec::Names(names) => {
            if plan.cut.is_some() {
                let owned_cut;
                let cut = if let Some(cut) = resolved_cut {
                    cut
                } else {
                    owned_cut = cuts::resolve(cfg, plan)?.context("Missing resolved cut")?;
                    &owned_cut
                };
                let mut times = cut
                    .chapter_ticks
                    .iter()
                    .map(|tick| {
                        ((*tick as f64 / 45.0) - cut.start_seconds * 1000.0)
                            .round()
                            .max(0.0) as i64
                    })
                    .collect::<Vec<_>>();
                if let Some(first) = times.first_mut() {
                    *first = 0;
                }
                let entries = chapters::name_times(times, names)?;
                return Ok(Some(ChapterData {
                    content: Some(chapters::ogm(&entries)?),
                    source: cut.playlist.clone(),
                    timing: None,
                }));
            }
            ensure!(
                plan.sources.len() == 1,
                "Episode {} uses multiple source clips; use explicit frame chapters or a TXT/XML chapter file",
                ep.id
            );
            let video = &plan.tracks[0];
            let track = if video.source == plan.source {
                video.track
            } else {
                0
            };
            let duration = probe_video_duration(&cfg.tools.ffprobe, &plan.source, track, fps);
            let (playlist, ch) = chapters::bdmv(
                &plan.source,
                None,
                fps,
                duration.as_ref().map(|value| value.frames),
            )?;
            let difference_ms = duration
                .as_ref()
                .map(|value| value.duration_ms - ch.duration_ms);
            if let Some(difference) = difference_ms {
                let tolerance_ms = (2000.0 * fps.den as f64 / fps.num as f64).ceil() as i64;
                if difference.abs() > tolerance_ms {
                    eprintln!(
                        "Warning: episode {} video duration differs from BDMV playlist by {} ms (video: {} ms; playlist: {} ms).",
                        ep.id,
                        difference,
                        duration.as_ref().unwrap().duration_ms,
                        ch.duration_ms
                    );
                }
            } else {
                eprintln!(
                    "Warning: episode {} video duration is unavailable; using BDMV playlist duration for chapter cutoff.",
                    ep.id
                );
            }
            let timing = ChapterTiming {
                playlist_duration_ms: ch.duration_ms,
                video_duration_ms: duration.as_ref().map(|value| value.duration_ms),
                video_duration_source: duration.as_ref().map(|value| value.source.into()),
                difference_ms,
                cutoff_frames: duration
                    .as_ref()
                    .map_or(ch.duration_frames, |value| value.frames),
                cutoff_source: duration
                    .as_ref()
                    .map_or("playlist", |value| value.source)
                    .into(),
            };
            let entries = chapters::name_times(ch.times, names)?;
            Ok(Some(ChapterData {
                content: Some(chapters::ogm(&entries)?),
                source: playlist,
                timing: Some(timing),
            }))
        }
    }
}
fn validate_fps(cfg: &Config, path: &Path, relative: usize) -> Result<()> {
    let out = Command::new(&cfg.tools.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            &format!("v:{relative}"),
            "-show_entries",
            "stream=avg_frame_rate,r_frame_rate",
            "-of",
            "json",
        ])
        .arg(path)
        .output()?;
    ensure!(
        out.status.success(),
        "ffprobe failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let stream = &v["streams"][0];
    let s = stream["avg_frame_rate"]
        .as_str()
        .filter(|s| *s != "0/0")
        .or_else(|| stream["r_frame_rate"].as_str())
        .context("Video has no FPS")?;
    let actual = Fps::parse(s)?;
    let configured = Fps::parse(&cfg.project.fps)?;
    ensure!(
        actual.num as i128 * configured.den as i128 == configured.num as i128 * actual.den as i128,
        "FPS of {} is {s}, configuration is {}",
        path.display(),
        cfg.project.fps
    );
    Ok(())
}
pub fn check(cfg: &Config, base: &Path, plans: &[EpisodePlan]) -> Result<()> {
    check_cached(cfg, base, plans, &mut FontCache::new())
}
fn check_cached(
    cfg: &Config,
    base: &Path,
    plans: &[EpisodePlan],
    cache: &mut FontCache,
) -> Result<()> {
    let mut identifiers = BTreeMap::new();
    for plan in plans {
        if plan.cut.is_some() {
            let cut = cuts::resolve(cfg, plan)?.context("Missing resolved cut")?;
            eprintln!(
                "{}: cut {:.9}..{:.9} s; start mark offset {:+.3} ms; end mark offset {}",
                plan.episode,
                cut.start_seconds,
                cut.end_seconds,
                cut.start_deviation_ms,
                cut.end_deviation_ms
                    .map_or_else(|| "playlist OUT".into(), |ms| format!("{ms:+.3} ms"))
            );
        }
        if plan.sources.len() > 1 {
            let mut previous: Option<Vec<(String, String)>> = None;
            for source in &plan.sources {
                let ident = identify(&cfg.tools.mkvmerge, source)?;
                let layout = ident
                    .tracks
                    .iter()
                    .map(|t| (t.kind.clone(), t.codec.clone()))
                    .collect::<Vec<_>>();
                if let Some(ref previous) = previous {
                    ensure!(
                        *previous == layout,
                        "Episode {} source clips have different track layouts or codecs: {}",
                        plan.episode,
                        source.display()
                    );
                }
                previous = Some(layout);
            }
        }
        let index = font_index(cfg, base, plan, cache)?;
        let mut used = BTreeSet::new();
        let mut check_video = plan.tracks[0].clone();
        if check_video.source == plan.source && plan.sources.len() > 1 {
            check_video.source = plan.sources[0].clone();
        }
        for tp in &plan.tracks {
            let t = config_track(cfg, &tp.id, &plan.episode);
            let check_source = if tp.source == plan.source && plan.sources.len() > 1 {
                &plan.sources[0]
            } else {
                &tp.source
            };
            if tp.category == "subtitle" && t.kind != "pgs" {
                let (_, doc) = load_sub(cfg, &t, check_source, &plan.episode, &check_video)?;
                if let Some(doc) = doc {
                    used.extend(requests(&doc)?);
                }
            } else {
                if !identifiers.contains_key(check_source) {
                    identifiers.insert(
                        check_source.clone(),
                        identify(&cfg.tools.mkvmerge, check_source)?,
                    );
                }
                if t.kind == "pgs" {
                    pgs_track(&identifiers[check_source], tp.track)?;
                } else {
                    track_id(&identifiers[check_source], &tp.category, tp.track)?;
                }
            }
        }
        if cfg.fonts.enabled {
            for req in used {
                if index.select(&req).is_none() {
                    eprintln!(
                        "Warning: font '{}' was not found (weight {}, italic {}).",
                        req.name, req.weight, req.italic
                    );
                }
            }
        }
        let video = &plan.tracks[0];
        if video.source == plan.source && plan.sources.len() > 1 {
            for source in &plan.sources {
                validate_fps(cfg, source, video.track)?;
            }
        } else {
            validate_fps(cfg, &video.source, video.track)?;
        }
        let ep = cfg
            .episodes
            .iter()
            .find(|ep| ep.id == plan.episode)
            .unwrap();
        chapter_data(cfg, base, ep, plan, None)?;
        eprintln!(
            "{}: {} tracks, paths/subtitles/fonts/chapters checked",
            plan.episode,
            plan.tracks.len()
        );
    }
    Ok(())
}
impl Audit {
    fn save(&self) -> Result<()> {
        std::fs::write(
            self.plan.work.join("audit.json"),
            serde_json::to_vec_pretty(self)?,
        )?;
        Ok(())
    }
    fn command(&mut self, program: &str, args: Vec<String>, warning_ok: bool) -> Result<()> {
        eprintln!(
            "Running {program} {}",
            args.iter()
                .map(|s| format!("{s:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        self.commands.push(CommandRecord {
            program: program.into(),
            args: args.clone(),
            status: None,
        });
        self.save()?;
        let status = Command::new(program)
            .args(&args)
            .status()
            .with_context(|| format!("Running {program}"))?;
        self.commands.last_mut().unwrap().status = status.code();
        self.save()?;
        ensure!(
            status.success() || (warning_ok && status.code() == Some(1)),
            "{program} exited with {status}"
        );
        Ok(())
    }
}
fn options(
    args: &mut Vec<String>,
    t: &Track,
    name: &str,
    id: u64,
    vars: &BTreeMap<String, String>,
    work: &Path,
) -> Result<()> {
    for (key, value) in [
        ("--track-name", name.to_owned()),
        ("--language", t.language.clone()),
        (
            "--default-track-flag",
            if t.default { "yes" } else { "no" }.into(),
        ),
        (
            "--forced-display-flag",
            if t.forced { "yes" } else { "no" }.into(),
        ),
        (
            "--hearing-impaired-flag",
            if t.hearing_impaired { "yes" } else { "no" }.into(),
        ),
        ("--sync", t.delay_ms.to_string()),
    ] {
        args.extend([key.into(), format!("{id}:{value}")]);
    }
    if !t.tags.is_empty() {
        let tags = t
            .tags
            .iter()
            .map(|(k, v)| Ok((k.clone(), expand(v, vars)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let path = work.join(format!("{}-tags.xml", t.id));
        std::fs::write(&path, tmdb::track_tags(&tags))?;
        args.extend(["--tags".into(), format!("{id}:{}", path.display())]);
    }
    for arg in &t.args {
        args.push(expand(arg, vars)?);
    }
    if let Some(colour) = &t.colour {
        for (flag, value) in [
            ("--colour-range", colour.range),
            ("--colour-primaries", colour.primaries),
            ("--colour-transfer-characteristics", colour.transfer),
            ("--colour-matrix", colour.matrix),
        ] {
            if let Some(value) = value {
                args.extend([flag.into(), format!("{id}:{value}")]);
            }
        }
    }
    Ok(())
}
fn add_input(
    args: &mut Vec<String>,
    tp: &TrackPlan,
    t: &Track,
    path: &Path,
    id: u64,
    vars: &BTreeMap<String, String>,
    work: &Path,
) -> Result<()> {
    if tp.category != "video" || !t.attachments {
        args.push("--no-attachments".into());
    }
    if tp.category != "video" || !t.global_tags {
        args.push("--no-global-tags".into());
    }
    args.push("--no-track-tags".into());
    if tp.category != "video" || !t.chapters {
        args.push("--no-chapters".into());
    }
    match tp.category.as_str() {
        "video" => {
            if !t.audio {
                args.push("--no-audio".into());
            }
            if !t.subtitles {
                args.push("--no-subtitles".into());
            }
            args.extend(["--video-tracks".into(), id.to_string()]);
        }
        "audio" => args.extend([
            "--no-video".into(),
            "--no-subtitles".into(),
            "--audio-tracks".into(),
            id.to_string(),
        ]),
        _ => args.extend([
            "--no-video".into(),
            "--no-audio".into(),
            "--subtitle-tracks".into(),
            id.to_string(),
        ]),
    }
    options(args, t, &tp.name, id, vars, work)?;
    args.push(path.to_string_lossy().into());
    Ok(())
}
fn run_episode(
    cfg: &Config,
    base: &Path,
    ep: &Episode,
    plan: &EpisodePlan,
    client: &mut Option<tmdb::Client>,
    cache: &mut FontCache,
) -> Result<()> {
    std::fs::create_dir_all(plan.work.parent().unwrap())?;
    std::fs::create_dir(&plan.work).with_context(|| {
        format!(
            "Workdir already exists; review it before removing: {}",
            plan.work.display()
        )
    })?;
    let mut audit = Audit {
        plan: plan.clone(),
        configuration: cfg.clone(),
        state: "running".into(),
        commands: Vec::new(),
        fonts: Vec::new(),
        error: None,
        chapter_source: None,
        chapter_timing: None,
        cut: None,
    };
    audit.save()?;
    let result = (|| -> Result<()> {
        let cut = cuts::resolve(cfg, plan)?;
        let mut cut_video_path = None;
        if let Some(ref cut) = cut {
            audit.cut = Some(CutAudit {
                resolved: cut.clone(),
                source_video_sha256: None,
                copied_video_sha256: None,
                final_video_sha256: None,
                source_video_packets: None,
                copied_video_packets: None,
                final_video_packets: None,
                source_audio_sha256: BTreeMap::new(),
                cut_audio_sha256: BTreeMap::new(),
                final_audio_sha256: BTreeMap::new(),
                source_audio_pcm_bytes: BTreeMap::new(),
                cut_audio_pcm_bytes: BTreeMap::new(),
                final_audio_pcm_bytes: BTreeMap::new(),
                audio_sample_ranges: BTreeMap::new(),
                pgs_sha256: BTreeMap::new(),
                first_video_ms: None,
                first_audio_ms: BTreeMap::new(),
            });
            audit.save()?;
            let video = &plan.tracks[0];
            let (path, source_hash) =
                cut_video(cfg, &mut audit, &plan.source, video.track, cut, &plan.work)?;
            ensure!(
                source_hash.1 == cut.video_frames,
                "Cut requested {} video frames but copied {}",
                cut.video_frames,
                source_hash.1
            );
            let copied_hash = video_sha256(cfg, &mut audit, &path, 0, None)?;
            ensure!(
                source_hash == copied_hash,
                "Cut video differs from the original H.264 picture data or packet count"
            );
            let record = audit.cut.as_mut().unwrap();
            record.source_video_sha256 = Some(source_hash.0);
            record.source_video_packets = Some(source_hash.1);
            record.copied_video_sha256 = Some(copied_hash.0);
            record.copied_video_packets = Some(copied_hash.1);
            audit.save()?;
            cut_video_path = Some(path);
        }
        if plan.sources.len() > 1 {
            let mut join = vec!["-o".into(), plan.source.to_string_lossy().into_owned()];
            join.push(plan.sources[0].to_string_lossy().into_owned());
            for source in plan.sources.iter().skip(1) {
                join.push(format!("+{}", source.display()));
            }
            audit.command(&cfg.tools.mkvmerge, join, true)?;
            ensure!(
                plan.source.is_file(),
                "mkvmerge did not create the joined source"
            );
        }
        let dir = plan.output.parent().unwrap();
        std::fs::create_dir_all(dir)?;
        // Temporary output in the destination filesystem; hard_link publishes without overwrite.
        let partial = dir.join(format!(
            ".{}.{}.partial.mkv",
            plan.output
                .file_stem()
                .context("Output has no filename stem")?
                .to_string_lossy(),
            std::process::id()
        ));
        ensure!(
            !partial.exists(),
            "Temporary output already exists: {}",
            partial.display()
        );
        let mut args = vec![
            "-o".into(),
            partial.to_string_lossy().into(),
            "--title".into(),
            plan.title.clone(),
        ];
        if let Some(chapter) = chapter_data(cfg, base, ep, plan, cut.as_ref())? {
            let path = if let Some(content) = chapter.content {
                let path = plan.work.join("chapters.txt");
                std::fs::write(&path, content)?;
                path
            } else {
                chapter.source.clone()
            };
            args.extend(["--chapters".into(), path.to_string_lossy().into()]);
            audit.chapter_source = Some(chapter.source);
            audit.chapter_timing = chapter.timing;
            audit.save()?;
        }
        if let (Some(t), Some(client)) = (&cfg.tmdb, client) {
            let path = plan.work.join("global-tags.xml");
            std::fs::write(&path, client.tags(t, ep)?)?;
            args.extend(["--global-tags".into(), path.to_string_lossy().into()]);
        }
        for arg in &cfg.mux_args {
            args.push(expand(arg, &plan.vars)?);
        }
        let index = font_index(cfg, base, plan, cache)?;
        let mut used = BTreeSet::new();
        let mut order = Vec::new();
        let mut cut_pgs = Vec::new();
        let mut cut_audio = BTreeMap::new();
        let mut subtitle_index = 0;
        for (file_index, tp) in plan.tracks.iter().enumerate() {
            let mut t = config_track(cfg, &tp.id, &ep.id);
            let (path, id) = if tp.category == "audio" {
                let selected_cut = cut.as_ref().filter(|_| tp.source == plan.source);
                let stream = audio_stream(&cfg.tools.ffprobe, &tp.source, tp.track)?;
                let range = if let Some(selected_cut) = selected_cut
                    && t.codec == "flac"
                {
                    Some(source_audio_sample_range(
                        cfg,
                        &tp.source,
                        tp.track,
                        &stream,
                        selected_cut,
                    )?)
                } else {
                    None
                };
                let prepared = extract_audio(
                    cfg,
                    &mut audit,
                    &t,
                    &tp.source,
                    &plan.work,
                    &stream,
                    selected_cut,
                    range.as_ref(),
                )?;
                if let Some(pcm) = &prepared.source_pcm {
                    let codec = pcm_codec(&stream);
                    let original = prepared
                        .source_pcm_hash
                        .clone()
                        .context("Missing source PCM hash from cut extraction")?;
                    let extracted = pcm_sha256(cfg, &mut audit, pcm, 0, codec, None)?;
                    let matches_source = original == extracted;
                    let record = audit.cut.as_mut().unwrap();
                    record.source_audio_sha256.insert(tp.id.clone(), original.0);
                    record
                        .source_audio_pcm_bytes
                        .insert(tp.id.clone(), original.1);
                    record
                        .cut_audio_sha256
                        .insert(tp.id.clone(), extracted.0.clone());
                    record
                        .cut_audio_pcm_bytes
                        .insert(tp.id.clone(), extracted.1);
                    record.audio_sample_ranges.insert(
                        tp.id.clone(),
                        prepared
                            .source_sample_range
                            .clone()
                            .context("Missing source audio sample range")?,
                    );
                    audit.save()?;
                    ensure!(
                        matches_source,
                        "Cut audio {} differs from corresponding source PCM samples (source {} bytes, cut {} bytes)",
                        tp.id,
                        original.1,
                        extracted.1
                    );
                    cut_audio.insert(tp.id.clone(), (extracted.0, extracted.1, codec));
                }
                for (key, value) in prepared.tags {
                    t.tags.entry(key).or_insert(value);
                }
                let id = track_id(&identify(&cfg.tools.mkvmerge, &prepared.path)?, "audio", 0)?;
                (prepared.path, id)
            } else if t.kind == "pgs" {
                let ident = identify(&cfg.tools.mkvmerge, &tp.source)?;
                let (id, subtitle_index) = pgs_track(&ident, t.track)?;
                if tp
                    .source
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("sup"))
                {
                    (tp.source.clone(), id)
                } else {
                    let extracted = plan.work.join(format!("{}.sup", t.id));
                    let selected_cut = cut.as_ref().filter(|_| tp.source == plan.source);
                    let full = selected_cut.map(|_| plan.work.join(format!("{}.full.sup", t.id)));
                    let target = full.as_ref().unwrap_or(&extracted);
                    let mut extract_args = vec![
                        "-nostdin".into(),
                        "-hide_banner".into(),
                        "-loglevel".into(),
                        "error".into(),
                        "-n".into(),
                    ];
                    extract_args.extend(["-i".into(), tp.source.to_string_lossy().into()]);
                    extract_args.extend([
                        "-map".into(),
                        format!("0:s:{subtitle_index}"),
                        "-c".into(),
                        "copy".into(),
                        target.to_string_lossy().into(),
                    ]);
                    audit.command(&cfg.tools.ffmpeg, extract_args, false)?;
                    if let Some(cut) = selected_cut {
                        let first_source =
                            first_packet_ms(&cfg.tools.ffprobe, &tp.source, "s", subtitle_index)?;
                        let cropped = pgs::cut(
                            &std::fs::read(full.as_ref().unwrap())?,
                            (first_source * 90.0).round() as i64,
                            (cut.start_seconds * 90000.0).round() as i64,
                            (cut.end_seconds * 90000.0).round() as i64,
                        )?;
                        std::fs::write(&extracted, cropped)?;
                    }
                    let id = pgs_track(&identify(&cfg.tools.mkvmerge, &extracted)?, 0)?.0;
                    (extracted, id)
                }
            } else if tp.category == "subtitle" {
                let (text, doc) = load_sub(cfg, &t, &tp.source, &ep.id, &plan.tracks[0])?;
                let ext = if let Some(doc) = doc {
                    used.extend(requests(&doc)?);
                    "ass"
                } else {
                    "srt"
                };
                let path = plan.work.join(format!("{}.{}", t.id, ext));
                std::fs::write(&path, text)?;
                let id = track_id(&identify(&cfg.tools.mkvmerge, &path)?, "subtitle", 0)?;
                (path, id)
            } else {
                let path = if tp.category == "video" && tp.source == plan.source {
                    cut_video_path.as_ref().unwrap_or(&tp.source).clone()
                } else {
                    tp.source.clone()
                };
                let ident = identify(&cfg.tools.mkvmerge, &path)?;
                let id = track_id(
                    &ident,
                    &tp.category,
                    if path == tp.source { t.track } else { 0 },
                )?;
                (path, id)
            };
            if tp.category == "subtitle" {
                if t.kind == "pgs" && tp.source == plan.source && cut.is_some() {
                    cut_pgs.push((t.id.clone(), path.clone(), subtitle_index));
                }
                subtitle_index += 1;
            }
            order.push(format!("{file_index}:{id}"));
            if tp.category == "video" && (t.audio || t.subtitles) {
                let ident = identify(&cfg.tools.mkvmerge, &path)?;
                for kind in ["audio", "subtitles"] {
                    if (kind == "audio" && t.audio) || (kind == "subtitles" && t.subtitles) {
                        order.extend(
                            ident
                                .tracks
                                .iter()
                                .filter(|track| track.kind == kind)
                                .map(|track| format!("{file_index}:{}", track.id)),
                        );
                    }
                }
            }
            add_input(&mut args, tp, &t, &path, id, &plan.vars, &plan.work)?;
        }
        if cfg.fonts.enabled {
            audit.fonts = index.collect(&used, &plan.work.join("fonts"))?;
            for font in &audit.fonts {
                let extension = font.path.extension().and_then(|s| s.to_str()).unwrap_or("");
                let mime = match extension {
                    "otf" => "font/otf",
                    "ttc" | "otc" => "font/collection",
                    _ => "font/ttf",
                };
                args.extend([
                    "--attachment-mime-type".into(),
                    mime.into(),
                    "--attachment-name".into(),
                    font.path.file_name().unwrap().to_string_lossy().into(),
                    "--attach-file".into(),
                    font.path.to_string_lossy().into(),
                ]);
            }
        }
        args.extend(["--track-order".into(), order.join(",")]);
        audit.command(&cfg.tools.mkvmerge, args, true)?;
        ensure!(partial.is_file(), "mkvmerge did not produce an output file");
        if cut.is_some() {
            let final_hash = video_sha256(cfg, &mut audit, &partial, 0, None)?;
            ensure!(
                audit.cut.as_ref().unwrap().source_video_sha256.as_deref()
                    == Some(final_hash.0.as_str())
                    && audit.cut.as_ref().unwrap().source_video_packets == Some(final_hash.1),
                "Final MKV video differs from the original H.264 picture data or packet count"
            );
            audit.cut.as_mut().unwrap().final_video_sha256 = Some(final_hash.0);
            audit.cut.as_mut().unwrap().final_video_packets = Some(final_hash.1);
            let first_video = first_packet_ms(&cfg.tools.ffprobe, &partial, "v", 0)?;
            audit.cut.as_mut().unwrap().first_video_ms = Some(first_video);
            let mut final_audio = 0;
            for tp in &plan.tracks {
                if tp.category == "audio" {
                    if tp.source == plan.source {
                        let source_pcm = cut_audio
                            .get(&tp.id)
                            .context("Missing extracted PCM reference")?
                            .clone();
                        let final_pcm =
                            pcm_sha256(cfg, &mut audit, &partial, final_audio, source_pcm.2, None)?;
                        let matches_cut =
                            source_pcm.0 == final_pcm.0 && source_pcm.1 == final_pcm.1;
                        audit
                            .cut
                            .as_mut()
                            .unwrap()
                            .final_audio_sha256
                            .insert(tp.id.clone(), final_pcm.0);
                        audit
                            .cut
                            .as_mut()
                            .unwrap()
                            .final_audio_pcm_bytes
                            .insert(tp.id.clone(), final_pcm.1);
                        audit.save()?;
                        ensure!(
                            matches_cut,
                            "Final MKV audio {} differs from the original PCM samples",
                            tp.id
                        );
                        let first_audio =
                            first_packet_ms(&cfg.tools.ffprobe, &partial, "a", final_audio)?;
                        audit
                            .cut
                            .as_mut()
                            .unwrap()
                            .first_audio_ms
                            .insert(tp.id.clone(), first_audio);
                        ensure!(
                            (first_audio - first_video).abs() <= 1.0,
                            "Audio {} starts {:.3} ms away from video",
                            tp.id,
                            first_audio - first_video
                        );
                    }
                    final_audio += 1;
                }
            }
            for (id, path, index) in cut_pgs {
                let extracted = pgs_sha256(cfg, &mut audit, &path, 0)?;
                let muxed = pgs_sha256(cfg, &mut audit, &partial, index)?;
                ensure!(
                    extracted == muxed,
                    "Final MKV PGS {id} differs from the extracted PGS packets"
                );
                audit.cut.as_mut().unwrap().pgs_sha256.insert(id, extracted);
            }
            audit.save()?;
        }
        std::fs::hard_link(&partial, &plan.output).with_context(|| {
            format!(
                "Publishing {} without overwriting; temporary file kept at {}",
                plan.output.display(),
                partial.display()
            )
        })?;
        std::fs::remove_file(partial)?;
        audit.state = "complete".into();
        audit.save()?;
        let report = plan.output.with_extension("audit.json");
        ensure!(
            !report.exists(),
            "Audit report already exists: {}",
            report.display()
        );
        std::fs::copy(plan.work.join("audit.json"), report)?;
        Ok(())
    })();
    if let Err(e) = &result {
        audit.state = "failed".into();
        audit.error = Some(format!("{e:#}"));
        audit.save()?;
    }
    if result.is_ok() && cfg.project.clean_work_dirs {
        std::fs::remove_dir_all(&plan.work)?;
    }
    result
}
pub fn run(cfg: &Config, base: &Path, plans: &[EpisodePlan]) -> Result<()> {
    for plan in plans {
        ensure!(
            !plan.output.exists(),
            "Output already exists: {}",
            plan.output.display()
        );
        ensure!(
            !plan.output.with_extension("audit.json").exists(),
            "Audit report already exists: {}",
            plan.output.with_extension("audit.json").display()
        );
        ensure!(
            !plan.work.exists(),
            "Workdir already exists: {}",
            plan.work.display()
        );
        ensure!(
            plan.tracks.iter().all(|t| t.source != plan.output
                && (t.source == plan.source || !t.source.starts_with(&plan.work))),
            "Input conflicts with output or workdir"
        );
        ensure!(
            plan.sources
                .iter()
                .all(|s| s != &plan.output && !s.starts_with(&plan.work)),
            "Source clip conflicts with output or workdir"
        );
    }
    let mut cache = FontCache::new();
    check_cached(cfg, base, plans, &mut cache)?;
    let mut client = cfg.tmdb.as_ref().map(tmdb::Client::new).transpose()?;
    let mut errors = Vec::new();
    for plan in plans {
        let ep = cfg.episodes.iter().find(|e| e.id == plan.episode).unwrap();
        eprintln!("Muxing episode {}", ep.id);
        if let Err(e) = run_episode(cfg, base, ep, plan, &mut client, &mut cache) {
            eprintln!("{}: {e:#}", ep.id);
            errors.push(ep.id.clone());
            if !cfg.project.continue_on_error {
                return Err(e);
            }
        }
    }
    ensure!(
        errors.is_empty(),
        "Batch finished with failures in episodes: {}",
        errors.join(", ")
    );
    Ok(())
}

#[cfg(test)]
mod pipeline_tests {
    use super::{matrix_from_metadata, video_duration_from_probe};
    use crate::subtitle::Fps;

    #[test]
    fn video_matrix_preserves_video_range() {
        assert_eq!(matrix_from_metadata("bt709", "tv"), "TV.709");
        assert_eq!(matrix_from_metadata("bt709", "pc"), "PC.709");
        assert_eq!(matrix_from_metadata("bt470bg", "tv"), "TV.601");
        assert_eq!(matrix_from_metadata("bt2020nc", "tv"), "None");
    }

    #[test]
    fn video_duration_prefers_video_stream_and_falls_back_to_container() {
        let fps = Fps::parse("24000/1001").unwrap();
        let stream = br#"{"streams":[{"duration":"11.011000"}],"format":{"duration":"12.000000"}}"#;
        let duration = video_duration_from_probe(stream, fps).unwrap();
        assert_eq!(duration.source, "video_stream");
        assert_eq!(duration.frames, 264);
        assert_eq!(duration.duration_ms, 11011);

        let container = br#"{"streams":[{}],"format":{"duration":"12.012000"}}"#;
        let duration = video_duration_from_probe(container, fps).unwrap();
        assert_eq!(duration.source, "container");
        assert_eq!(duration.frames, 288);
        assert!(
            video_duration_from_probe(br#"{"streams":[],"format":{"duration":"12"}}"#, fps)
                .is_none()
        );
    }
}
