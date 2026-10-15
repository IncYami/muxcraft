use crate::{
    chapters,
    config::{ChapterSpec, Config, Episode, EpisodePlan, Track, TrackPlan, absolute, expand},
    cuts::{self, ResolvedCut},
    fonts::{FontIndex, requests},
    pgs,
    subtitle::{self, Ass, Fps, SubtitleCutoff},
    tmdb,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::Command,
};

fn ffmpeg_output(program: &str, args: &[String], label: &str) -> Result<std::process::Output> {
    use std::process::Stdio;
    let frame_total = args
        .windows(2)
        .find(|pair| pair[0] == "-frames:v")
        .and_then(|pair| pair[1].parse().ok());
    let time_total = args
        .windows(2)
        .find(|pair| pair[0] == "-t")
        .and_then(|pair| pair[1].parse::<f64>().ok())
        .map(|seconds| (seconds * 1_000_000.0).round() as u64);
    let total = frame_total.or(time_total);
    let progress = crate::ui::Progress::new(label, total);
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().context("Missing FFmpeg stdout")?;
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr = child.stderr.take().context("Missing FFmpeg stderr")?;
    let mut stderr_bytes = Vec::new();
    for line in BufReader::new(stderr).lines() {
        let line = line?;
        if let Some((key, Some(value))) = crate::ui::ffmpeg_progress_line(&line) {
            if (key == "frame" && frame_total.is_some())
                || (key == "time" && frame_total.is_none() && time_total.is_some())
            {
                progress.update(value);
            }
        }
        stderr_bytes.extend_from_slice(line.as_bytes());
        stderr_bytes.push(b'\n');
    }
    let stdout = stdout_thread
        .join()
        .map_err(|_| anyhow::anyhow!("FFmpeg stdout reader panicked"))??;
    let status = child.wait()?;
    progress.finish();
    if status.success() {
        for line in String::from_utf8_lossy(&stderr_bytes).lines() {
            if crate::ui::ffmpeg_progress_line(line).is_none() {
                crate::ui::tool_output_line(line);
            }
        }
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr: stderr_bytes,
    })
}

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
struct CommandRunner<'a> {
    cfg: &'a Config,
}
#[derive(Debug, Clone)]
struct AudioSampleRange {
    first_source_sample_pts_seconds: f64,
    sample_rate: u32,
    start_sample: u64,
    end_sample: u64,
    decoded_end_sample: u64,
}
#[derive(Debug, Clone)]
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
    crate::ui::command(program, &["-J".into(), path.to_string_lossy().into_owned()]);
    let out = Command::new(program)
        .args(["-J"])
        .arg(path)
        .output()
        .with_context(|| format!("Running {program}"))?;
    crate::ui::diagnostic(&String::from_utf8_lossy(&out.stderr));
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
fn cached_identify<'a>(
    cache: &'a mut BTreeMap<(String, PathBuf), Identification>,
    program: &str,
    path: &Path,
) -> Result<&'a Identification> {
    let key = (program.to_owned(), path.to_owned());
    if !cache.contains_key(&key) {
        cache.insert(key.clone(), identify(program, path)?);
    }
    Ok(cache.get(&key).unwrap())
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
    let args = vec![
        "-v".into(), "error".into(), "-select_streams".into(), format!("a:{track}"),
        "-show_entries".into(), "stream=codec_name,sample_rate,channels,time_base,bits_per_raw_sample,bits_per_sample,sample_fmt".into(),
        "-of".into(), "json".into(), path.to_string_lossy().into_owned(),
    ];
    crate::ui::command(program, &args);
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
    crate::ui::diagnostic(&String::from_utf8_lossy(&out.stderr));
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
    let args = vec![
        "-v".into(),
        "error".into(),
        "-select_streams".into(),
        format!("a:{track}"),
        "-read_intervals".into(),
        "%+#1".into(),
        "-show_frames".into(),
        "-show_entries".into(),
        "frame=pts".into(),
        "-of".into(),
        "json".into(),
        source.to_string_lossy().into_owned(),
    ];
    crate::ui::command(&cfg.tools.ffprobe, &args);
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
    crate::ui::diagnostic(&String::from_utf8_lossy(&output.stderr));
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
    crate::ui::command(program, &["--version".into()]);
    let output = Command::new(program).arg("--version").output().ok()?;
    crate::ui::diagnostic(&String::from_utf8_lossy(&output.stderr));
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
    crate::ui::command(&cfg.tools.ffmpeg, &args);
    let result = ffmpeg_output(&cfg.tools.ffmpeg, &args, "Cut video")?;
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

fn video_sha256(cfg: &Config, source: &Path, track: usize) -> Result<(String, u64)> {
    let mut args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:2".into(),
    ];
    args.extend(["-i".into(), source.to_string_lossy().into()]);
    args.extend([
        "-map".into(),
        format!("0:v:{track}"),
        "-c:v".into(),
        "copy".into(),
        "-bsf:v".into(),
        "h264_mp4toannexb,filter_units=pass_types=1-5".into(),
    ]);
    args.extend([
        "-f".into(),
        "hash".into(),
        "-hash".into(),
        "SHA256".into(),
        "-".into(),
    ]);
    crate::ui::command(&cfg.tools.ffmpeg, &args);
    let output = ffmpeg_output(&cfg.tools.ffmpeg, &args, "Verify video")?;
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

fn pcm_sha256(cfg: &Config, source: &Path, track: usize, codec: &str) -> Result<(String, u64)> {
    let mut args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
    ];
    args.extend(["-i".into(), source.to_string_lossy().into()]);
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
    args.splice(
        0..0,
        ["-progress".into(), "pipe:2".into(), "-nostats".into()],
    );
    crate::ui::command(&cfg.tools.ffmpeg, &args);
    let progress = crate::ui::Progress::new(&format!("Audio PCM track {track}"), None);
    let mut child = Command::new(&cfg.tools.ffmpeg)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let mut output = child.stdout.take().context("Missing FFmpeg PCM output")?;
    let error_output = child
        .stderr
        .take()
        .context("Missing FFmpeg diagnostic output")?;
    let error_bar = progress.clone();
    let error_thread =
        std::thread::spawn(move || crate::ui::collect_ffmpeg_progress(error_output, error_bar));
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
    let status = child.wait()?;
    progress.finish();
    let stderr = error_thread
        .join()
        .map_err(|_| anyhow::anyhow!("FFmpeg diagnostic reader panicked"))??;
    ensure!(
        status.success(),
        "PCM hash failed: {}",
        String::from_utf8_lossy(&stderr)
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
    let args = vec![
        "-v".into(),
        "error".into(),
        "-select_streams".into(),
        format!("{kind}:{track}"),
        "-read_intervals".into(),
        "%+#1".into(),
        "-show_packets".into(),
        "-show_entries".into(),
        "packet=pts_time".into(),
        "-of".into(),
        "default=noprint_wrappers=1:nokey=1".into(),
        source.to_string_lossy().into_owned(),
    ];
    crate::ui::command(program, &args);
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
    crate::ui::diagnostic(&String::from_utf8_lossy(&result.stderr));
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

fn extract_audio(
    cfg: &Config,
    runner: &CommandRunner<'_>,
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
            "always".into(),
            "-bitexact".into(),
            extracted.to_string_lossy().into(),
            "-map".into(),
            "[source_out]".into(),
            "-c:a".into(),
            codec.into(),
            "-f".into(),
            format.into(),
            "-".into(),
        ];
        let mut args = args;
        args.splice(
            0..0,
            ["-progress".into(), "pipe:2".into(), "-nostats".into()],
        );
        crate::ui::detail(&format!(
            "Extract cut audio and hash source PCM: {}",
            track.id
        ));
        crate::ui::command(&cfg.tools.ffmpeg, &args);
        let progress = crate::ui::Progress::new(&format!("Audio {} PCM", track.id), None);
        let mut child = Command::new(&cfg.tools.ffmpeg)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let mut output = child.stdout.take().context("Missing FFmpeg PCM output")?;
        let error_output = child
            .stderr
            .take()
            .context("Missing FFmpeg diagnostic output")?;
        let error_bar = progress.clone();
        let error_thread =
            std::thread::spawn(move || crate::ui::collect_ffmpeg_progress(error_output, error_bar));
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
        let status = child.wait()?;
        progress.finish();
        let stderr = error_thread
            .join()
            .map_err(|_| anyhow::anyhow!("FFmpeg diagnostic reader panicked"))??;
        ensure!(
            status.success(),
            "Cut audio extraction and source PCM hash failed: {}",
            String::from_utf8_lossy(&stderr)
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
        let hash = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        crate::ui::detail(&format!(
            "{} source PCM SHA-256: {} ({} bytes)",
            track.id, hash, bytes
        ));
        Some((hash, bytes))
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
                "always".into(),
                "-bitexact".into(),
            ]);
        } else {
            args.extend(["-c:a".into(), "copy".into()]);
            if extension == "dts" {
                args.extend(["-f".into(), "dts".into()]);
            }
        }
        args.push(extracted.to_string_lossy().into());
        runner.command(&cfg.tools.ffmpeg, args, false)?;
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
        runner.command(&cfg.tools.ffmpeg, args, false)?;
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
    runner.command(&cfg.tools.flac, args, false)?;
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
fn is_pgs(cfg: &Config, id: &str) -> bool {
    cfg.pgs.iter().any(|track| track.id == id)
}
fn matrix_from_metadata(space: &str, video_range: &str) -> String {
    let matrix = match space {
        "bt709" => "709",
        "smpte170m" | "bt470bg" => "601",
        "smpte240m" => "240M",
        "fcc" => "FCC",
        unknown => {
            crate::ui::warning(&format!(
                "Video matrix '{unknown}' cannot be represented in ASS; using None."
            ));
            return "None".into();
        }
    };
    let range = match video_range {
        "pc" => "PC",
        "tv" => "TV",
        unknown => {
            crate::ui::warning(&format!(
                "Video range '{unknown}' is unspecified; assuming limited range."
            ));
            "TV"
        }
    };
    format!("{range}.{matrix}")
}
fn video_matrix(cfg: &Config, video: &TrackPlan) -> Result<String> {
    let args = vec![
        "-v".into(),
        "error".into(),
        "-select_streams".into(),
        format!("v:{}", video.track),
        "-show_entries".into(),
        "stream=color_space,color_range".into(),
        "-of".into(),
        "json".into(),
        video.source.to_string_lossy().into_owned(),
    ];
    crate::ui::command(&cfg.tools.ffprobe, &args);
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
    crate::ui::diagnostic(&String::from_utf8_lossy(&out.stderr));
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
    cutoff: Option<SubtitleCutoff>,
) -> Result<(String, Option<Ass>)> {
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let is_srt = extension.eq_ignore_ascii_case("srt");
    let is_ass = extension.eq_ignore_ascii_case("ass");
    if !(is_srt || is_ass) {
        if extension.eq_ignore_ascii_case("sup") {
            bail!("PGS subtitle {} belongs in [pgs]", t.id);
        } else if extension.is_empty() {
            bail!("Subtitle {} has no extension; expected .ass or .srt", t.id);
        } else {
            bail!(
                "Subtitle {} has unsupported extension .{}; expected .ass or .srt",
                t.id,
                extension
            );
        }
    }
    let input = std::fs::read_to_string(path)
        .with_context(|| format!("UTF-8 subtitle: {}", path.display()))?;
    let default = crate::config::Correction::default();
    let correction = if let Some(table) = t.shift_table.as_ref() {
        table.episodes.get(ep).unwrap_or(&table.default)
    } else {
        &default
    };
    let fps = Fps::parse(&cfg.project.fps)?;
    if is_srt {
        ensure!(
            t.headers.is_empty() && t.clean.is_empty() && t.ycbcr_matrix.is_none(),
            "Subtitle {}: ASS headers and cleaning are not valid for SRT",
            t.id
        );
        return Ok((
            subtitle::process_srt(&input, t, correction, ep, fps, cutoff)?,
            None,
        ));
    }
    if is_ass {
        let mut effective = t.clone();
        for (enabled, operation) in [
            (cfg.subtitle_processing.clean_garbage, "garbage"),
            (cfg.subtitle_processing.clean_extradata, "extradata"),
        ] {
            if enabled && !effective.clean.iter().any(|op| op == operation) {
                effective.clean.push(operation.into());
            }
        }
        if effective.ycbcr_matrix.is_none() {
            effective.ycbcr_matrix = cfg.subtitle_processing.ycbcr_matrix.clone();
        }
        if let Some(matrix) = &effective.ycbcr_matrix {
            ensure!(
                [
                    "auto", "None", "TV.601", "PC.601", "TV.709", "PC.709", "TV.240M", "PC.240M",
                    "TV.FCC", "PC.FCC"
                ]
                .contains(&matrix.as_str()),
                "Invalid YCbCr matrix for {}: {matrix}",
                t.id
            );
            ensure!(
                !effective.headers.contains_key("YCbCr Matrix"),
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
        let ass = subtitle::process_ass(&input, &effective, correction, ep, fps, cutoff)?;
        return Ok((ass.dump(), Some(ass)));
    }
    unreachable!("validated subtitle extension")
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
            let is_ass = track
                .source
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ass"));
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
    let args = vec![
        "-v".into(),
        "error".into(),
        "-select_streams".into(),
        format!("v:{track}"),
        "-show_entries".into(),
        "stream=duration:format=duration".into(),
        "-of".into(),
        "json".into(),
        path.to_string_lossy().into_owned(),
    ];
    crate::ui::command(program, &args);
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
fn subtitle_cutoff(
    cfg: &Config,
    plan: &EpisodePlan,
    cut: Option<&ResolvedCut>,
) -> Result<Option<SubtitleCutoff>> {
    let Some(mode) = &cfg.subtitle_processing.truncate_video else {
        return Ok(None);
    };
    let fps = Fps::parse(&cfg.project.fps)?;
    let end_ms = if plan.cut.is_some() {
        let cut = cut.context("Missing resolved cut")?;
        ((cut.video_frames as i128 * fps.den as i128 * 1000 + fps.num as i128 / 2)
            / fps.num as i128) as i64
    } else {
        let video = &plan.tracks[0];
        let sources = if plan.sources.len() > 1 && video.source == plan.source {
            &plan.sources
        } else {
            std::slice::from_ref(&video.source)
        };
        let mut duration = 0i64;
        for source in sources {
            let measured = probe_video_duration(&cfg.tools.ffprobe, source, video.track, fps)
                .with_context(|| {
                    format!(
                        "Cannot determine video end for subtitle truncation: {}",
                        source.display()
                    )
                })?;
            duration = duration
                .checked_add(measured.duration_ms)
                .context("Video duration overflow")?;
        }
        duration
    };
    ensure!(
        end_ms > 0,
        "Video end must be positive for subtitle truncation"
    );
    Ok(Some(SubtitleCutoff {
        end_ms,
        trimmed: mode.trimmed,
        comment: mode.comment,
    }))
}
fn has_text_subtitles(cfg: &Config, plan: &EpisodePlan) -> bool {
    plan.tracks
        .iter()
        .any(|track| track.category == "subtitle" && !is_pgs(cfg, &track.id))
}
fn chapter_data(
    cfg: &Config,
    base: &Path,
    ep: &Episode,
    plan: &EpisodePlan,
    resolved_cut: Option<&ResolvedCut>,
) -> Result<Option<ChapterData>> {
    let fps = Fps::parse(&cfg.project.fps)?;
    if ep.chapters.is_none() {
        if cfg
            .video
            .iter()
            .any(|track| track.chapters && track.enabled(&ep.id))
        {
            return Ok(None);
        }
        if plan.sources.len() < 2 {
            return Ok(None);
        }
        let Some((playlist, chapters)) = chapters::bdmv_joined(&plan.sources, fps)? else {
            if plan.sources[0]
                .parent()
                .is_some_and(|parent| parent.file_name().is_some_and(|name| name == "STREAM"))
            {
                crate::ui::warning(&format!(
                    "Episode {} has no MPLS playlist matching its ordered clips; no chapters will be added.",
                    ep.id
                ));
            }
            return Ok(None);
        };
        let entries = chapters::name_times(chapters.times, &[])?;
        crate::ui::detail(&format!(
            "Auto chapters: {} marks from {}",
            entries.len(),
            playlist.display()
        ));
        return Ok(Some(ChapterData {
            content: Some(chapters::ogm(&entries)?),
            source: playlist,
            timing: None,
        }));
    }
    let c = ep.chapters.as_ref().unwrap();
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
            if plan.sources.len() > 1 {
                let (playlist, chapters) = chapters::bdmv_joined(&plan.sources, fps)?
                    .context("No MPLS playlist matches the ordered chapter source clips")?;
                let entries = chapters::name_times(chapters.times, names)?;
                crate::ui::detail(&format!(
                    "BDMV chapters: {} marks from {}",
                    entries.len(),
                    playlist.display()
                ));
                return Ok(Some(ChapterData {
                    content: Some(chapters::ogm(&entries)?),
                    source: playlist,
                    timing: None,
                }));
            }
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
                    crate::ui::warning(&format!(
                        "Episode {} video duration differs from BDMV playlist by {} ms (video: {} ms; playlist: {} ms).",
                        ep.id,
                        difference,
                        duration.as_ref().unwrap().duration_ms,
                        ch.duration_ms
                    ));
                }
            } else {
                crate::ui::warning(&format!(
                    "Episode {} video duration is unavailable; using BDMV playlist duration for chapter cutoff.",
                    ep.id
                ));
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
    let args = vec![
        "-v".into(),
        "error".into(),
        "-select_streams".into(),
        format!("v:{relative}"),
        "-show_entries".into(),
        "stream=avg_frame_rate,r_frame_rate".into(),
        "-of".into(),
        "json".into(),
        path.to_string_lossy().into_owned(),
    ];
    crate::ui::command(&cfg.tools.ffprobe, &args);
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
    crate::ui::diagnostic(&String::from_utf8_lossy(&out.stderr));
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
    let started = std::time::Instant::now();
    crate::ui::batch("Check", plans.len());
    let mut cache = FontCache::new();
    let mut identifiers = BTreeMap::new();
    if cfg.episode_configs.is_empty() {
        check_cached(cfg, base, plans, &mut cache, &mut identifiers)?;
    } else {
        for plan in plans {
            check_cached(
                cfg.for_episode(&plan.episode),
                base,
                std::slice::from_ref(plan),
                &mut cache,
                &mut identifiers,
            )?;
        }
    }
    crate::ui::checked(plans.len(), started.elapsed());
    Ok(())
}
fn check_cached(
    cfg: &Config,
    base: &Path,
    plans: &[EpisodePlan],
    cache: &mut FontCache,
    identifiers: &mut BTreeMap<(String, PathBuf), Identification>,
) -> Result<()> {
    for plan in plans {
        let check_step = crate::ui::Step::new(format!("Check episode {}", plan.episode));
        let resolved_cut = if plan.cut.is_some() {
            let cut = cuts::resolve(cfg, plan)?.context("Missing resolved cut")?;
            crate::ui::detail(&format!(
                "Cut {:.9}..{:.9} s; start mark offset {:+.3} ms; end mark offset {}",
                cut.start_seconds,
                cut.end_seconds,
                cut.start_deviation_ms,
                cut.end_deviation_ms
                    .map_or_else(|| "playlist OUT".into(), |ms| format!("{ms:+.3} ms"))
            ));
            Some(cut)
        } else {
            None
        };
        if plan.sources.len() > 1 {
            let mut previous: Option<Vec<(String, String)>> = None;
            for source in &plan.sources {
                let ident = cached_identify(identifiers, &cfg.tools.mkvmerge, source)?;
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
        let cutoff = if has_text_subtitles(cfg, plan) {
            subtitle_cutoff(cfg, plan, resolved_cut.as_ref())?
        } else {
            None
        };
        for tp in &plan.tracks {
            let t = config_track(cfg, &tp.id, &plan.episode);
            let check_source = if tp.source == plan.source && plan.sources.len() > 1 {
                &plan.sources[0]
            } else {
                &tp.source
            };
            if tp.category == "subtitle" && !is_pgs(cfg, &tp.id) {
                let (_, doc) =
                    load_sub(cfg, &t, check_source, &plan.episode, &check_video, cutoff)?;
                if let Some(doc) = doc {
                    used.extend(requests(&doc)?);
                }
            } else {
                let ident = cached_identify(identifiers, &cfg.tools.mkvmerge, check_source)?;
                if is_pgs(cfg, &tp.id) {
                    pgs_track(ident, tp.track)?;
                } else {
                    track_id(ident, &tp.category, tp.track)?;
                }
            }
        }
        if cfg.fonts.enabled {
            for req in used {
                if index.select(&req).is_none() {
                    crate::ui::warning(&format!(
                        "Font '{}' was not found (weight {}, italic {}).",
                        req.name, req.weight, req.italic
                    ));
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
        crate::ui::detail(&format!(
            "{} tracks; paths, subtitles, fonts, and chapters checked",
            plan.tracks.len()
        ));
        check_step.done();
    }
    Ok(())
}
impl CommandRunner<'_> {
    fn command(&self, program: &str, mut args: Vec<String>, warning_ok: bool) -> Result<()> {
        let mkvmerge = program == self.cfg.tools.mkvmerge;
        let ffmpeg = program == self.cfg.tools.ffmpeg;
        if mkvmerge && !args.iter().any(|arg| arg == "--gui-mode") {
            args.insert(0, "--gui-mode".into());
        }
        if ffmpeg {
            args.splice(
                0..0,
                ["-progress".into(), "pipe:2".into(), "-nostats".into()],
            );
        }
        crate::ui::command(program, &args);
        let mut tool_error = None;
        let status = if mkvmerge {
            crate::ui::run_mkvmerge(program, &args)?.status
        } else if ffmpeg {
            let output = ffmpeg_output(program, &args, "FFmpeg")?;
            if !output.status.success() {
                tool_error = Some(String::from_utf8_lossy(&output.stderr).trim().to_owned());
            }
            output.status
        } else {
            let output = crate::ui::run_tool(program, &args)
                .with_context(|| format!("Running {program}"))?;
            if !output.status.success() {
                tool_error = Some(String::from_utf8_lossy(&output.stderr).trim().to_owned());
            }
            output.status
        };
        ensure!(
            status.success() || (warning_ok && status.code() == Some(1)),
            "{program} exited with {status}{}",
            tool_error
                .map(|message| format!(": {message}"))
                .unwrap_or_default()
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
    let runner = CommandRunner { cfg };
    let result = (|| -> Result<()> {
        let cut = cuts::resolve(cfg, plan)?;
        if let Some(cut) = &cut {
            crate::ui::detail(&format!(
                "Cut interval: {:.6}–{:.6} s; chapters {}..{}; {} video frames",
                cut.start_seconds,
                cut.end_seconds,
                cut.start_chapter,
                cut.end_chapter
                    .map_or_else(|| "playlist OUT".into(), |value| value.to_string()),
                cut.video_frames
            ));
            crate::ui::detail(&format!(
                "Playlist: {}; keyframe boundary deviations: start {:.3} ms, end {}",
                cut.playlist.display(),
                cut.start_deviation_ms,
                cut.end_deviation_ms
                    .map_or_else(|| "playlist OUT".into(), |value| format!("{value:.3} ms"))
            ));
        }
        let cutoff = if has_text_subtitles(cfg, plan) {
            subtitle_cutoff(cfg, plan, cut.as_ref())?
        } else {
            None
        };
        let mut cut_video_path = None;
        if let Some(ref cut) = cut {
            let cut_step = crate::ui::Step::new("Cut video and hash source");
            let video = &plan.tracks[0];
            let (path, source_hash) = cut_video(cfg, &plan.source, video.track, cut, &plan.work)?;
            crate::ui::detail(&format!(
                "Source video SHA-256: {} ({} packets)",
                source_hash.0, source_hash.1
            ));
            ensure!(
                source_hash.1 == cut.video_frames,
                "Cut requested {} video frames but copied {}",
                cut.video_frames,
                source_hash.1
            );
            cut_step.done();
            let verify_step = crate::ui::Step::new("Verify cut video hash and packet count");
            let copied_hash = video_sha256(cfg, &path, 0)?;
            crate::ui::detail(&format!(
                "Cut video SHA-256: {} ({} packets)",
                copied_hash.0, copied_hash.1
            ));
            ensure!(
                source_hash == copied_hash,
                "Cut video differs from the original H.264 picture data or packet count"
            );
            verify_step.done();
            cut_video_path = Some(path);
        }
        if plan.sources.len() > 1 {
            let join_step = crate::ui::Step::new("Join source clips");
            let mut join = vec!["-o".into(), plan.source.to_string_lossy().into_owned()];
            join.push(plan.sources[0].to_string_lossy().into_owned());
            for source in plan.sources.iter().skip(1) {
                join.push(format!("+{}", source.display()));
            }
            runner.command(&cfg.tools.mkvmerge, join, true)?;
            ensure!(
                plan.source.is_file(),
                "mkvmerge did not create the joined source"
            );
            join_step.done();
        }
        let dir = plan.output.parent().unwrap();
        std::fs::create_dir_all(dir)?;
        // Keep the mux hidden in the destination filesystem until it is complete.
        let partial = dir.join(format!(
            ".{}",
            plan.output
                .file_name()
                .context("Output has no filename")?
                .to_string_lossy()
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
            crate::ui::detail(&format!("Chapters: {}", chapter.source.display()));
            if let Some(timing) = chapter.timing {
                crate::ui::detail(&format!(
                    "Chapter cutoff: {} frames ({}); playlist {} ms, video {} ms ({})",
                    timing.cutoff_frames,
                    timing.cutoff_source,
                    timing.playlist_duration_ms,
                    timing
                        .video_duration_ms
                        .map_or_else(|| "unknown".into(), |value| value.to_string()),
                    timing.video_duration_source.as_deref().unwrap_or("unknown")
                ));
                if let Some(difference) = timing.difference_ms {
                    crate::ui::detail(&format!("Video minus playlist duration: {difference} ms"));
                }
            }
        }
        if let (Some(t), Some(client)) = (&cfg.tmdb, client) {
            let tags_step = crate::ui::Step::new("Prepare TMDB tags");
            let path = plan.work.join("global-tags.xml");
            std::fs::write(&path, client.tags(t, ep)?)?;
            args.extend(["--global-tags".into(), path.to_string_lossy().into()]);
            crate::ui::detail(&format!("TMDB tags: {}", path.display()));
            tags_step.done();
        }
        for arg in &cfg.mux_args {
            args.push(expand(arg, &plan.vars)?);
        }
        let index = font_index(cfg, base, plan, cache)?;
        let mut used = BTreeSet::new();
        let mut order = Vec::new();
        for (file_index, tp) in plan.tracks.iter().enumerate() {
            let track_step = crate::ui::Step::new(format!("Prepare {} ({})", tp.id, tp.category));
            crate::ui::detail(&format!(
                "Source: {} (track {}; {} / {})",
                tp.source.display(),
                tp.track,
                tp.name,
                tp.language
            ));
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
                    &runner,
                    &t,
                    &tp.source,
                    &plan.work,
                    &stream,
                    selected_cut,
                    range.as_ref(),
                )?;
                if let Some(pcm) = &prepared.source_pcm {
                    let verify_audio =
                        crate::ui::Step::new(format!("Verify cut audio PCM: {}", tp.id));
                    let codec = pcm_codec(&stream);
                    let original = prepared
                        .source_pcm_hash
                        .clone()
                        .context("Missing source PCM hash from cut extraction")?;
                    let extracted = pcm_sha256(cfg, pcm, 0, codec)?;
                    crate::ui::detail(&format!(
                        "{} cut PCM SHA-256: {} ({} bytes)",
                        tp.id, extracted.0, extracted.1
                    ));
                    let matches_source = original == extracted;
                    if let Some(range) = &prepared.source_sample_range {
                        crate::ui::detail(&format!(
                            "{} samples: {}..{} requested, decoded through {} at {} Hz (first source PTS {:.6} s)",
                            tp.id,
                            range.start_sample,
                            range.end_sample,
                            range.decoded_end_sample,
                            range.sample_rate,
                            range.first_source_sample_pts_seconds
                        ));
                    }
                    ensure!(
                        matches_source,
                        "Cut audio {} differs from corresponding source PCM samples (source {} bytes, cut {} bytes)",
                        tp.id,
                        original.1,
                        extracted.1
                    );
                    verify_audio.done();
                }
                for (key, value) in prepared.tags {
                    t.tags.entry(key).or_insert(value);
                }
                let id = track_id(&identify(&cfg.tools.mkvmerge, &prepared.path)?, "audio", 0)?;
                (prepared.path, id)
            } else if is_pgs(cfg, &tp.id) {
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
                    runner.command(&cfg.tools.ffmpeg, extract_args, false)?;
                    if let Some(cut) = selected_cut {
                        let verify_pgs = crate::ui::Step::new(format!("Verify cut PGS: {}", tp.id));
                        let first_source =
                            first_packet_ms(&cfg.tools.ffprobe, &tp.source, "s", subtitle_index)?;
                        let full_bytes = std::fs::read(full.as_ref().unwrap())?;
                        let source_first_pts = (first_source * 90.0).round() as i64;
                        let start_pts = (cut.start_seconds * 90000.0).round() as i64;
                        let end_pts = (cut.end_seconds * 90000.0).round() as i64;
                        let cropped = pgs::cut(&full_bytes, source_first_pts, start_pts, end_pts)?;
                        std::fs::write(&extracted, cropped)?;
                        let integrity = pgs::verify_cut(
                            &full_bytes,
                            &std::fs::read(&extracted)?,
                            source_first_pts,
                            start_pts,
                            end_pts,
                        )?;
                        crate::ui::detail(&format!(
                            "{} source PGS SHA-256: {} ({} segments)",
                            tp.id, integrity.source_sha256, integrity.segments
                        ));
                        crate::ui::detail(&format!(
                            "{} cut PGS SHA-256: {} ({} segments; timestamps verified)",
                            tp.id, integrity.cut_sha256, integrity.segments
                        ));
                        verify_pgs.done();
                    }
                    let id = pgs_track(&identify(&cfg.tools.mkvmerge, &extracted)?, 0)?.0;
                    (extracted, id)
                }
            } else if tp.category == "subtitle" {
                let (text, doc) = load_sub(cfg, &t, &tp.source, &ep.id, &plan.tracks[0], cutoff)?;
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
            track_step.done();
        }
        if cfg.fonts.enabled {
            let fonts_step = crate::ui::Step::new("Collect fonts");
            let fonts = index.collect(&used, &plan.work.join("fonts"))?;
            crate::ui::detail(&format!("Collected {} font files", fonts.len()));
            for font in &fonts {
                crate::ui::detail(&format!(
                    "Font: {} <- {} (SHA-256 {})",
                    font.path.display(),
                    font.source.display(),
                    font.sha256
                ));
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
            fonts_step.done();
        }
        args.extend(["--track-order".into(), order.join(",")]);
        let mux_step = crate::ui::Step::new("Mux output");
        runner.command(&cfg.tools.mkvmerge, args, true)?;
        ensure!(partial.is_file(), "mkvmerge did not produce an output file");
        mux_step.done();
        let publish_step = crate::ui::Step::new("Publish output");
        std::fs::hard_link(&partial, &plan.output).with_context(|| {
            format!(
                "Publishing {} without overwriting; temporary file kept at {}",
                plan.output.display(),
                partial.display()
            )
        })?;
        if let Err(error) = std::fs::remove_file(&partial) {
            crate::ui::warning(&format!(
                "Published MKV, but could not remove temporary name {}: {error}",
                partial.display()
            ));
        }
        publish_step.done();
        Ok(())
    })();
    if result.is_ok() && cfg.project.clean_work_dirs {
        std::fs::remove_dir_all(&plan.work)?;
    }
    result
}
pub fn run(cfg: &Config, base: &Path, plans: &[EpisodePlan]) -> Result<()> {
    let batch_started = std::time::Instant::now();
    crate::ui::batch("Run", plans.len());
    for plan in plans {
        ensure!(
            !plan.output.exists(),
            "Output already exists: {}",
            plan.output.display()
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
    let mut identifiers = BTreeMap::new();
    if cfg.episode_configs.is_empty() {
        check_cached(cfg, base, plans, &mut cache, &mut identifiers)?;
    } else {
        for plan in plans {
            check_cached(
                cfg.for_episode(&plan.episode),
                base,
                std::slice::from_ref(plan),
                &mut cache,
                &mut identifiers,
            )?;
        }
    }
    let mut errors = Vec::new();
    for (index, plan) in plans.iter().enumerate() {
        let episode_started = std::time::Instant::now();
        let episode_cfg = cfg.for_episode(&plan.episode);
        let ep = episode_cfg
            .episodes
            .iter()
            .find(|e| e.id == plan.episode)
            .or_else(|| cfg.episodes.iter().find(|e| e.id == plan.episode))
            .unwrap();
        crate::ui::episode(&ep.id, index + 1, plans.len(), &plan.title);
        let mut client = episode_cfg
            .tmdb
            .as_ref()
            .map(tmdb::Client::new)
            .transpose()?;
        if let Err(e) = run_episode(episode_cfg, base, ep, plan, &mut client, &mut cache) {
            errors.push(ep.id.clone());
            if !episode_cfg.project.continue_on_error {
                return Err(e).with_context(|| format!("Episode {} failed", ep.id));
            }
            crate::ui::error(&format!("Episode {} failed: {e:#}", ep.id));
        } else {
            crate::ui::complete(&ep.id, &plan.output, episode_started.elapsed());
        }
    }
    ensure!(
        errors.is_empty(),
        "Batch finished with failures in episodes: {}",
        errors.join(", ")
    );
    crate::ui::batch_complete(plans.len(), batch_started.elapsed());
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
