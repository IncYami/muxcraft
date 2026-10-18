use muxcraft::{
    chapters,
    config::{self, ChapterSpec, Config, Correction, Track},
    fonts,
    subtitle::{self, Ass, Fps},
};
use std::{collections::BTreeMap, path::Path, process::Command};

fn documentation_config(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/configs")
        .join(name)
}

#[test]
fn committed_documentation_configs_load_and_plan() {
    for config in [
        "simple-batch.toml",
        "joined-parts.toml",
        "joined-bdmv.toml",
        "chapter-cut.toml",
        "per-episode-overlay.toml",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
            .arg(documentation_config(config))
            .args(["--mode", "plan", "--no-check"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{config}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(plan.as_array().is_some_and(|episodes| !episodes.is_empty()));
    }
}

#[test]
fn plan_keeps_stdout_as_json_and_verbose_option_is_accepted() {
    let output = Command::new(env!("CARGO_BIN_EXE_muxcraft"))
        .arg(documentation_config("simple-batch.toml"))
        .args(["--mode", "plan", "--no-check", "--verbose"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(plan.as_array().is_some_and(|episodes| !episodes.is_empty()));
    assert!(output.stderr.is_empty());
}
fn track(extra: &str) -> Track {
    toml::from_str(&format!("id='sub'\nsource='sub.ass'\n{extra}")).unwrap()
}
const ASS: &str = "[Script Info]\nScriptType: v4.00+\n[V4+ Styles]\nFormat: Name, Fontname, Bold, Italic\nStyle: Default,Lato,0,0\nStyle: Sign,Other,0,0\nStyle: Unused,Unused,0,0\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,hello,{\\fnAlt\\b700\\i1}world{\\rSign}sign\nDialogue: 0,0:00:05.00,0:00:06.00,*Missing,,0,0,0,,fallback\n";
#[test]
fn expansion_and_literal_glob() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("[EA] 01.ass"), "").unwrap();
    let vars = BTreeMap::from([
        ("ep".into(), "01".into()),
        ("raw".into(), "{config_dir}".into()),
        ("config_dir".into(), dir.path().display().to_string()),
    ]);
    assert!(config::resolve(dir.path(), "{raw}/[EA] {ep}*.ass", &vars, true).is_ok());
    assert!(config::resolve(dir.path(), "{raw}/[EA] 0?.ass", &vars, true).is_ok());
    std::fs::write(dir.path().join("[EA] 01-extra.ass"), "").unwrap();
    assert!(config::resolve(dir.path(), "{raw}/[EA] {ep}*.ass", &vars, true).is_err());
    assert!(config::expand("{bad}", &vars).is_err());
    assert!(
        config::expand(
            "{a}",
            &BTreeMap::from([("a".into(), "{b}".into()), ("b".into(), "{a}".into())])
        )
        .is_err()
    );
}

#[test]
fn episode_config_inherits_and_overrides_only_its_episode() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("episode-01.toml"),
        r#"
[episodes]
01 = 'actual-01.m2ts'
[subtitles]
US = 'replacement.ass'
CR12 = false
[subtitles.shift]
US = { frames = 0, ranges = [{ start = 1, end = 10, frames = 2 }] }
[VideoTrack]
02 = false
[AudioTrack]
02 = false
[pgs]
PGS = false
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("batch.toml"),
        r#"
[project]
show = 'Example'
out_dir = 'out'
work_dir = 'work'
[episodes]
01 = 'episode-01.toml'
02 = 'actual-02.m2ts'
[VideoTrack.01]
source = '{episodes}'
[AudioTrack.01]
source = '{episodes}'
codec = 'flac'
[VideoTrack.02]
source = '{episodes}'
[AudioTrack.02]
source = '{episodes}'
codec = 'flac'
[subtitles]
US = 'original.ass'
CR12 = 'cr12.ass'
[pgs]
PGS = 'subtitle.sup'
[subtitles.shift]
US = { default = 2 }
[mux]
'VideoTrack.01' = { name = 'Video', language = 'ja' }
'VideoTrack.02' = { name = 'Video 2', language = 'ja' }
'AudioTrack.01' = { name = 'Audio', language = 'ja' }
'AudioTrack.02' = { name = 'Audio 2', language = 'ja' }
US = { name = 'US', language = 'en' }
CR12 = { name = 'CR12', language = 'en' }
PGS = { name = 'PGS', language = 'ja' }
"#,
    )
    .unwrap();
    let cfg = Config::load(&dir.path().join("batch.toml")).unwrap();
    let plan = cfg.plan(dir.path(), false).unwrap();
    let first = plan.iter().find(|p| p.episode == "01").unwrap();
    let second = plan.iter().find(|p| p.episode == "02").unwrap();
    assert!(first.source.ends_with("actual-01.m2ts"));
    assert!(first.tracks.iter().any(|t| t.id == "AudioTrack.01"));
    assert!(!first.tracks.iter().any(|t| t.id == "AudioTrack.02"));
    assert!(!first.tracks.iter().any(|t| t.id == "VideoTrack.02"));
    assert!(!first.tracks.iter().any(|t| t.id == "PGS"));
    assert!(
        first
            .tracks
            .iter()
            .any(|t| t.id == "US" && t.source.ends_with("replacement.ass"))
    );
    let us = cfg
        .for_episode("01")
        .subtitles
        .iter()
        .find(|t| t.id == "US")
        .unwrap();
    let correction = &us.shift_table.as_ref().unwrap().default;
    assert_eq!(correction.shift, 0);
    assert_eq!(correction.ranges.len(), 1);
    assert_eq!(correction.ranges[0].shift, 2);
    assert!(!first.tracks.iter().any(|t| t.id == "CR12"));
    assert!(
        second
            .tracks
            .iter()
            .any(|t| t.id == "US" && t.source.ends_with("original.ass"))
    );
    assert!(second.tracks.iter().any(|t| t.id == "CR12"));
    assert!(second.tracks.iter().any(|t| t.id == "AudioTrack.02"));
    assert!(second.tracks.iter().any(|t| t.id == "VideoTrack.02"));
    assert!(second.tracks.iter().any(|t| t.id == "PGS"));
    std::fs::write(
        dir.path().join("episode-01.toml"),
        "[episodes]\n01='nested.toml'\n",
    )
    .unwrap();
    assert!(
        format!(
            "{:#}",
            Config::load(&dir.path().join("batch.toml")).unwrap_err()
        )
        .contains("cannot include another episode config")
    );
    std::fs::write(
        dir.path().join("episode-01.toml"),
        "[episodes]\n99='actual-01.m2ts'\n",
    )
    .unwrap();
    assert!(
        format!(
            "{:#}",
            Config::load(&dir.path().join("batch.toml")).unwrap_err()
        )
        .contains("must define exactly the referenced episode 01")
    );
}
#[test]
fn ass_preserves_commas_and_collects_overrides_without_fallback() {
    let doc = Ass::parse(ASS).unwrap();
    let requests = fonts::requests(&doc).unwrap();
    assert!(
        requests
            .iter()
            .any(|r| r.name == "Alt" && r.weight == 700 && r.italic)
    );
    assert!(requests.iter().any(|r| r.name == "Other"));
    assert!(requests.iter().any(|r| r.name == "Lato"));
    assert!(!requests.iter().any(|r| r.name == "Unused"));
    let t = track("headers={'YCbCr Matrix'='TV.709'}");
    let out = subtitle::process_ass(
        ASS,
        &t,
        &Correction::default(),
        "01",
        Fps::parse("24").unwrap(),
        None,
        subtitle::AssCleaning {
            garbage: true,
            extradata: true,
        },
    )
    .unwrap()
    .dump();
    assert!(out.contains("hello,{\\fnAlt"));
    assert!(out.contains("YCbCr Matrix: TV.709"));
    assert!(out.contains("Style: Unused"));
    assert!(out.contains("Style: Sign"));
    let missing = ASS.replace(",Default,", ",*Missing,");
    let cleaned = subtitle::process_ass(
        &missing,
        &t,
        &Correction::default(),
        "01",
        Fps::parse("24").unwrap(),
        None,
        subtitle::AssCleaning {
            garbage: true,
            extradata: true,
        },
    )
    .unwrap();
    assert!(cleaned.dump().contains("Style: Default"));
    let requests = fonts::requests(&cleaned).unwrap();
    assert!(!requests.iter().any(|r| r.name == "Lato"));
    assert!(requests.iter().any(|r| r.name == "Alt"));
    assert!(requests.iter().any(|r| r.name == "Other"));
    let no_override = missing.replace("hello,{\\fnAlt\\b700\\i1}world{\\rSign}sign", "plain text");
    assert!(
        fonts::requests(&Ass::parse(&no_override).unwrap())
            .unwrap()
            .is_empty()
    );
}
#[test]
fn missing_font_is_a_warning_and_collects_no_attachment() {
    let dir = tempfile::tempdir().unwrap();
    let request = fonts::Request {
        name: "Missing Family".into(),
        weight: 400,
        italic: false,
    };
    let output = fonts::FontIndex::default()
        .collect(&std::collections::BTreeSet::from([request]), dir.path())
        .unwrap();
    assert!(output.is_empty());
}
#[test]
fn shift_ranges_and_explicit_deletions() {
    let fps = Fps::parse("24").unwrap();
    let mut t = track("");
    t.deletes = vec![config::DeleteRange {
        episodes: vec!["01".into()],
        start: 96,
        end: Some(121),
    }];
    let correction: Correction =
        toml::from_str("shift=24\nranges=[{start=24,end=48,shift=24}]").unwrap();
    let input = "1\n00:00:01,000 --> 00:00:02,000\na\n\n2\n00:00:03,000 --> 00:00:04,000\nb\n\n3\n00:00:04,000 --> 00:00:05,000\nc\n";
    let out = subtitle::process_srt(input, &t, &correction, "01", fps, None).unwrap();
    let cues = subtitle::parse_srt(&out).unwrap();
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].text, "a");
    assert_eq!(fps.frame(cues[0].start, false), 72);
}
#[test]
fn video_truncation_handles_srt_and_ass_boundaries() {
    let fps = Fps::parse("24").unwrap();
    let t = track("");
    let srt = "1\n00:00:00,000 --> 00:00:01,000\na\n\n2\n00:00:01,000 --> 00:00:02,500\nb\n\n3\n00:00:02,000 --> 00:00:03,000\nc\n";
    let trimmed = subtitle::process_srt(
        srt,
        &t,
        &Correction::default(),
        "01",
        fps,
        Some(subtitle::SubtitleCutoff {
            end_ms: 2000,
            trimmed: true,
            comment: false,
        }),
    )
    .unwrap();
    let cues = subtitle::parse_srt(&trimmed).unwrap();
    assert_eq!(cues.len(), 2);
    assert_eq!(cues[1].end, 2000);
    let strict = subtitle::process_srt(
        srt,
        &t,
        &Correction::default(),
        "01",
        fps,
        Some(subtitle::SubtitleCutoff {
            end_ms: 2000,
            trimmed: false,
            comment: false,
        }),
    )
    .unwrap();
    assert_eq!(subtitle::parse_srt(&strict).unwrap().len(), 1);

    let ass = "[Script Info]\nTitle: test\n[V4+ Styles]\nFormat: Name, Fontname, Bold, Italic\nStyle: Default,Arial,0,0\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,a\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,b\nDialogue: 0,0:00:02.00,0:00:03.00,Default,,0,0,0,,c\n";
    let clipped = subtitle::process_ass(
        ass,
        &t,
        &Correction::default(),
        "01",
        fps,
        Some(subtitle::SubtitleCutoff {
            end_ms: 2000,
            trimmed: true,
            comment: false,
        }),
        subtitle::AssCleaning::default(),
    )
    .unwrap()
    .dump();
    assert!(clipped.contains("0:00:02.00,Default,,0,0,0,,b"));
    assert!(!clipped.contains(",,c"));
    let quantized = subtitle::process_ass(
        ass,
        &t,
        &Correction::default(),
        "01",
        fps,
        Some(subtitle::SubtitleCutoff {
            end_ms: 1005,
            trimmed: true,
            comment: false,
        }),
        subtitle::AssCleaning::default(),
    )
    .unwrap()
    .dump();
    assert!(!quantized.contains(",,b"));
}
#[test]
fn ass_video_truncation_can_comment_removed_dialogue() {
    let fps = Fps::parse("24").unwrap();
    let t = track("");
    let ass = "[Script Info]\n[V4+ Styles]\nFormat: Name, Fontname, Bold, Italic\nStyle: Default,Arial,0,0\nStyle: OnlyAfter,Arial,0,0\nStyle: Unused,Arial,0,0\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,crossing\nDialogue: 0,0:00:03.00,0:00:04.00,OnlyAfter,,0,0,0,,after\nComment: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,crossing-comment\nComment: 0,0:00:03.00,0:00:04.00,OnlyAfter,,0,0,0,,after-comment\n";
    let out = subtitle::process_ass(
        ass,
        &t,
        &Correction::default(),
        "01",
        fps,
        Some(subtitle::SubtitleCutoff {
            end_ms: 2000,
            trimmed: true,
            comment: true,
        }),
        subtitle::AssCleaning::default(),
    )
    .unwrap()
    .dump();
    let generated = out
        .find("Comment: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,crossing")
        .unwrap();
    let dialogue = out
        .find("Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,crossing")
        .unwrap();
    assert!(generated < dialogue);
    assert!(out.contains("Comment: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,crossing-comment"));
    assert!(out.contains("Comment: 0,0:00:03.00,0:00:04.00,OnlyAfter,,0,0,0,,after-comment"));
    assert!(out.contains("Comment: 0,0:00:03.00,0:00:04.00,OnlyAfter,,0,0,0,,after"));
    assert!(out.contains("Style: OnlyAfter,Arial,0,0"));
    assert!(out.contains("Style: Unused,Arial,0,0"));

    let preserved = subtitle::process_ass(
        ass,
        &track(""),
        &Correction::default(),
        "01",
        fps,
        Some(subtitle::SubtitleCutoff {
            end_ms: 2000,
            trimmed: true,
            comment: true,
        }),
        subtitle::AssCleaning::default(),
    )
    .unwrap()
    .dump();
    assert!(
        preserved.contains("Comment: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,crossing-comment")
    );
    assert!(preserved.contains("Comment: 0,0:00:03.00,0:00:04.00,OnlyAfter,,0,0,0,,after-comment"));
    assert_eq!(preserved.matches("crossing-comment").count(), 1);
    assert_eq!(preserved.matches("after-comment").count(), 1);

    let strict = subtitle::process_ass(
        ass,
        &track(""),
        &Correction::default(),
        "01",
        fps,
        Some(subtitle::SubtitleCutoff {
            end_ms: 2000,
            trimmed: false,
            comment: true,
        }),
        subtitle::AssCleaning::default(),
    )
    .unwrap()
    .dump();
    assert!(strict.contains("Comment: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,crossing"));
    assert!(!strict.contains("Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,crossing"));
}
#[test]
fn negative_shift_keeps_srt_line_at_zero() {
    let input = "1\n00:00:00,100 --> 00:00:01,000\nhello\n";
    let t = track("shift_frames=-24");
    let fps = Fps::parse("24").unwrap();
    let out = subtitle::process_srt(input, &t, &Correction::default(), "01", fps, None).unwrap();
    let c = subtitle::parse_srt(&out).unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!((c[0].start, c[0].end), (0, 0));
    assert!(subtitle::parse_srt("1\ninvalid\nhello").is_err());
    assert!(subtitle::parse_time("00:61:00,000").is_err());
}
#[test]
fn negative_shift_keeps_ass_dialogue_at_zero() {
    let t = track("shift_frames=-48");
    let out = subtitle::process_ass(
        ASS,
        &t,
        &Correction::default(),
        "01",
        Fps::parse("24").unwrap(),
        None,
        subtitle::AssCleaning::default(),
    )
    .unwrap()
    .dump();
    assert_eq!(
        out.lines()
            .filter(|line| line.starts_with("Dialogue:"))
            .count(),
        2
    );
    assert!(out.contains("Dialogue: 0,0:00:00.00,0:00:00.00,Default,,0,0,0,,hello,"));
    assert!(out.contains("{\\fnAlt\\b700\\i1}world{\\rSign}sign"));
}
#[test]
fn frame_roundtrip_including_ntsc() {
    for rate in ["24", "24000/1001", "30000/1001"] {
        let fps = Fps::parse(rate).unwrap();
        for frame in [1, 24, 2374, 18628, 33023, 50000] {
            assert_eq!(fps.frame(fps.subtitle_ms(frame, false), false), frame);
            assert_eq!(fps.frame(fps.subtitle_ms(frame, true), true), frame);
        }
    }
}
#[test]
fn ass_line_add() {
    let mut t = track("");
    t.additions = vec![config::AddLines {
        episodes: vec!["01".into()],
        lines: vec!["Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,added,comma".into()],
    }];
    let out = subtitle::process_ass(
        ASS,
        &t,
        &Correction::default(),
        "01",
        Fps::parse("24").unwrap(),
        None,
        subtitle::AssCleaning::default(),
    )
    .unwrap();
    assert!(out.dump().contains("added,comma"));
    let sorted = subtitle::process_ass(
        ASS,
        &t,
        &Correction::default(),
        "01",
        Fps::parse("24").unwrap(),
        None,
        subtitle::AssCleaning::default(),
    )
    .unwrap()
    .dump();
    assert!(sorted.find("added,comma").unwrap() < sorted.find(",,fallback").unwrap());
}
fn put16(b: &mut [u8], i: usize, n: u16) {
    b[i..i + 2].copy_from_slice(&n.to_be_bytes());
}
fn put32(b: &mut [u8], i: usize, n: u32) {
    b[i..i + 4].copy_from_slice(&n.to_be_bytes());
}
fn mpls_fixture() -> Vec<u8> {
    let mut b = vec![0u8; 160];
    b[..4].copy_from_slice(b"MPLS");
    put32(&mut b, 8, 32);
    put32(&mut b, 12, 80);
    put16(&mut b, 38, 1);
    put16(&mut b, 42, 20);
    b[44..49].copy_from_slice(b"00001");
    b[49..53].copy_from_slice(b"M2TS");
    put32(&mut b, 56, 45000 * 600);
    put32(&mut b, 60, 45000 * 610);
    put16(&mut b, 84, 3);
    for (i, seconds) in [600, 604, 610].iter().enumerate() {
        let p = 86 + i * 14;
        b[p + 1] = 1;
        put16(&mut b, p + 2, 0);
        put32(&mut b, p + 4, 45000 * seconds);
    }
    b
}
fn joined_mpls_fixture(clips: [&str; 3], middle_mark: u32) -> Vec<u8> {
    let marks_offset = 32 + 10 + 3 * 22;
    let mut b = vec![0u8; marks_offset + 6 + 3 * 14];
    b[..4].copy_from_slice(b"MPLS");
    put32(&mut b, 8, 32);
    put32(&mut b, 12, marks_offset as u32);
    put16(&mut b, 38, 3);
    for (i, (clip, (input, output))) in clips
        .into_iter()
        .zip([(600, 610), (609, 629), (622, 632)])
        .enumerate()
    {
        let pos = 42 + i * 22;
        put16(&mut b, pos, 20);
        b[pos + 2..pos + 7].copy_from_slice(clip.as_bytes());
        b[pos + 7..pos + 11].copy_from_slice(b"M2TS");
        put32(&mut b, pos + 14, input * 45000);
        put32(&mut b, pos + 18, output * 45000);
    }
    put16(&mut b, marks_offset + 4, 3);
    for (i, (item, tick)) in [
        (0, 600 * 45000),
        (1, middle_mark * 45000),
        (2, 631 * 45000 + 44000),
    ]
    .into_iter()
    .enumerate()
    {
        let pos = marks_offset + 6 + i * 14;
        b[pos + 1] = 1;
        put16(&mut b, pos + 2, item);
        put32(&mut b, pos + 4, tick);
    }
    b
}
#[test]
fn joined_bdmv_chapters_follow_exact_clip_order_and_playitem_timing() {
    let dir = tempfile::tempdir().unwrap();
    let bdmv = dir.path().join("BDMV");
    let stream = bdmv.join("STREAM");
    let playlists = bdmv.join("PLAYLIST");
    std::fs::create_dir_all(&stream).unwrap();
    std::fs::create_dir_all(&playlists).unwrap();
    for clip in ["00001", "00002", "00003", "00004", "00005"] {
        std::fs::write(stream.join(format!("{clip}.m2ts")), b"").unwrap();
    }
    std::fs::write(
        playlists.join("00004.mpls"),
        joined_mpls_fixture(["00002", "00003", "00005"], 614),
    )
    .unwrap();
    std::fs::write(
        playlists.join("00012.mpls"),
        joined_mpls_fixture(["00001", "00003", "00004"], 614),
    )
    .unwrap();
    for (clips, expected) in [
        (["00002", "00003", "00005"], "00004.mpls"),
        (["00001", "00003", "00004"], "00012.mpls"),
    ] {
        let sources = clips
            .map(|clip| stream.join(format!("{clip}.m2ts")))
            .to_vec();
        let (playlist, chapters) = chapters::bdmv_joined(&sources, Fps::parse("24").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(playlist, playlists.join(expected));
        assert_eq!(chapters.times, [0, 15000]);
        assert_eq!(chapters.duration_ms, 40000);
    }
    let wrong_order = ["00003", "00002", "00005"].map(|clip| stream.join(format!("{clip}.m2ts")));
    assert!(
        chapters::bdmv_joined(&wrong_order, Fps::parse("24").unwrap())
            .unwrap()
            .is_none()
    );
    std::fs::write(
        playlists.join("00013.mpls"),
        joined_mpls_fixture(["00002", "00003", "00005"], 615),
    )
    .unwrap();
    let jp = ["00002", "00003", "00005"].map(|clip| stream.join(format!("{clip}.m2ts")));
    assert!(chapters::bdmv_joined(&jp, Fps::parse("24").unwrap()).is_err());
}
#[test]
fn mpls_offsets_marks_and_terminal_cutoff() {
    let b = mpls_fixture();
    let cuts = chapters::cut_marks(&b, "00001").unwrap();
    assert_eq!(cuts.input_ticks, 45000 * 600);
    assert_eq!(cuts.output_ticks, 45000 * 610);
    assert_eq!(cuts.marks, vec![45000 * 600, 45000 * 604]);
    let ch = chapters::parse_mpls(&b, "00001", Fps::parse("24").unwrap(), None)
        .unwrap()
        .unwrap();
    assert_eq!(ch.times, vec![0, 4000]);
    assert_eq!(ch.duration_ms, 10000);
    assert_eq!(ch.duration_frames, 240);
    let mut near_end = b.clone();
    put32(&mut near_end, 118, 45000 * 608);
    let playlist_cutoff = chapters::parse_mpls(&near_end, "00001", Fps::parse("24").unwrap(), None)
        .unwrap()
        .unwrap();
    assert_eq!(playlist_cutoff.times, vec![0, 4000]);
    let video_cutoff =
        chapters::parse_mpls(&near_end, "00001", Fps::parse("24").unwrap(), Some(264))
            .unwrap()
            .unwrap();
    assert_eq!(video_cutoff.times, vec![0, 4000, 8000]);
    let mut between_frames = b.clone();
    put32(&mut between_frames, 104, 45000 * 604 + 1000);
    let precise = chapters::parse_mpls(&between_frames, "00001", Fps::parse("24").unwrap(), None)
        .unwrap()
        .unwrap();
    assert_eq!(precise.times, vec![0, 4022]);
    assert!(
        chapters::ogm(&chapters::name_times(precise.times, &[]).unwrap())
            .unwrap()
            .contains("CHAPTER02=00:00:04.022")
    );
    assert!(chapters::parse_mpls(&b[..60], "00001", Fps::parse("24").unwrap(), None).is_err());
    assert!(
        chapters::parse_mpls(&b, "00002", Fps::parse("24").unwrap(), None)
            .unwrap()
            .is_none()
    );
}
#[test]
fn bdmv_playlist_selection_and_ambiguity() {
    let dir = tempfile::tempdir().unwrap();
    let bdmv = dir.path().join("BDMV");
    let stream = bdmv.join("STREAM");
    let playlists = bdmv.join("PLAYLIST");
    std::fs::create_dir_all(&stream).unwrap();
    std::fs::create_dir_all(&playlists).unwrap();
    let clip = stream.join("00001.m2ts");
    std::fs::write(&clip, b"").unwrap();
    let fixture = mpls_fixture();
    std::fs::write(playlists.join("00001.mpls"), &fixture).unwrap();
    std::fs::write(playlists.join("00002.mpls"), &fixture).unwrap();
    assert_eq!(
        chapters::discover_cut_playlist(&clip).unwrap(),
        playlists.join("00001.mpls")
    );
    let (_, selected) = chapters::bdmv(&clip, None, Fps::parse("24").unwrap(), None).unwrap();
    assert_eq!(selected.times, [0, 4000]);
    let mut different = fixture;
    put32(&mut different, 104, 45000 * 605);
    std::fs::write(playlists.join("00002.mpls"), different).unwrap();
    assert!(chapters::discover_cut_playlist(&clip).is_err());
    assert!(chapters::bdmv(&clip, None, Fps::parse("24").unwrap(), None).is_err());
}
#[test]
fn tmdb_multilingual_xml() {
    let cfg: config::Tmdb = toml::from_str("id=1\nlanguages=['ja-JP','en-US']").unwrap();
    let data = vec![
        (
            "ja-JP".into(),
            serde_json::json!({"overview":"日本 & <test>","external_ids":{"tvdb_id":123,"imdb_id":"tt1"}}),
            Some(serde_json::json!({"name":"日本","overview":"概要","air_date":"2026-01-01"})),
        ),
        (
            "en-US".into(),
            serde_json::json!({"overview":"English"}),
            Some(serde_json::json!({"name":"Title","overview":"Synopsis"})),
        ),
    ];
    let xml = muxcraft::tmdb::make_tags(&cfg, &data);
    assert!(xml.contains("&amp; &lt;test&gt;"));
    assert!(xml.contains("<TagLanguage>jpn</TagLanguage>"));
    assert!(xml.contains("<TagLanguageIETF>en-US</TagLanguageIETF>"));
    assert!(xml.contains("<Name>TVDB</Name>"));
}
#[test]
fn local_examples_parse_and_expand() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    if !dir.is_dir() {
        return;
    }
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|s| s == "toml") {
            let cfg = Config::load(&path).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            assert!(cfg.fonts.use_system_fonts);
            let plans = cfg
                .plan(&dir, false)
                .unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
            assert!(!plans.is_empty());
            count += 1;
        }
    }
    assert_eq!(count, 7);
}

#[test]
fn numbered_tracks_and_subtitle_settings_are_resolved() {
    let dir = tempfile::tempdir().unwrap();
    let input = r#"
[project]
show='Example'
out_dir='out'
work_dir='work'
[episodes]
'01'='disc.mkv'
[VideoTrack.01]
source='{episodes}'
track=0
audio=true
subtitles=true
attachments=true
global_tags=true
chapters=true
colour={range=1,primaries=1,transfer=1,matrix=1}
[VideoTrack.02]
source='alternate.mkv'
track=1
[AudioTrack.01]
source='{episodes}'
track=1
codec='copy'
[subtitles]
US='us.ass'
CR='cr.ass'
[subtitles.processing]
clean_garbage=true
ycbcr_matrix='auto'
[subtitles.shift]
US={default=24,'01'={frames=48,ranges=[{start=2374,frames=72}]}}
CR={default=24,'01'={frames=48,ranges=[{start=2374,frames=72}]}}
[subtitles.delete]
US=[{episodes=['01'],start=72,end=96}]
[subtitles.add]
CR=[{episodes=['01'],lines=['Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Added line']}]
[mux]
'VideoTrack.01'={name='Main',language='ja-JP',default=true}
'VideoTrack.02'={name='Alternate',language='ja-JP'}
'AudioTrack.01'={name='Commentary',language='en-US'}
US={name='Kaleido',language='en-US',default=true}
CR={name='Crunchyroll',language='en-US'}
"#;
    let path = dir.path().join("batch.toml");
    std::fs::write(&path, input).unwrap();
    let cfg = Config::load(&path).unwrap();
    let plan = cfg.plan(dir.path(), false).unwrap();
    let ids: Vec<_> = plan[0].tracks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "VideoTrack.01",
            "VideoTrack.02",
            "AudioTrack.01",
            "US",
            "CR"
        ]
    );
    assert_eq!(plan[0].tracks[1].track, 1);
    assert!(cfg.video[0].audio && cfg.video[0].subtitles && cfg.video[0].attachments);
    assert!(cfg.video[0].global_tags && cfg.video[0].chapters);
    assert!(!cfg.video[1].global_tags && !cfg.video[1].chapters);
    assert_eq!(cfg.video[0].colour.as_ref().unwrap().matrix, Some(1));
    assert_eq!(plan[0].tracks[1].name, "Alternate");
    assert_eq!(plan[0].tracks[2].language, "en-US");
    let us = cfg.subtitles[0].shift_table.as_ref().unwrap();
    let cr = cfg.subtitles[1].shift_table.as_ref().unwrap();
    assert_eq!(us.episodes["01"].shift, 48);
    assert_eq!(cr.episodes["01"].ranges[0].shift, 72);
    assert_eq!(cfg.subtitles[0].deletes[0].start, 72);
    assert_eq!(cfg.subtitles[1].additions[0].lines.len(), 1);
    for invalid in [
        input.replace(
            "US=[{episodes=['01'],start=72,end=96}]",
            "US=[{start=96,end=72}]",
        ),
        input.replace(
            "US=[{episodes=['01'],start=72,end=96}]",
            "BAD=[{start=72}]",
        ),
        input.replace(
            "CR=[{episodes=['01'],lines=['Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Added line']}]",
            "BAD=[{lines=['x']}]",
        ),
        input.replace("US='us.ass'", "US={source='us.ass',patches=[]}"),
        input.replace(
            "US={default=24,'01'={frames=48,ranges=[{start=2374,frames=72}]}}",
            "US={default={frames=24,delete=[{start=72}]},'01'=48}",
        ),
        input.replace(
            "US={default=24,'01'={frames=48,ranges=[{start=2374,frames=72}]}}",
            "US={default=24,out_of_bounds='drop_line','01'=48}",
        ),
        input.replace(
            "CR={default=24,'01'={frames=48,ranges=[{start=2374,frames=72}]}}",
            "CR='US'",
        ),
        input.replace(
            "CR={default=24,'01'={frames=48,ranges=[{start=2374,frames=72}]}}",
            "CR={default=24,'99'=48}",
        ),
        input.replace(
            "CR={default=24,'01'={frames=48,ranges=[{start=2374,frames=72}]}}",
            "CR={'01'=48}",
        ),
        input.replace("'VideoTrack.02'={name='Alternate',language='ja-JP'}\n", ""),
        input.replace("[VideoTrack.02]", "[VideoTrack.bad]"),
        input.replace("[VideoTrack.01]\nsource='{episodes}'", "[VideoTrack.01]"),
        input.replace("[AudioTrack.01]\nsource='{episodes}'", "[AudioTrack.01]"),
        input.replace("codec='copy'", ""),
        input.replace("codec='copy'", "codec='copy'\nkind='ass'"),
        input.replace("US='us.ass'", "US={source='us.ass',kind='ass'}"),
        input.replace("US='us.ass'", "US={source='us.srt',kind='srt_to_ass'}"),
        input.replace("US='us.ass'", "US={source='us.ass',style='LATO'}"),
        input.replace("US='us.ass'", "US={source='us.ass',glob=true}"),
        input.replace("range=1", "range=4"),
        input.replace("matrix=1", "matrix='bt709'"),
        input.replace("matrix=1", "matrix=1,unknown=1"),
        input.replace("global_tags=true", "global_tags='yes'"),
        input.replace(
            "[VideoTrack.02]\nsource='alternate.mkv'",
            "[VideoTrack.02]\nsource='alternate.mkv'\nchapters=true",
        ),
        format!("{input}\n[chapters]\n'01'=['Intro']\n"),
        input.replace("track=0\naudio=true", "track=0\naudio='none'"),
        input.replace(
            "track=0\naudio=true",
            "track=0\naudio=true\nargs=['--colour-range','0:1']",
        ),
        format!("{input}\n[styles.LATO]\nFontname='Lato'\n"),
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(Config::load(&path).is_err());
    }
}

#[test]
fn pgs_tracks_have_their_own_section_and_unique_ids() {
    let dir = tempfile::tempdir().unwrap();
    let source = "[project]\nshow='Test'\nout_dir='out'\nwork_dir='work'\n[episodes]\n'01'='source.mkv'\n[VideoTrack.01]\nsource='{episodes}'\n[subtitles]\nUS='US.ass'\n[pgs]\nPGS='{episodes}'\nPGS2={source='subtitles.mks',track=1,only=['01']}\n[mux]\n'VideoTrack.01'={name='Video',language='ja-JP'}\nUS={name='US',language='en-US'}\nPGS={name='PGS',language='ja-JP'}\nPGS2={name='Second PGS',language='ja-JP'}\n";
    let path = dir.path().join("batch.toml");
    std::fs::write(&path, source).unwrap();
    let cfg = Config::load(&path).unwrap();
    let plan = cfg.plan(dir.path(), false).unwrap();
    assert_eq!(cfg.pgs.len(), 2);
    assert_eq!(plan[0].tracks[2].id, "PGS");
    assert_eq!(plan[0].tracks[3].id, "PGS2");
    assert_eq!(plan[0].tracks[3].track, 1);

    let duplicate = source.replace("PGS='{episodes}'", "US='{episodes}'");
    std::fs::write(&path, duplicate).unwrap();
    assert!(Config::load(&path).is_err());
}

#[test]
fn episode_maps_preserve_order_and_resolve_grouped_variables() {
    let dir = tempfile::tempdir().unwrap();
    let input = r#"
[project]
show='Test'
out_dir='out'
work_dir='work'
[vars]
DISC_ID={ 'Disc A'=['1','10'], 'Disc B'=['2','3'] }
[episodes]
'1'='one.mkv'
'10'='ten.mkv'
'2'='two.mkv'
'3'='three.mkv'
[chapters]
'1'=[]
'10'=['Opening']
'2'=[{frame=0,name='First'},{frame=24,name='Second'}]
'3'='chapters.txt'
[VideoTrack.01]
source='{episodes}'
[mux]
'VideoTrack.01'={name='{DISC_ID}',language='ja-JP'}
"#;
    let path = dir.path().join("batch.toml");
    std::fs::write(&path, input).unwrap();
    let cfg = Config::load(&path).unwrap();
    let ids: Vec<_> = cfg.episodes.iter().map(|ep| ep.id.as_str()).collect();
    assert_eq!(ids, ["1", "10", "2", "3"]);
    assert!(
        matches!(cfg.episodes[0].chapters, Some(ChapterSpec::Names(ref names)) if names.is_empty())
    );
    assert!(
        matches!(cfg.episodes[1].chapters, Some(ChapterSpec::Names(ref names)) if names == &["Opening"])
    );
    assert!(
        matches!(cfg.episodes[2].chapters, Some(ChapterSpec::Frames(ref frames)) if frames.len() == 2)
    );
    assert!(
        matches!(cfg.episodes[3].chapters, Some(ChapterSpec::File(ref file)) if file == "chapters.txt")
    );
    let plans = cfg.plan(dir.path(), false).unwrap();
    assert_eq!(plans[0].tracks[0].name, "Disc A");
    assert_eq!(plans[1].tracks[0].name, "Disc A");
    assert_eq!(plans[2].tracks[0].name, "Disc B");
    assert_eq!(plans[3].tracks[0].name, "Disc B");
    for invalid in [
        input.replace("'Disc B'=['2','3']", "'Disc B'=['2','3','99']"),
        input.replace("'Disc B'=['2','3']", "'Disc B'=['2','3','1']"),
        input.replace("'3'='chapters.txt'", "'99'='chapters.txt'"),
        input.replace("'3'='chapters.txt'", "'3'='chapters.ogm'"),
        input.replace("'1'='one.mkv'", "'bad.id'='one.mkv'"),
        input.replace("[VideoTrack.01]", "[video]"),
        input.replace("[VideoTrack.01]", "[VideoTrack.01]\nid='video'"),
        input.replace("[VideoTrack.01]", "[VideoTrack.01]\nname='BD'"),
        input.replace("[vars]", "[vars]\nsource='old'"),
        format!("{input}\n[[audio]]\nid='audio'\nsource='one.mkv'\n"),
        format!("{input}\n[shifts.OLD]\ndefault={{ shift=24 }}\n"),
        input.replace(
            "[episodes]\n'1'='one.mkv'",
            "[[episodes]]\nid='1'\nsource='one.mkv'",
        ),
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(Config::load(&path).is_err());
    }
    std::fs::write(
        &path,
        input.replace("source='{episodes}'", "source='{source}'"),
    )
    .unwrap();
    assert!(
        Config::load(&path)
            .unwrap()
            .plan(dir.path(), false)
            .is_err()
    );
    std::fs::write(&path, input.replace("'Disc B'=['2','3']", "'Disc B'=['2']")).unwrap();
    assert!(
        Config::load(&path)
            .unwrap()
            .plan(dir.path(), false)
            .is_err()
    );
    std::fs::write(&path, input.replace("chapters.txt", "chapters.xml")).unwrap();
    assert!(Config::load(&path).is_ok());
}

#[test]
fn derived_episode_variable_applies_offset_and_padding() {
    let dir = tempfile::tempdir().unwrap();
    let input = r#"
[project]
show='Test'
out_dir='out'
work_dir='work'
title='Episode {season_episode}'
[vars]
season_episode={from='ep',offset=-28,pad=2}
[episodes]
'29'='29.mkv'
'38'='38.mkv'
[VideoTrack.01]
source='{episodes}'
[subtitles]
CR='S02E{season_episode}.ass'
[mux]
'VideoTrack.01'={name='BD',language='ja-JP'}
CR={name='CR',language='en-US'}
"#;
    let path = dir.path().join("batch.toml");
    std::fs::write(&path, input).unwrap();
    let cfg = Config::load(&path).unwrap();
    let plans = cfg.plan(dir.path(), false).unwrap();
    assert_eq!(cfg.episodes[0].vars["season_episode"], "01");
    assert_eq!(cfg.episodes[1].vars["season_episode"], "10");
    assert_eq!(plans[0].title, "Episode 01");
    assert_eq!(plans[0].tracks[1].source.file_name().unwrap(), "S02E01.ass");
    assert_eq!(plans[1].tracks[1].source.file_name().unwrap(), "S02E10.ass");
    for invalid in [
        input.replace("from='ep'", "from='episodes'"),
        input.replace("offset=-28", "offset=-29"),
        input.replace("pad=2", "pad=0"),
        input.replace("pad=2", "pad=11"),
        input.replace("pad=2", "pad=2,unknown=true"),
        input.replace("'29'='29.mkv'", "'special'='29.mkv'"),
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(Config::load(&path).is_err());
    }
}

#[test]
fn ogm_chapter_timestamps_and_names() {
    let entries = vec![(0, "Opening".into()), (4260, "Part Two".into())];
    assert_eq!(
        chapters::ogm(&entries).unwrap(),
        "CHAPTER01=00:00:00.000\nCHAPTER01NAME=Opening\nCHAPTER02=00:00:04.260\nCHAPTER02NAME=Part Two\n"
    );
    assert_eq!(
        chapters::name_times(vec![0, 4000], &[]).unwrap()[1].1,
        "Chapter 02"
    );
    assert!(chapters::name_times(vec![0, 4000], &["Only One".into()]).is_err());
    assert!(chapters::ogm(&[(0, "One".into()), (0, "Two".into())]).is_err());
}

#[test]
fn mkv_millisecond_boundaries_use_quantized_pts() {
    let fps = Fps::parse("24000/1001").unwrap();
    assert_eq!(fps.frame(459, false), 11);
    assert_eq!(fps.frame(459, true), 10);
    for frame in 1..2000 {
        let ms = fps.millis(frame);
        assert_eq!(fps.frame(ms, false), frame);
        assert_eq!(fps.frame(ms, true), frame - 1);
    }
    assert_eq!(fps.subtitle_ms(2, false), 60);
}
