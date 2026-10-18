use muxcraft::{
    config::{Correction, Track},
    fonts::{FontIndex, Request},
    subtitle::{self, Fps},
};
use std::collections::BTreeSet;
use std::{path::Path, process::Command};
fn available(cmd: &str) -> bool {
    Command::new(cmd).arg("-version").output().is_ok()
}
fn cmd(program: &str, args: &[&str]) {
    let out = Command::new(program).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn logged_hash<'a>(log: &'a str, label: &str) -> &'a str {
    let line = log
        .lines()
        .find(|line| line.trim_start().starts_with(label))
        .unwrap_or_else(|| panic!("Missing log entry: {label}"));
    line.trim_start()
        .strip_prefix(label)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
}
fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
// Tiny complete SUP display set: a 2x2 white bitmap, then a clear composition.
fn pgs_fixture() -> Vec<u8> {
    let mut data = Vec::new();
    let mut segment = |pts: u32, kind: u8, body: &[u8]| {
        data.extend(b"PG");
        data.extend(pts.to_be_bytes());
        data.extend(0u32.to_be_bytes());
        data.push(kind);
        data.extend((body.len() as u16).to_be_bytes());
        data.extend(body);
    };
    segment(
        45000,
        0x16,
        &[
            0, 128, 0, 72, 0x10, 0, 0, 0x80, 0, 0, 1, 0, 0, 0, 0, 0, 10, 0, 10,
        ],
    );
    segment(45000, 0x17, &[1, 0, 0, 10, 0, 10, 0, 2, 0, 2]);
    segment(45000, 0x14, &[0, 0, 1, 235, 128, 128, 255]);
    segment(
        45000,
        0x15,
        &[0, 0, 0, 0xc0, 0, 0, 12, 0, 2, 0, 2, 1, 1, 0, 0, 1, 1, 0, 0],
    );
    segment(45000, 0x80, &[]);
    segment(135000, 0x16, &[0, 128, 0, 72, 0x10, 0, 1, 0, 0, 0, 0]);
    segment(135000, 0x80, &[]);
    data
}

#[test]
fn mux_logs_commands_without_writing_audit_files() {
    if !available("ffmpeg") || !available("ffprobe") || !available("mkvmerge") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mkv");
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:r=24",
            "-t",
            "1",
            "-c:v",
            "libx264",
            source.to_str().unwrap(),
        ],
    );
    let config = dir.path().join("batch.toml");
    std::fs::write(
        &config,
        "[project]\nshow='NoAudit'\nout_dir='out'\nwork_dir='work'\nfps='24'\n[subtitles.fonts]\nenabled=false\n[episodes]\n01='source.mkv'\n[VideoTrack.01]\nsource='{episodes}'\n[mux]\n'VideoTrack.01'={name='Video',language='ja'}\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.path().join("out/NoAudit.01.mkv").is_file());
    assert!(!dir.path().join("out/NoAudit.01.audit.json").exists());
    assert!(!dir.path().join("work/01/audit.json").exists());
    assert!(!dir.path().join("out/.NoAudit.01.mkv").exists());
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(log.contains("$ mkvmerge --gui-mode -o"), "{log}");
    assert!(log.contains(".NoAudit.01.mkv"), "{log}");
}

#[test]
fn webvtt_is_muxed_as_a_webvtt_track() {
    if !available("ffmpeg") || !available("ffprobe") || !available("mkvmerge") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mkv");
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:r=24",
            "-t",
            "1",
            "-c:v",
            "libx264",
            source.to_str().unwrap(),
        ],
    );
    std::fs::write(
        dir.path().join("captions.vtt"),
        "WEBVTT\n\nSTYLE\n::cue { color: yellow; }\n\nline-a\n00:00:00.100 --> 00:00:00.900 align:center\nHello <b>world</b>\n",
    )
    .unwrap();
    let config = dir.path().join("batch.toml");
    std::fs::write(
        &config,
        "[project]\nshow='Vtt'\nout_dir='out'\nwork_dir='work'\nfps='24'\n[subtitles.fonts]\nenabled=false\n[episodes]\n01='source.mkv'\n[VideoTrack.01]\nsource='{episodes}'\n[subtitles]\nUS='captions.vtt'\n[mux]\n'VideoTrack.01'={name='Video',language='ja'}\nUS={name='English',language='en-US'}\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mkv = dir.path().join("out/Vtt.01.mkv");
    let output = Command::new("mkvmerge")
        .arg("-J")
        .arg(&mkv)
        .output()
        .unwrap();
    assert!(output.status.success());
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tracks = info["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[1]["properties"]["codec_id"], "S_TEXT/WEBVTT");
}

#[test]
fn reference_webvtt_survives_mkvtoolnix_roundtrip() {
    if !available("mkvmerge") || !available("mkvextract") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let track: Track = toml::from_str("id='US'\nsource='captions.vtt'\nshift_frames=24").unwrap();
    for name in ["settings-multiple", "comment-in-cue-text", "w3c-features"] {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/webvtt")
            .join(format!("{name}.vtt"));
        let input = std::fs::read_to_string(&fixture).unwrap();
        let shifted = subtitle::process_vtt(
            &input,
            &track,
            &Correction::default(),
            "01",
            Fps::parse("24").unwrap(),
            None,
        )
        .unwrap();
        let processed = dir.path().join(format!("{name}.vtt"));
        let mkv = dir.path().join(format!("{name}.mkv"));
        let extracted = dir.path().join(format!("{name}.extracted.vtt"));
        std::fs::write(&processed, &shifted).unwrap();
        cmd(
            "mkvmerge",
            &["-o", mkv.to_str().unwrap(), processed.to_str().unwrap()],
        );
        cmd(
            "mkvextract",
            &[
                "tracks",
                mkv.to_str().unwrap(),
                &format!("0:{}", extracted.display()),
            ],
        );
        let result = std::fs::read_to_string(extracted).unwrap();
        assert!(
            result.trim_start_matches('\u{feff}').starts_with("WEBVTT"),
            "{name}: {result}"
        );
        match name {
            "settings-multiple" => {
                assert!(result.contains("00:00:00.980"));
                assert!(result.contains("id0"));
                assert!(result.contains("align:start line:1% vertical:lr size:50% position:25%"));
            }
            "comment-in-cue-text" => {
                assert!(result.contains("00:00:00.980"));
                assert!(result.contains("NOTE text\nNOTE text2"));
            }
            "w3c-features" => {
                assert!(result.contains("STYLE\n::cue(.speaker) { color: lime; }"));
                assert!(result.contains("REGION\nid:bottom"));
                assert!(
                    result.contains("<00:00:02.980><lang ja><ruby>旅<rt>たび</rt></ruby></lang>")
                );
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn copy_and_flac_keep_source_audio_start_offset() {
    if !available("ffmpeg") || !available("ffprobe") || !available("flac") || !available("mkvmerge")
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let source = base.join("source.mkv");
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:r=24",
            "-itsoffset",
            "0.5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-t",
            "3",
            "-c:v",
            "libx264",
            "-c:a",
            "pcm_s16le",
            source.to_str().unwrap(),
        ],
    );
    let config = "[project]\nshow='Sync'\nout_dir='out'\nwork_dir='work'\nfps='24'\n[subtitles.fonts]\nenabled=false\n[episodes]\n01='source.mkv'\n[VideoTrack.01]\nsource='{episodes}'\n[AudioTrack.01]\nsource='{episodes}'\ncodec='copy'\n[AudioTrack.02]\nsource='{episodes}'\ncodec='flac'\n[mux]\n'VideoTrack.01'={name='Video',language='ja'}\n'AudioTrack.01'={name='Copy',language='ja'}\n'AudioTrack.02'={name='FLAC',language='ja',delay_ms=100}\n";
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let log = String::from_utf8_lossy(&run.stderr);
    assert!(log.contains("Copy audio directly from source"), "{log}");
    assert!(!log.contains("AudioTrack.01.extracted"), "{log}");
    assert!(log.contains("applied sync: +600 ms"), "{log}");
    let output = base.join("out/Sync.01.mkv");
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,start_time",
            "-of",
            "json",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(probe.status.success());
    let info: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    let starts = info["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stream| {
            stream["start_time"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 3);
    assert!((starts[1] - 0.5).abs() < 0.05, "{starts:?}");
    assert!((starts[2] - 0.6).abs() < 0.05, "{starts:?}");
}

#[test]
fn flac_keeps_32_bit_integer_pcm() {
    if !available("ffmpeg") || !available("ffprobe") || !available("flac") || !available("mkvmerge")
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let source = base.join("source.mkv");
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:r=24",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=sin(2*PI*440*t):s=48000",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-c:a",
            "pcm_s32le",
            source.to_str().unwrap(),
        ],
    );
    let config = "[project]\nshow='Depth'\nout_dir='out'\nwork_dir='work'\nfps='24'\n[subtitles.fonts]\nenabled=false\n[episodes]\n01='source.mkv'\n[VideoTrack.01]\nsource='{episodes}'\n[AudioTrack.01]\nsource='{episodes}'\ncodec='flac'\n[mux]\n'VideoTrack.01'={name='Video',language='ja'}\n'AudioTrack.01'={name='FLAC',language='ja'}\n";
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let output = base.join("out/Depth.01.mkv");
    let hash = |path: &Path| {
        let result = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:a:0",
                "-c:a",
                "pcm_s32le",
                "-f",
                "hash",
                "-hash",
                "sha256",
                "-",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        result.stdout
    };
    assert_eq!(hash(&source), hash(&output));
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=bits_per_raw_sample",
            "-of",
            "json",
        ])
        .arg(&output)
        .output()
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert_eq!(info["streams"][0]["bits_per_raw_sample"], "32");
}

#[test]
fn tmdb_failure_stops_before_creating_work_files() {
    if !available("ffmpeg") || !available("ffprobe") || !available("mkvmerge") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let source = base.join("source.mkv");
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:r=24",
            "-t",
            "1",
            "-c:v",
            "libx264",
            source.to_str().unwrap(),
        ],
    );
    let config = "[project]\nshow='Metadata'\nout_dir='out'\nwork_dir='work'\nfps='24'\n[subtitles.fonts]\nenabled=false\n[episodes]\n01='source.mkv'\n[VideoTrack.01]\nsource='{episodes}'\n[tmdb]\nid=1\noffset=-1\n[mux]\n'VideoTrack.01'={name='Video',language='ja'}\n";
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("Invalid TMDB number"));
    assert!(!base.join("work/01").exists());
    assert!(!base.join("out/Metadata.01.mkv").exists());
}

#[cfg(unix)]
#[test]
fn failed_mux_and_invalid_output_remove_hidden_partial() {
    use std::os::unix::fs::PermissionsExt;
    if !available("ffmpeg") || !available("ffprobe") || !available("mkvmerge") {
        return;
    }
    for behavior in ["fail", "corrupt"] {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let source = base.join("source.mkv");
        cmd(
            "ffmpeg",
            &[
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:r=24",
                "-t",
                "1",
                "-c:v",
                "libx264",
                source.to_str().unwrap(),
            ],
        );
        let wrapper = base.join("mkvmerge-wrapper.sh");
        std::fs::write(
            &wrapper,
            "#!/bin/sh\nif [ \"$1\" = '--version' ] || [ \"$1\" = '-J' ]; then exec mkvmerge \"$@\"; fi\nout=''\nprevious=''\nfor arg in \"$@\"; do if [ \"$previous\" = '-o' ]; then out=\"$arg\"; break; fi; previous=\"$arg\"; done\nmkvmerge \"$@\"\nstatus=$?\nif [ \"$status\" -ne 0 ]; then exit \"$status\"; fi\nif [ \"$MUXCRAFT_TEST_MUX_BEHAVIOR\" = 'corrupt' ]; then : > \"$out\"; exit 0; fi\nexit 2\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&wrapper).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&wrapper, permissions).unwrap();
        let config = format!(
            "[project]\nshow='Publish'\nout_dir='out'\nwork_dir='work'\nfps='24'\n[tools]\nmkvmerge='{}'\n[subtitles.fonts]\nenabled=false\n[episodes]\n01='source.mkv'\n[VideoTrack.01]\nsource='{{episodes}}'\n[mux]\n'VideoTrack.01'={{name='Video',language='ja'}}\n",
            wrapper.display()
        );
        let config_path = base.join("batch.toml");
        std::fs::write(&config_path, config).unwrap();
        let run = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
            .arg(&config_path)
            .env("MUXCRAFT_TEST_MUX_BEHAVIOR", behavior)
            .output()
            .unwrap();
        assert!(!run.status.success(), "{behavior}");
        assert!(!base.join("out/.Publish.01.mkv").exists(), "{behavior}");
        assert!(!base.join("out/Publish.01.mkv").exists(), "{behavior}");
        assert!(base.join("work/01").exists(), "{behavior}");
    }
}

#[test]
fn cuts_shared_m2ts_and_verifies_extracted_tracks() {
    if !available("ffmpeg") || !available("ffprobe") || !available("flac") || !available("mkvmerge")
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let bdmv = base.join("BDMV");
    std::fs::create_dir_all(bdmv.join("STREAM")).unwrap();
    std::fs::create_dir_all(bdmv.join("PLAYLIST")).unwrap();
    let source = bdmv.join("STREAM/00001.m2ts");
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=128x72:r=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=660:sample_rate=48000",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-map",
            "2:a:0",
            "-t",
            "40",
            "-c:v",
            "libx264",
            "-g",
            "48",
            "-keyint_min",
            "48",
            "-sc_threshold",
            "0",
            "-bf",
            "3",
            "-c:a",
            "ac3",
            "-c:a:1",
            "ac3",
            "-output_ts_offset",
            "4219",
            "-f",
            "mpegts",
            source.to_str().unwrap(),
        ],
    );
    let packet_output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_packets",
            "-show_entries",
            "packet=pts_time,flags",
            "-of",
            "json",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(packet_output.status.success());
    let packets: serde_json::Value = serde_json::from_slice(&packet_output.stdout).unwrap();
    let timestamps: Vec<f64> = packets["packets"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["pts_time"].as_str()?.parse().ok())
        .collect();
    let keyframes: Vec<f64> = packets["packets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["flags"].as_str().is_some_and(|f| f.contains('K')))
        .filter_map(|p| p["pts_time"].as_str()?.parse().ok())
        .collect();
    assert!(keyframes.len() >= 17);
    let first = (keyframes[0] * 45000.0).round() as u32;
    let second = (keyframes[16] * 45000.0).round() as u32;
    let end = ((timestamps.iter().copied().reduce(f64::max).unwrap() + 1.0 / 24.0) * 45000.0)
        .round() as u32;
    let mut mpls = vec![0_u8; 128];
    mpls[..4].copy_from_slice(b"MPLS");
    put32(&mut mpls, 8, 32);
    put32(&mut mpls, 12, 80);
    put16(&mut mpls, 38, 1);
    put16(&mut mpls, 42, 20);
    mpls[44..49].copy_from_slice(b"00001");
    mpls[49..53].copy_from_slice(b"M2TS");
    put32(&mut mpls, 56, first);
    put32(&mut mpls, 60, end);
    put16(&mut mpls, 84, 2);
    for (i, tick) in [first, second].iter().enumerate() {
        let p = 86 + i * 14;
        mpls[p + 1] = 1;
        put32(&mut mpls, p + 4, *tick);
    }
    std::fs::write(bdmv.join("PLAYLIST/00001.mpls"), mpls).unwrap();
    let config = "[project]\nshow='Split'\nout_dir='out'\nwork_dir=\"work [one|two]'s\"\nclean_work_dirs=false\nfps='24'\n[subtitles.fonts]\nenabled=false\n[episodes]\n01={source='BDMV/STREAM/00001.m2ts',start_chapter=1,end_chapter=2}\n02={source='BDMV/STREAM/00001.m2ts',start_chapter=2}\n[chapters]\n01=[]\n02=[]\n[VideoTrack.01]\nsource='{episodes}'\n[AudioTrack.01]\nsource='{episodes}'\ncodec='flac'\n[AudioTrack.02]\nsource='{episodes}'\ntrack=1\ncodec='flac'\n[mux]\n'VideoTrack.01'={name='Video',language='ja'}\n'AudioTrack.01'={name='Audio',language='ja'}\n'AudioTrack.02'={name='Audio 2',language='ja'}\n";
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("muxcraft: Run (2 episodes)"), "{stderr}");
    assert!(stderr.contains("Episode 01 (1/2)"), "{stderr}");
    assert!(
        stderr.contains("Starting: Cut video and hash source"),
        "{stderr}"
    );
    assert!(
        stderr.contains("Done: Verify cut video hash and packet count"),
        "{stderr}"
    );
    assert!(stderr.contains("Batch complete"), "{stderr}");
    assert!(
        !stderr.contains("skipping unknown chunk 'JUNK'"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("skipping unknown chunk 'LIST'"),
        "{stderr}"
    );
    for episode in ["01", "02"] {
        let final_path = base.join(format!("out/Split.{episode}.mkv"));
        assert!(final_path.is_file());
        assert!(
            base.join(format!("work [one|two]'s/{episode}/00001.mkv"))
                .is_file()
        );
        let chapter_probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "chapter=start_time",
                "-of",
                "json",
            ])
            .arg(&final_path)
            .output()
            .unwrap();
        assert!(chapter_probe.status.success());
        let chapter_json: serde_json::Value =
            serde_json::from_slice(&chapter_probe.stdout).unwrap();
        let chapter_entries = chapter_json["chapters"].as_array().unwrap();
        assert_eq!(chapter_entries.len(), 1);
        assert_eq!(chapter_entries[0]["start_time"], "0.000000");
        assert!(!final_path.with_extension("audit.json").exists());
        let episode_log = stderr
            .split(&format!("Episode {episode} ("))
            .nth(1)
            .unwrap();
        if episode == "01" {
            assert!(episode_log.contains("768 video frames"));
        }
        assert_eq!(
            logged_hash(episode_log, "Source video SHA-256: "),
            logged_hash(episode_log, "Cut video SHA-256: ")
        );
        for track in ["AudioTrack.01", "AudioTrack.02"] {
            assert_eq!(
                logged_hash(episode_log, &format!("{track} source PCM SHA-256: ")),
                logged_hash(episode_log, &format!("{track} cut PCM SHA-256: "))
            );
        }
        assert!(episode_log.contains("-rf64 always"));
        assert!(episode_log.contains("-bitexact"));
        assert!(episode_log.contains("-f matroska"));
        assert!(episode_log.contains("-f hash -hash SHA256"));
        assert!(episode_log.contains("at 48000 Hz"));
    }
    let raw = |path: &Path, interval: Option<(f64, f64)>| {
        let mut command = Command::new("ffmpeg");
        command.args(["-v", "error"]);
        command.arg("-i").arg(path);
        if let Some((start, end)) = interval {
            command.args([
                "-af",
                &format!("atrim=start={start:.9}:end={end:.9},asetpts=PTS-STARTPTS"),
            ]);
        }
        command.args(["-map", "0:a:0", "-c:a", "pcm_s24le", "-f", "s24le", "-"]);
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        result.stdout
    };
    let source_start = {
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=start_time",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse::<f64>()
            .unwrap()
    };
    assert!(source_start > 4200.0);
    let whole = raw(
        &source,
        Some((
            keyframes[0] - source_start,
            end as f64 / 45000.0 - source_start,
        )),
    );
    let mut joined = raw(&base.join("out/Split.01.mkv"), None);
    joined.extend(raw(&base.join("out/Split.02.mkv"), None));
    let mismatch = joined.iter().zip(&whole).position(|(a, b)| a != b);
    assert!(
        joined == whole,
        "Adjacent PCM mismatch: joined={} bytes, source={} bytes, first mismatch at {:?}",
        joined.len(),
        whole.len(),
        mismatch
    );
    let video_packets = |path: &Path| -> u64 {
        let result = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-count_packets",
                "-show_entries",
                "stream=nb_read_packets",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(result.status.success());
        String::from_utf8(result.stdout)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    assert_eq!(
        video_packets(&source),
        video_packets(&base.join("out/Split.01.mkv"))
            + video_packets(&base.join("out/Split.02.mkv")),
        "Adjacent cuts must preserve every video packet exactly once"
    );
}
#[test]
fn joins_m2ts_clips_before_extracting_audio() {
    if !available("ffmpeg") || !available("flac") || !available("mkvmerge") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let stream = base.join("BDMV/STREAM");
    let playlist = base.join("BDMV/PLAYLIST");
    std::fs::create_dir_all(&stream).unwrap();
    std::fs::create_dir_all(&playlist).unwrap();
    for (name, color, tone) in [("00001", "black", "440"), ("00002", "white", "880")] {
        let path = stream.join(format!("{name}.m2ts"));
        cmd(
            "ffmpeg",
            &[
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c={color}:s=128x72:r=24000/1001"),
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency={tone}:sample_rate=48000"),
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-c:a",
                "ac3",
                "-f",
                "mpegts",
                path.to_str().unwrap(),
            ],
        );
    }
    let marks_offset = 32 + 10 + 2 * 22;
    let mut mpls = vec![0_u8; marks_offset + 6 + 2 * 14];
    mpls[..4].copy_from_slice(b"MPLS");
    put32(&mut mpls, 8, 32);
    put32(&mut mpls, 12, marks_offset as u32);
    put16(&mut mpls, 38, 2);
    for (i, clip) in ["00001", "00002"].into_iter().enumerate() {
        let pos = 42 + i * 22;
        put16(&mut mpls, pos, 20);
        mpls[pos + 2..pos + 7].copy_from_slice(clip.as_bytes());
        mpls[pos + 7..pos + 11].copy_from_slice(b"M2TS");
        put32(&mut mpls, pos + 18, 45000);
        let mark = marks_offset + 6 + i * 14;
        mpls[mark + 1] = 1;
        put16(&mut mpls, mark + 2, i as u16);
    }
    put16(&mut mpls, marks_offset + 4, 2);
    std::fs::write(playlist.join("00001.mpls"), mpls).unwrap();
    let config = "[project]\nshow='Joined'\nout_dir='out'\nwork_dir='work'\nclean_work_dirs=false\n[subtitles.fonts]\nenabled=false\n[episodes]\n01=['BDMV/STREAM/00001.m2ts','BDMV/STREAM/00002.m2ts']\n[VideoTrack.01]\nsource='{episodes}'\n[AudioTrack.01]\nsource='{episodes}'\ncodec='flac'\n[mux]\n'VideoTrack.01'={name='Video',language='ja-JP'}\n'AudioTrack.01'={name='Audio',language='ja-JP'}\n";
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
    let mut run_log = String::new();
    for mode in ["check", "run"] {
        let out = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
            .arg(&config_path)
            .args(["--mode", mode])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if mode == "run" {
            run_log = String::from_utf8(out.stderr).unwrap();
        }
    }
    let output = base.join("out/Joined.01.mkv");
    assert!(output.is_file());
    assert!(base.join("work/01/00001.joined.mkv").is_file());
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success());
    let duration: f64 = String::from_utf8(out.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        (1.9..2.2).contains(&duration),
        "joined duration: {duration}"
    );
    let chapter_probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "chapter=start_time",
            "-of",
            "json",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(chapter_probe.status.success());
    let chapter_json: serde_json::Value = serde_json::from_slice(&chapter_probe.stdout).unwrap();
    let chapter_entries = chapter_json["chapters"].as_array().unwrap();
    assert_eq!(chapter_entries.len(), 2);
    assert_eq!(chapter_entries[0]["start_time"], "0.000000");
    assert_eq!(chapter_entries[1]["start_time"], "1.000000");
    assert!(run_log.contains("Starting: Join source clips"));
    assert!(run_log.contains("Auto chapters: 2 marks from"));
    assert!(run_log.contains("$ mkvmerge --gui-mode -o"));
    assert!(!output.with_extension("audit.json").exists());
}
#[test]
fn bdmv_chapters_use_video_duration_for_terminal_mark_and_cli() {
    if !available("ffmpeg") || !available("ffprobe") || !available("mkvmerge") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let stream = base.join("BDMV/STREAM");
    let playlist = base.join("BDMV/PLAYLIST");
    std::fs::create_dir_all(&stream).unwrap();
    std::fs::create_dir_all(&playlist).unwrap();
    let clip = stream.join("00001.m2ts");
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=128x72:r=24",
            "-frames:v",
            "264",
            "-c:v",
            "libx264",
            "-an",
            "-f",
            "mpegts",
            clip.to_str().unwrap(),
        ],
    );
    let mut mpls = vec![0u8; 160];
    mpls[..4].copy_from_slice(b"MPLS");
    mpls[8..12].copy_from_slice(&32u32.to_be_bytes());
    mpls[12..16].copy_from_slice(&80u32.to_be_bytes());
    mpls[38..40].copy_from_slice(&1u16.to_be_bytes());
    mpls[42..44].copy_from_slice(&20u16.to_be_bytes());
    mpls[44..49].copy_from_slice(b"00001");
    mpls[49..53].copy_from_slice(b"M2TS");
    mpls[56..60].copy_from_slice(&(45000u32 * 600).to_be_bytes());
    mpls[60..64].copy_from_slice(&(45000u32 * 610).to_be_bytes());
    mpls[84..86].copy_from_slice(&3u16.to_be_bytes());
    for (index, second) in [600u32, 604, 608].into_iter().enumerate() {
        let position = 86 + index * 14;
        mpls[position + 1] = 1;
        mpls[position + 4..position + 8].copy_from_slice(&(45000 * second).to_be_bytes());
    }
    std::fs::write(playlist.join("00001.mpls"), mpls).unwrap();
    let config = "[project]\nshow='ChapterTest'\nout_dir='out'\nwork_dir='work'\nclean_work_dirs=false\nfps='24'\n[subtitles.fonts]\nenabled=false\n[episodes]\n'01'='BDMV/STREAM/00001.m2ts'\n[chapters]\n'01'=['First','Middle','Tail']\n[VideoTrack.01]\nsource='{episodes}'\n[mux]\n'VideoTrack.01'={name='Video',language='ja-JP'}\n";
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
    let mut run_log = String::new();
    for mode in ["check", "run"] {
        let output = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
            .arg(&config_path)
            .args(["--mode", mode])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if mode == "run" {
            run_log = String::from_utf8(output.stderr).unwrap();
        }
    }
    let chapters = std::fs::read_to_string(base.join("work/01/chapters.txt")).unwrap();
    assert!(chapters.contains("CHAPTER03=00:00:08.000"));
    assert!(run_log.contains("Chapter cutoff: 264 frames"));
    assert!(run_log.contains("playlist 10000 ms"));
    assert!(!base.join("out/ChapterTest.01.audit.json").exists());
}
#[test]
fn real_batch_flac_copy_subtitles_chapters_flags_and_cli() {
    if !available("ffmpeg") || !available("flac") || !available("mkvmerge") {
        eprintln!("Media test requires ffmpeg, flac, and mkvmerge");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let source = base.join("source.mkv");
    let truehd = base.join("truehd.mka");
    let pgs = base.join("subtitle.sup");
    let pgs_container = base.join("pgs.mks");
    let middle_subtitle = base.join("middle.srt");
    std::fs::write(&pgs, pgs_fixture()).unwrap();
    std::fs::write(
        &middle_subtitle,
        "1\n00:00:00,500 --> 00:00:01,500\nMiddle\n",
    )
    .unwrap();
    cmd(
        "mkvmerge",
        &[
            "-o",
            pgs_container.to_str().unwrap(),
            pgs.to_str().unwrap(),
            middle_subtitle.to_str().unwrap(),
            pgs.to_str().unwrap(),
        ],
    );
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "2",
            "-c:a",
            "truehd",
            "-strict",
            "-2",
            truehd.to_str().unwrap(),
        ],
    );
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=128x72:r=24000/1001",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-c:a",
            "flac",
            source.to_str().unwrap(),
        ],
    );
    std::fs::write(
        base.join("01.srt"),
        "1\n00:00:00,500 --> 00:00:01,500\n<i>Hello</i>\n",
    )
    .unwrap();
    std::fs::write(
        base.join("02.srt"),
        "1\n00:00:00,500 --> 00:00:01,500\nEpisode 2\n",
    )
    .unwrap();
    std::fs::write(
        base.join("chapters.txt"),
        "CHAPTER01=00:00:00.000\nCHAPTER01NAME=Start\nCHAPTER02=00:00:01.000\nCHAPTER02NAME=Middle\n",
    )
    .unwrap();
    let config = r#"
[project]
show='Test'
out_dir='out'
work_dir='work'
output='{show}.{ep}.mkv'
title='{show} - {ep}'
clean_work_dirs=true
[subtitles.fonts]
enabled=false
[episodes]
'01'='source.mkv'
'02'='source.mkv'
[chapters]
'01'=[{frame=0,name='Start & <one>'},{frame=24,name='End'}]
'02'='chapters.txt'
[VideoTrack.01]
source='{episodes}'
[VideoTrack.02]
source='{episodes}'
[AudioTrack.01]
source='{episodes}'
codec='flac'
[AudioTrack.02]
source='{episodes}'
codec='copy'
[AudioTrack.03]
source='truehd.mka'
codec='copy'
[AudioTrack.04]
source='truehd.mka'
codec='flac'
[subtitles]
styled='subtitle-service/{ep}.ass'
srt='{ep}.srt*'
[subtitles.processing]
clean_garbage=true
[pgs]
pgs='subtitle.sup'
pgs_alt={source='pgs.mks',track=1}
[mux]
'VideoTrack.01'={name='BD',language='ja-JP',default=true}
'VideoTrack.02'={name='Alternate',language='ja-JP'}
'AudioTrack.01'={name='FLAC',language='ja-JP',default=true}
'AudioTrack.02'={name='Copy',language='en-US'}
'AudioTrack.03'={name='TrueHD',language='ja-JP'}
'AudioTrack.04'={name='TrueHD to FLAC',language='ja-JP'}
styled={name='Styled',language='pt-BR',default=true,tags={SubtitleVersion='BluRay'}}
srt={name='SDH',language='en-US',hearing_impaired=true,forced=true}
pgs={name='PGS',language='ja-JP'}
pgs_alt={name='Second PGS',language='ja-JP'}
"#;
    let mut config = config.to_owned();
    let mut with_font = false;
    let mut font_family = "Lato".to_owned();
    std::fs::create_dir(base.join("subtitle-service")).unwrap();
    if let Some(font) = std::env::var_os("MUXCRAFT_TEST_FONT") {
        let fonts = base.join("subtitle-service/assets/release-pack");
        std::fs::create_dir_all(&fonts).unwrap();
        std::fs::copy(font, fonts.join("fixture.ttf")).unwrap();
        let index = FontIndex::scan(&[fonts], false).unwrap();
        config = config.replace("enabled=false", "enabled=true\nsearch_current_dir=false");
        font_family = index.faces[0].family.clone();
        with_font = true;
    }
    for ep in ["01", "02"] {
        std::fs::write(
            base.join(format!("subtitle-service/{ep}.ass")),
            format!("[Script Info]\nScriptType: v4.00+\n[V4+ Styles]\nFormat: Name, Fontname, Bold, Italic\nStyle: Default,{font_family},0,0\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.50,0:00:01.50,Default,,0,0,0,,Hello\n"),
        )
        .unwrap();
    }
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run_log = String::from_utf8_lossy(&out.stderr);
    for ep in ["01", "02"] {
        let output = base.join(format!("out/Test.{ep}.mkv"));
        assert!(output.exists());
        assert!(!base.join(format!("work/{ep}")).exists());
        assert!(!output.with_extension("audit.json").exists());
        let out = Command::new("mkvmerge")
            .arg("-J")
            .arg(&output)
            .output()
            .unwrap();
        let info: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let tracks = info["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 10);
        assert_eq!(tracks[0]["type"], "video");
        assert_eq!(tracks[1]["type"], "video");
        assert_eq!(tracks[2]["codec"], "FLAC");
        assert_eq!(tracks[3]["codec"], "FLAC");
        assert_eq!(tracks[4]["codec"], "TrueHD");
        assert_eq!(tracks[5]["codec"], "FLAC");
        assert_eq!(tracks[6]["codec"], "SubStationAlpha");
        assert!(tracks[7]["codec"].as_str().unwrap().contains("SubRip"));
        assert_eq!(tracks[7]["properties"]["flag_hearing_impaired"], true);
        assert_eq!(tracks[7]["properties"]["forced_track"], true);
        assert_eq!(tracks[6]["properties"]["language_ietf"], "pt-BR");
        assert!(tracks[8]["codec"].as_str().unwrap().contains("PGS"));
        assert!(tracks[9]["codec"].as_str().unwrap().contains("PGS"));
        if with_font {
            assert_eq!(info["attachments"].as_array().unwrap().len(), 1);
            assert!(run_log.contains("Collected 1 font files"));
        }
        let tags = Command::new("mkvextract")
            .arg(&output)
            .arg("tags")
            .output()
            .unwrap();
        assert!(tags.status.success());
        let tags = String::from_utf8_lossy(&tags.stdout);
        assert!(tags.contains("SubtitleVersion"));
        assert_eq!(tags.matches("<Name>ENCODER</Name>").count(), 2);
        assert_eq!(tags.matches("<Name>ENCODER_SETTINGS</Name>").count(), 2);
        assert!(tags.contains("<String>-8 --verify"));
        for track in [&tracks[2], &tracks[5]] {
            let uid = track["properties"]["uid"].as_u64().unwrap();
            assert!(tags.contains(&format!("<TrackUID>{uid}</TrackUID>")));
        }
        assert_eq!(info["chapters"][0]["num_entries"], 2);
        // Packet data must survive both copy paths bit for bit.
        for selector in ["v:0", "v:1", "a:1", "a:2", "s:2", "s:3"] {
            let hash = |p: &Path| {
                let out = Command::new("ffmpeg")
                    .args(["-v", "error", "-i"])
                    .arg(p)
                    .args([
                        "-map", selector, "-c", "copy", "-f", "hash", "-hash", "sha256", "-",
                    ])
                    .output()
                    .unwrap();
                assert!(out.status.success());
                out.stdout
            };
            let source_selector = if selector == "s:3" {
                "s:2"
            } else if selector.starts_with("a:") {
                "a:0"
            } else if selector.starts_with("s:") {
                "s:0"
            } else {
                "v:0"
            };
            let input = Command::new("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(if selector == "a:2" {
                    &truehd
                } else if selector == "s:3" {
                    &pgs_container
                } else if selector == "s:2" {
                    &pgs
                } else {
                    &source
                })
                .args([
                    "-map",
                    source_selector,
                    "-c",
                    "copy",
                    "-f",
                    "hash",
                    "-hash",
                    "sha256",
                    "-",
                ])
                .output()
                .unwrap();
            assert_eq!(hash(&output), input.stdout);
        }
    }
    assert!(run_log.contains("$ flac -8 -o"), "{run_log}");
    assert!(run_log.contains("--verify"), "{run_log}");
    assert!(run_log.contains("pcm_s24le"), "{run_log}");
    assert!(run_log.contains("-map 0:s:2 -c copy"), "{run_log}");
    assert!(run_log.contains("--no-global-tags"), "{run_log}");
    assert!(run_log.contains("--no-chapters"), "{run_log}");
    let second = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(config_path)
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("Output already exists"));
}

#[test]
fn episode_config_is_used_for_mux_and_cli() {
    if !available("ffmpeg") || !available("mkvmerge") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=128x72:r=24",
            "-t",
            "1",
            "-c:v",
            "libx264",
            base.join("source.mkv").to_str().unwrap(),
        ],
    );
    for (name, text) in [
        ("original.srt", "Original"),
        ("replacement.srt", "Replacement"),
        ("commentary.srt", "Commentary"),
    ] {
        std::fs::write(
            base.join(name),
            format!("1\n00:00:00,100 --> 00:00:00,800\n{text}\n"),
        )
        .unwrap();
    }
    std::fs::write(
        base.join("styled.ass"),
        "[Script Info]\nTitle: test\n[Aegisub Project Garbage]\nLast Style Storage: Default\n[V4+ Styles]\nFormat: Name, Fontname, Bold, Italic\nStyle: Default,Arial,0,0\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.10,0:00:00.80,Default,,0,0,0,,Styled\n",
    )
    .unwrap();
    std::fs::create_dir(base.join("episodes")).unwrap();
    std::fs::write(
        base.join("episodes/01.toml"),
        "[episodes]\n01='source.mkv'\n[subtitles]\nUS='replacement.srt'\nCR12=false\n[subtitles.processing]\nclean_garbage=false\nycbcr_matrix='PC.709'\n",
    )
    .unwrap();
    let config = r#"
[project]
show='EpisodeConfig'
out_dir='out'
work_dir='work'
clean_work_dirs=false
fps='24'
[episodes]
01='episodes/01.toml'
02='source.mkv'
[VideoTrack.01]
source='{episodes}'
[subtitles]
US='original.srt'
CR12='commentary.srt'
Styled='styled.ass'
[subtitles.fonts]
enabled=false
[subtitles.processing]
clean_garbage=true
ycbcr_matrix='TV.709'
[mux]
'VideoTrack.01'={name='Video',language='und'}
US={name='US',language='en'}
CR12={name='Commentary',language='en'}
Styled={name='Styled',language='en'}
"#;
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
    let planned = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .args(["--mode", "plan", "--no-check", "--episodes", "02"])
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        planned.status.success(),
        "{}",
        String::from_utf8_lossy(&planned.stderr)
    );
    let selected: serde_json::Value = serde_json::from_slice(&planned.stdout).unwrap();
    assert_eq!(selected.as_array().unwrap().len(), 1);
    assert_eq!(selected[0]["episode"], "02");
    let output = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inspect = |ep: &str| {
        let path = base.join(format!("out/EpisodeConfig.{ep}.mkv"));
        let result = Command::new("mkvmerge")
            .args(["-J"])
            .arg(path)
            .output()
            .unwrap();
        assert!(result.status.success());
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()
    };
    let first = inspect("01");
    let second = inspect("02");
    let names = |info: &serde_json::Value| {
        info["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                t["properties"]["track_name"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };
    assert!(names(&first).contains(&"US".to_owned()));
    assert!(!names(&first).contains(&"Commentary".to_owned()));
    assert!(names(&second).contains(&"Commentary".to_owned()));
    let first_ass = std::fs::read_to_string(base.join("work/01/Styled.ass")).unwrap();
    let second_ass = std::fs::read_to_string(base.join("work/02/Styled.ass")).unwrap();
    assert!(first_ass.contains("[Aegisub Project Garbage]"));
    assert!(!second_ass.contains("[Aegisub Project Garbage]"));
    assert!(first_ass.contains("YCbCr Matrix: PC.709"));
    assert!(second_ass.contains("YCbCr Matrix: TV.709"));
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(log.contains("replacement.srt"), "{log}");
    assert!(!base.join("out/EpisodeConfig.01.audit.json").exists());
}

#[test]
fn video_source_can_keep_audio_subtitles_attachments_and_set_colour() {
    if !available("ffmpeg") || !available("mkvmerge") {
        eprintln!("Media test requires ffmpeg and mkvmerge");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let video = base.join("video.mkv");
    let premux = base.join("premux.mkv");
    let output = base.join("output.mkv");
    std::fs::write(
        base.join("subtitle.srt"),
        "1\n00:00:00,100 --> 00:00:00,800\nHello\n",
    )
    .unwrap();
    std::fs::write(base.join("note.txt"), "attachment").unwrap();
    std::fs::write(
        base.join("chapters.txt"),
        "CHAPTER01=00:00:00.000\nCHAPTER01NAME=Source chapter\n",
    )
    .unwrap();
    std::fs::write(
        base.join("tags.xml"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Tags><Tag><Targets/><Simple><Name>SOURCE_MARKER</Name><String>source-global-tag</String></Simple></Tag></Tags>",
    )
    .unwrap();
    cmd(
        "ffmpeg",
        &[
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=128x72:r=24000/1001",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-c:a",
            "flac",
            video.to_str().unwrap(),
        ],
    );
    cmd(
        "mkvmerge",
        &[
            "-o",
            premux.to_str().unwrap(),
            "--chapters",
            base.join("chapters.txt").to_str().unwrap(),
            "--global-tags",
            base.join("tags.xml").to_str().unwrap(),
            "--attachment-mime-type",
            "text/plain",
            "--attachment-name",
            "note.txt",
            "--attach-file",
            base.join("note.txt").to_str().unwrap(),
            video.to_str().unwrap(),
            base.join("subtitle.srt").to_str().unwrap(),
        ],
    );
    let config = "[project]\nshow='Test'\nout_dir='.'\nwork_dir='work'\noutput='output.mkv'\n[subtitles.fonts]\nenabled=false\n[episodes]\n'01'='premux.mkv'\n[VideoTrack.01]\nsource='{episodes}'\naudio=true\nsubtitles=true\nattachments=true\nglobal_tags=true\nchapters=true\ncolour={range=1,primaries=1,transfer=1,matrix=1}\n[mux]\n'VideoTrack.01'={name='BD',language='ja-JP',default=true}\n";
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(&config_path)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let identified = Command::new("mkvmerge")
        .arg("-J")
        .arg(&output)
        .output()
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&identified.stdout).unwrap();
    let kinds: Vec<_> = info["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["video", "audio", "subtitles"]);
    assert_eq!(info["attachments"].as_array().unwrap().len(), 1);
    assert_eq!(info["chapters"][0]["num_entries"], 1);
    let tags = Command::new("mkvextract")
        .arg(&output)
        .arg("tags")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&tags.stdout).contains("SOURCE_MARKER"));
    assert!(!output.with_extension("audit.json").exists());
    let log = String::from_utf8_lossy(&run.stderr);
    let mux_command = log
        .lines()
        .find(|line| line.contains("$ mkvmerge --gui-mode -o"))
        .unwrap();
    assert!(!mux_command.contains("--no-global-tags"));
    assert!(!mux_command.contains("--no-chapters"));
    for flag in [
        "--colour-range",
        "--colour-primaries",
        "--colour-transfer-characteristics",
        "--colour-matrix",
    ] {
        assert!(mux_command.contains(flag));
    }
}
#[test]
fn real_font_names_selection_and_deduplication() {
    let Some(font) = std::env::var_os("MUXCRAFT_TEST_FONT") else {
        eprintln!("Font fixture: set MUXCRAFT_TEST_FONT to a local TTF file");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::copy(&font, input.join("a.ttf")).unwrap();
    std::fs::copy(&font, input.join("b.ttf")).unwrap();
    let index = FontIndex::scan(&[input], false).unwrap();
    assert_eq!(index.faces.len(), 2);
    let face = &index.faces[0];
    let req = Request {
        name: face.family.clone(),
        weight: face.weight,
        italic: face.italic,
    };
    assert!(index.select(&req).is_some());
    let result = index
        .collect(&BTreeSet::from([req]), &dir.path().join("fonts"))
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(
        std::fs::read(&result[0].path).unwrap(),
        std::fs::read(&font).unwrap()
    );
    assert!(
        !result[0]
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(' ')
    );
}
