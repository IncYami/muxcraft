use muxcraft::fonts::{FontIndex, Request};
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
fn cuts_shared_m2ts_and_verifies_final_tracks() {
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
    let config = "[project]\nshow='Split'\nout_dir='out'\nwork_dir='work'\nclean_work_dirs=false\nfps='24'\n[subtitles.fonts]\nenabled=false\n[episodes]\n01={source='BDMV/STREAM/00001.m2ts',start_chapter=1,end_chapter=2}\n02={source='BDMV/STREAM/00001.m2ts',start_chapter=2}\n[chapters]\n01=[]\n02=[]\n[VideoTrack.01]\nsource='{episodes}'\n[AudioTrack.01]\nsource='{episodes}'\ncodec='flac'\n[mux]\n'VideoTrack.01'={name='Video',language='ja'}\n'AudioTrack.01'={name='Audio',language='ja'}\n";
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
    for episode in ["01", "02"] {
        let final_path = base.join(format!("out/Split.{episode}.mkv"));
        assert!(final_path.is_file());
        assert!(base.join(format!("work/{episode}/00001.mkv")).is_file());
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
        let audit: serde_json::Value = serde_json::from_slice(
            &std::fs::read(base.join(format!("out/Split.{episode}.audit.json"))).unwrap(),
        )
        .unwrap();
        if episode == "01" {
            assert_eq!(audit["cut"]["resolved"]["video_frames"], 768);
        }
        assert_eq!(
            audit["cut"]["source_video_sha256"],
            audit["cut"]["final_video_sha256"]
        );
        let audio = &audit["cut"];
        assert_eq!(
            audio["source_audio_sha256"]["AudioTrack.01"],
            audio["cut_audio_sha256"]["AudioTrack.01"]
        );
        assert_eq!(
            audio["cut_audio_sha256"]["AudioTrack.01"],
            audio["final_audio_sha256"]["AudioTrack.01"]
        );
        assert_eq!(
            audio["source_audio_pcm_bytes"]["AudioTrack.01"],
            audio["cut_audio_pcm_bytes"]["AudioTrack.01"]
        );
        assert_eq!(
            audio["cut_audio_pcm_bytes"]["AudioTrack.01"],
            audio["final_audio_pcm_bytes"]["AudioTrack.01"]
        );
        assert!(
            audio["cut_audio_pcm_bytes"]["AudioTrack.01"]
                .as_u64()
                .unwrap()
                > 0
        );
        let range = &audio["audio_sample_ranges"]["AudioTrack.01"];
        assert!(range["end_sample"].as_u64().unwrap() > range["start_sample"].as_u64().unwrap());
        assert_eq!(range["sample_rate"], 48000);
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
        command.args(["-map", "0:a:0", "-c:a", "pcm_s16le", "-f", "s16le", "-"]);
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
    for (name, color, tone) in [("a", "black", "440"), ("b", "white", "880")] {
        let path = base.join(format!("{name}.m2ts"));
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
    let config = "[project]\nshow='Joined'\nout_dir='out'\nwork_dir='work'\nclean_work_dirs=false\n[subtitles.fonts]\nenabled=false\n[episodes]\n'01'=['a.m2ts','b.m2ts']\n[chapters]\n'01'=[{frame=0,name='First'},{frame=24,name='Second'}]\n[VideoTrack.01]\nsource='{episodes}'\n[AudioTrack.01]\nsource='{episodes}'\ncodec='flac'\n[mux]\n'VideoTrack.01'={name='Video',language='ja-JP'}\n'AudioTrack.01'={name='Audio',language='ja-JP'}\n";
    let config_path = base.join("batch.toml");
    std::fs::write(&config_path, config).unwrap();
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
    }
    let output = base.join("out/Joined.01.mkv");
    assert!(output.is_file());
    assert!(base.join("work/01/a.joined.mkv").is_file());
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
    let audit: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.with_extension("audit.json")).unwrap())
            .unwrap();
    assert_eq!(audit["commands"][0]["program"], "mkvmerge");
    assert_eq!(audit["plan"]["sources"].as_array().unwrap().len(), 2);
}
#[test]
fn bdmv_chapters_use_video_duration_for_terminal_mark_and_audit() {
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
    }
    let chapters = std::fs::read_to_string(base.join("work/01/chapters.txt")).unwrap();
    assert!(chapters.contains("CHAPTER03=00:00:08.000"));
    let audit: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("out/ChapterTest.01.audit.json")).unwrap())
            .unwrap();
    assert_eq!(audit["chapter_timing"]["playlist_duration_ms"], 10000);
    assert_eq!(audit["chapter_timing"]["cutoff_frames"], 264);
    assert_ne!(audit["chapter_timing"]["cutoff_source"], "playlist");
    assert!(audit["chapter_timing"]["difference_ms"].as_i64().unwrap() > 500);
}
#[test]
fn real_batch_flac_copy_subtitles_chapters_flags_and_audit() {
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
srt='{ep}.srt'
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
    for ep in ["01", "02"] {
        let output = base.join(format!("out/Test.{ep}.mkv"));
        assert!(output.exists());
        assert!(!base.join(format!("work/{ep}")).exists());
        let audit: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.with_extension("audit.json")).unwrap())
                .unwrap();
        assert_eq!(audit["state"], "complete");
        assert_eq!(audit["commands"].as_array().unwrap().len(), 9);
        assert_eq!(audit["commands"][1]["program"], "flac");
        let flac_args = audit["commands"][1]["args"].as_array().unwrap();
        assert!(flac_args.iter().any(|arg| arg == "--verify"));
        assert!(flac_args.iter().any(|arg| arg == "-8"));
        assert_eq!(audit["commands"][5]["program"], "ffmpeg");
        assert!(
            audit["commands"][5]["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|arg| arg == "pcm_s24le")
        );
        assert_eq!(audit["commands"][6]["program"], "flac");
        assert_eq!(audit["commands"][7]["program"], "ffmpeg");
        let pgs_args = audit["commands"][7]["args"].as_array().unwrap();
        assert!(
            pgs_args
                .windows(2)
                .any(|args| args[0] == "-map" && args[1] == "0:s:2")
        );
        assert!(
            pgs_args
                .windows(2)
                .any(|args| args[0] == "-c" && args[1] == "copy")
        );
        let mux_args = audit["commands"][8]["args"].as_array().unwrap();
        assert!(mux_args.iter().any(|arg| arg == "--no-global-tags"));
        assert!(mux_args.iter().any(|arg| arg == "--no-chapters"));
        assert_eq!(audit["configuration"]["project"]["show"], "Test");
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
        assert_eq!(tracks[7]["properties"]["flag_hearing_impaired"], true);
        assert_eq!(tracks[7]["properties"]["forced_track"], true);
        assert_eq!(tracks[6]["properties"]["language_ietf"], "pt-BR");
        assert!(tracks[8]["codec"].as_str().unwrap().contains("PGS"));
        assert!(tracks[9]["codec"].as_str().unwrap().contains("PGS"));
        if with_font {
            assert_eq!(info["attachments"].as_array().unwrap().len(), 1);
            assert_eq!(audit["fonts"].as_array().unwrap().len(), 1);
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
    let second = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(config_path)
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("Output already exists"));
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
    let audit: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.with_extension("audit.json")).unwrap())
            .unwrap();
    let args = audit["commands"][0]["args"].as_array().unwrap();
    assert!(!args.iter().any(|arg| arg == "--no-global-tags"));
    assert!(!args.iter().any(|arg| arg == "--no-chapters"));
    for flag in [
        "--colour-range",
        "--colour-primaries",
        "--colour-transfer-characteristics",
        "--colour-matrix",
    ] {
        assert!(args.iter().any(|arg| arg == flag));
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
