# muxcraft

A Rust binary for **batch remuxing** from an auditable TOML file. The configuration records source paths, tracks, fonts, chapters, and subtitle corrections.

## Install and run

On NixOS or any system with the Nix package manager and flakes enabled, install the latest GitHub revision with:

```sh
nix --extra-experimental-features 'nix-command flakes' profile install github:IncYami/muxcraft
```

The package adds FFmpeg, ffprobe, FLAC, and MKVToolNix to the application's runtime `PATH`. To run it without installing:

```sh
nix run github:IncYami/muxcraft -- my-batch.toml --mode plan --no-check
```

### NixOS without flakes

The repository also contains `default.nix`. Add it to `environment.systemPackages` in `configuration.nix` without enabling flakes or submitting the package to Nixpkgs:

```nix
{ pkgs, ... }:
let
  muxcraftSource = builtins.fetchGit {
    url = "https://github.com/IncYami/muxcraft.git";
    ref = "main";
  };
in {
  environment.systemPackages = [
    (import muxcraftSource { inherit pkgs; })
  ];
}
```

Run `sudo nixos-rebuild switch` to install or update muxcraft. Nix fetches the current `main` commit when it refreshes its Git cache, so future rebuilds can pick up new commits without editing this configuration. This builds against your NixOS `pkgs` and adds FFmpeg, FLAC, MKVToolNix, and CA certificates through the package wrapper. You can also test the local package with `nix-build default.nix`.

### Release archives

Pushing a `v*` tag builds Linux x86_64, Windows x86_64, and macOS arm64 binaries and publishes them under [GitHub Releases](https://github.com/IncYami/muxcraft/releases) with a `SHA256SUMS` file. A manual workflow run builds the same archives without publishing a release. The archives contain muxcraft only: install FFmpeg, ffprobe, FLAC, and MKVToolNix separately and make them available on `PATH` (or configure `[tools]`). The Nix package includes these tools in its runtime path.

## Build from source

Building requires Rust and Cargo. Running requires **FFmpeg, ffprobe, FLAC, and MKVToolNix** on `PATH`, or explicit paths under `[tools]`. The binary does not require Python, VapourSynth, FontCollector, or installed Rust crates on the machine where it runs. Media conversions still use the external tools; rewriting the orchestration in Rust does not make FLAC encoding faster.

```sh
cargo build --release --locked
./target/release/muxcraft my-batch.toml --mode plan --no-check
./target/release/muxcraft my-batch.toml --mode check
./target/release/muxcraft my-batch.toml
```

In a Nix environment:

```sh
nix-shell
cargo build --release --locked
cargo test --locked
cargo clippy --all-targets -- -D warnings
cargo audit
```

`cargo audit` checks `Cargo.lock` against the RustSec advisory database; it does not inspect muxcraft's source code or external media tools. The Nix shell includes `cargo-audit`.

`--mode plan` prints JSON with resolved paths and track order; normally it requires existing files and unambiguous globs. `--no-check` shows a template plan when the source media is unavailable. `--mode check` validates tracks and FPS, processes subtitles in memory, and checks fonts and chapters without encoding audio or making an MKV. A run checks the batch before its first mux. `--episodes 01,02` selects episode IDs; the default runs all episodes in order.

The local `examples/` directory contains sample configurations but is excluded from the GitHub distribution because its media paths are machine-specific. Create `my-batch.toml` using the configuration example below. Validate media paths and chapter names against your files. Missing ASS styles and fonts produce warnings and do not stop the batch. Ambiguous chapters remain an error.

## Why TOML

TOML supports comments, literal paths, explicit tables, and readable typed settings. Variables such as `{raw}`, `{ep}`, and `{DISC_ID}` avoid repeating paths without turning the configuration into a program. The order of entries in `[episodes]` is the batch order. An ambiguous glob is never resolved by choosing an arbitrary file.

Use single quotes for Windows paths so backslashes stay literal, for example `bdmv = '\\server\disc\[RELEASE]'`. Relative paths start at the TOML directory, regardless of the command's working directory. `{config_dir}` denotes that directory; `{show}` is the show name; `{ep}` is the current episode ID. `{episodes}` is the resolved file from `[episodes]`: for a list of clips, it points to the joined MKV in the episode workdir. A string in `[vars]` applies to every episode. A grouped table such as `[vars.DISC_ID]` assigns each quoted disc name to the episode IDs in its array. An episode may appear only once per grouped variable, and referenced episode IDs must exist. Variable references can refer to other variables; unknown and circular references are errors.

An episode can list source clips in playback order:

```toml
[episodes]
'01' = [
  '{bdmv}/BDMV/STREAM/00002.m2ts',
  '{bdmv}/BDMV/STREAM/00003.m2ts',
  '{bdmv}/BDMV/STREAM/00004.m2ts',
]

[VideoTrack.01]
source = '{episodes}'

[AudioTrack.01]
source = '{episodes}'
codec = 'flac'
```

Before processing the tracks, muxcraft appends the clips with MKVToolNix to `joined-source.mkv` in the episode workdir. This is a lossless remux, so it uses extra disk space and time but gives video, audio, and PGS one timeline. Each clip must expose the same track order and codecs. `--mode check` validates the individual clips without creating the joined file; `--mode plan` lists both the original clips and the joined path. A failed join leaves the workdir and audit for inspection.

For a season whose subtitle numbering differs from the episode IDs, define a derived variable: `season_episode = { from = 'ep', offset = -28, pad = 2 }` under `[vars]`. Episode `29` then expands `{season_episode}` to `01`, while episode `38` expands it to `10`. `from` must be `ep`; the IDs must be numeric, the offset result must be positive, and `pad` must be between 1 and 10. `{ep}` retains the original ID.

## Configuration example

```toml
[project]
show = 'Fate/Strange Fake'
out_dir = '/mnt/Muxed'
work_dir = '/mnt/Muxed/_workdir'
output = 'Fate.Strange.Fake.{ep}.1080p.BluRay.REMUX.AVC.FLAC.2.0-Shadow.mkv'
title = '{show} - {ep} 1080p BluRay REMUX AVC FLAC 2.0 - Shadow'
fps = '24000/1001'
clean_work_dirs = true

[vars]
bdmv = '/mnt/BDMV'
raw = '{config_dir}/Raw'

[vars.DISC_ID]
'ANIPLEX ANZX-16981' = ['01', '02']

[episodes]
'01' = '{bdmv}/ANZX-16981/BDMV/STREAM/00001.m2ts'
'02' = '{bdmv}/ANZX-16981/BDMV/STREAM/00002.m2ts'

[chapters]
'01' = ['AVANT', 'A-PART', 'B-PART', 'ENDING', 'C-PART']
'02' = ['AVANT', 'OPENING', 'A-PART', 'B-PART', 'ENDING']

[VideoTrack.01]
source = '{episodes}'
track = 0
audio = false
subtitles = false
attachments = false
global_tags = false
chapters = false
colour = { range = 1, primaries = 1, transfer = 1, matrix = 1 }

[AudioTrack.01]
source = '{episodes}'
track = 0
codec = 'flac'

[subtitles]
US = '{raw}/Kaleido/*S01E{ep}*.ass'
CR01 = '{raw}/Crunchyroll/Fate.strange.Fake.S01E{ep}.CR.en-US.ass'
SDH = '{raw}/SDH/Fate.strange.Fake.S01E{ep}.srt'

[subtitles.processing]
clean_garbage = true
clean_extradata = true
ycbcr_matrix = 'auto'

[subtitles.shift]
US = 24
CR01 = { default = 24, '01' = { frames = 24, ranges = [{ start = 2374, end = 18628, frames = 72 }, { start = 33023, frames = -1438 }] } }

[pgs]
PGS = '{episodes}'

[mux]
'VideoTrack.01' = { name = 'JPN BD 1080p ({DISC_ID})', language = 'ja-JP', default = true }
'AudioTrack.01' = { name = 'FLAC 2.0', language = 'ja-JP', default = true }
US = { name = 'Kaleido', language = 'en-US', default = true }
CR01 = { name = 'Crunchyroll', language = 'en-US', default = false }
SDH = { name = 'SDH', language = 'en-US', hearing_impaired = true }
PGS = { name = 'PGS', language = 'ja-JP' }
```

## Tracks and subtitles

| Setting | Behavior |
| --- | --- |
| `[VideoTrack.01]`, `[VideoTrack.02]` | Copy the selected type-relative video tracks. Set `source = '{episodes}'` when video comes from the current episode; each track can also use a different path. |
| `audio`, `subtitles`, `attachments` in `[VideoTrack.01]` | `true` keeps all source tracks or attachments of that category; `false` excludes them. Each defaults to `false`. The selected video track is controlled by `track`. |
| `global_tags`, `chapters` in `[VideoTrack.01]` | `true` keeps the source file's global tags or chapters; `false` excludes them. Both default to `false`. |
| `colour = { range = 1, primaries = 1, transfer = 1, matrix = 1 }` | Sets the MKVToolNix video color metadata for the selected video track. Omit a field to leave that property as supplied by the source. Values are numeric MKVToolNix/Matroska codes. |
| `[AudioTrack.01]`, `codec = 'flac'` | FFmpeg extracts `0:a:track` first. If necessary, FFmpeg then prepares a PCM Wave64 file. The standalone `flac` executable encodes at level 8 with `--verify`. No normalization or resampling is configured. |
| `[AudioTrack.02]`, `codec = 'copy'` | FFmpeg extracts the original audio bitstream to a separate file for muxing. PCM is written as WAV. |
| `kind = 'ass'` | Edits ASS in Rust while preserving dialogue text, tags, and fields. |
| `kind = 'srt'` | Keeps SRT and supports shifts, deletions, and additions. |
| `[pgs]` | FFmpeg extracts a PGS stream from a container to `.sup` with stream copy, then MKVToolNix muxes it. An existing `.sup` is used directly. Text subtitle processing does not apply. |

FLAC tracks receive `ENCODER` (the FLAC version) and `ENCODER_SETTINGS` (the encoder options) as Matroska track tags. Tags explicitly supplied in `[mux]` take precedence over these generated values. Copy tracks do not receive encoding tags.

The `.01`, `.02` suffixes identify output tracks; `track` is a separate **zero-based index within the source media type**. The program resolves MKVToolNix track IDs; FFmpeg selects audio by type as the scripts do. Explicit tracks follow TOML order: videos, audios, then subtitles. Audio and subtitle tracks kept by a video source follow that video's track.

Every video and audio table must declare `source`; audio tables must also declare `codec`. `[mux]` gives each track its `name`, `language`, `default`, `forced`, `hearing_impaired`, `delay_ms`, and `tags`. Every track needs a `[mux]` entry. Dotted IDs must be quoted there: `'AudioTrack.01'`. The source track tables accept `args` (individual mkvmerge arguments, without a shell). Root-level `mux_args` adds global mkvmerge arguments. Languages keep their IETF code, such as `pt-BR` or `es-419`. Track flags are set explicitly.

`audio = true` and `subtitles = true` preserve all tracks of those types from the video source, including their source names and languages. Use separate `[AudioTrack.xx]` or `[subtitles]` entries to select and configure individual tracks. `attachments = true` retains source attachments; fonts collected from ASS files are also attached when `[subtitles.fonts]` is enabled. Use either `chapters = true` for chapters embedded in one video source or `[chapters]` for that episode; combining them is rejected. `global_tags = true` retains source global tags alongside any generated TMDB tags.

Paths containing `*` or `?` are searched automatically after variable expansion. Both wildcards match within one path segment. Square brackets are literal, which is useful for release names like `[EA]`. The pattern must match exactly one file. Recursive `**` is unsupported; enumerate directories or use variables.

`[subtitles]` maps a text subtitle ID to a path or to an inline table when it needs `kind`, `track`, `only`, `except`, `overrides`, or other options. An `.ass` or `.srt` extension selects its type automatically. SRT remains SRT; muxcraft does not convert subtitle formats.

`[pgs]` maps each image subtitle ID directly to a container or `.sup` file. A string selects the first PGS stream; use `{ source = '{episodes}', track = 1 }` to select the second PGS stream from a container. `track` is zero-based among PGS streams. `only`, `except`, and `args` are available in the inline form. PGS tracks are muxed after text subtitles and use the same `[mux]` naming and language settings. Container extraction uses FFmpeg stream copy, with no automatic delay preservation. Text cleaning, shifts, deletions, and additions do not apply to PGS; use `delay_ms` in `[mux]` for timing.

`[subtitles.processing]` sets shared `clean_styles`, `clean_garbage`, `clean_extradata`, `clean_comments`, and `ycbcr_matrix` options for ASS. `ycbcr_matrix = 'auto'` reads the video's color matrix and range with ffprobe. BT.709 limited range becomes `TV.709`; BT.709 full range becomes `PC.709`. An unsupported or unknown matrix becomes `None` with a warning; an unspecified range assumes limited range with a warning. You may set `None`, `TV.709`, or `PC.709` explicitly. This writes the ASS header; it does not recolor subtitle pixels. Individual subtitle tables may still set `clean`, `headers`, or `ycbcr_matrix` when needed. See [libass color matrix notes](https://github.com/libass/libass/blob/master/libass/ass_types.h).

Cleaning removes unused styles, project/extradata sections, and Comment events. Styles referenced by dialogue or `\rName` are retained. Missing styles are reported and never cause an unrelated style to be retained as a font fallback.


### Episode exceptions

```toml
[subtitles]
US = { source = '{raw}/Kaleido/*S01E{ep}*.ass',
       except = ['01'], overrides = { '02' = { source = '{raw}/Kaleido/*PMR*.ass' } } }
```

`except` omits a track from listed episodes; `only` includes it only for listed episodes. Overrides accept `source`. Timing corrections belong under `[subtitles.shift]`; output names belong under `[mux]`.

### Subtitle timing and edits

`[subtitles.shift]` maps each subtitle ID directly to a frame count. For an episode exception, use an inline table with `default` and quoted episode IDs, such as `US = { default = 24, '01' = 0 }`. An episode value replaces the default correction. A value can also be a table with `frames` and `ranges`. Shift ranges are half-open `[start, end)` and use the starting frame before correction; omitting `end` extends to the end. Overlapping range shifts add together. Unknown episode IDs are errors.

Timing uses rational CFR FPS and centisecond-centered ASS frame boundaries, following Aegisub. SRT keeps milliseconds when no shift is applied; shifts use the same frame centering. If a shift would move either boundary before zero, both boundaries are set to zero and the line is retained. There is no out-of-bounds setting.

`[subtitles.delete]` removes lines whose starting frame is in the specified half-open range **after shifting**. `[subtitles.add]` inserts complete ASS Dialogue lines or SRT blocks at their final timestamps. Operations run in the fixed order shift, delete, add. Each subtitle ID maps to a list; `episodes` limits an item to those episodes and may be omitted to apply it to all episodes:

```toml
[subtitles]
US = '{raw}/US/{ep}.ass'

[subtitles.shift]
US = { default = 24, '01' = { frames = 24, ranges = [{ start = 2374, end = 18628, frames = 72 }] } }

[subtitles.delete]
US = [{ episodes = ['01'], start = 50000 }]

[subtitles.add]
US = [{ episodes = ['01'], lines = [
  'Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Added line',
] }]
```

Omitting `episodes` applies an item to all episodes. SRT additions take complete SRT blocks in multiline TOML strings. Additions are not shifted by earlier corrections. Added events are sorted by start time while preserving the relative order of equal times; SRT lines are renumbered.

## Chapters

`[chapters]` uses the same episode IDs as `[episodes]`. An omitted ID produces no chapters. Four value forms are supported:

- `'01' = ['AVANT', 'OPENING']` reads chapter times from the matching BDMV playlist and applies the listed names. The count must match the playlist marks.
- `'02' = []` reads BDMV times and uses generic names such as `Chapter 01`.
- `'00' = [{ frame = 0, name = 'Part One' }, { frame = 4260, name = 'Part Two' }]` creates chapters at the specified frames using the project's FPS.
- `'03' = '{raw}/chapters.txt'` passes an existing simple OGM chapter text file to MKVToolNix; XML is also supported.

Generated chapters use temporary `chapters.txt` files in the simple OGM text format. The native MPLS parser finds the clip in `BDMV/PLAYLIST`, anchors marks to the playlist IN time or an earlier first mark, and converts 45 kHz ticks directly to milliseconds without snapping chapter times to video frames. To discard a terminal chapter mark, muxcraft estimates the video's frame count from its ffprobe video-stream duration and the configured FPS. If the stream duration is unavailable, it tries the container duration, then the playlist's `OUT - IN` duration. Marks within the last 50 frames of that duration are ignored; this does not trim the video. A difference of more than two frames between video and playlist durations produces a warning, and the durations and cutoff source are recorded in the audit report. Explicit `{ frame = ... }` chapters use the configured FPS. If matching playlists have different chapter results, muxcraft reports an ambiguity. It handles one clip and does not concatenate playlists. For an episode with multiple source clips, supply frame chapters or a TXT/XML chapter file.

For a Blu-ray containing several episodes in one H.264 M2TS, put the source and chapter interval together in `[episodes]`:

```toml
[episodes]
"01" = { source = "{bdmv}/BDMV/STREAM/00002.m2ts", start_chapter = 1, end_chapter = 6 }
"02" = { source = "{bdmv}/BDMV/STREAM/00002.m2ts", start_chapter = 6 }
```

Chapter numbers are one-based; `end_chapter` is exclusive. Omitting it uses the playlist OUT time. Muxcraft searches the sibling `BDMV/PLAYLIST` directory for single-clip MPLS files referencing the M2TS. Identical playlists are interchangeable; different chapter timings cause an ambiguity error. Each internal boundary must be within one frame of a keyframe. Audio from the shared M2TS requires `codec = 'flac'` and is extracted independently to PCM before standalone FLAC encoding. The H.264 video is copied without re-encoding. SHA-256 comparisons of H.264 picture units and decoded PCM samples must pass before the final MKV is published. The audit records the chosen playlist, selected keyframes, timing deviations, and hashes.

For cut episodes, `[chapters]` names only the marks inside that episode, and its first chapter starts at zero. PGS from the shared M2TS is selected as complete display sets, with timestamps rebased to the episode. An active display at either boundary or a display that depends on objects from before the cut causes an error. The final PGS packet hash is also compared with the extracted track.

The [chapter-cut flowchart](docs/chapter-cut-flow.svg) shows each tool and intermediate file. Its editable sources are available in [Mermaid](docs/chapter-cut-flow.mmd) and [Graphviz DOT](docs/chapter-cut-flow.dot).

## Native font collector

The collector reads only the OpenType name, OS/2, head, and fvar metadata needed to identify fonts. It examines styles used by Dialogue events and follows `\fn`, `\b`, `\i`, and `\r` override tags. Variants inside `\t` are collected conservatively. A missing ASS style logs a warning and contributes no implicit font. An explicit `\fn` in that dialogue can still contribute its font. A missing font file also logs a warning; other fonts are still attached and the batch continues. No Default, first-style, or Arial fallback is applied.

Font collection is enabled by default. For each ASS subtitle, muxcraft searches the directory containing the resolved subtitle file and every subdirectory, regardless of its name or nesting depth. It also searches the directory containing the TOML file and all its subdirectories. No font paths are needed when files follow that layout. An explicit `[subtitles.fonts]` table is optional:

```toml
[subtitles.fonts]
search_subtitle_dirs = true
search_current_dir = true
use_system_fonts = true
dirs = []
```

Each `dirs` entry is an additional recursive search root. For example, `dirs = ['{raw}']` searches all directories below `{raw}` without listing them individually. `search_current_dir` refers to the TOML file's directory, independent of where the command is run. `use_system_fonts` adds usual system directories. All four settings above are their defaults.

Fonts are matched by family, full name, or PostScript name, then weight and italic. The metadata reader supports Unicode name records; legacy fonts with only non-Unicode names may not match an ASS request. An approximate variant produces a warning. Whole TTF/OTF/TTC/OTC files are copied, including variable fonts, without glyph conversion, subsetting, creating collections, or editing internal names. Attachment names use patterns such as `Family-BoldItalic.ttf` or `Family.ttf`. Identical content is deduplicated by SHA-256; filename collisions gain a hash suffix. Selection is deterministic, and source paths appear in the audit report.

This collector does not reproduce every libass substitution rule, check glyph coverage, or select particular variable font instances.

## TMDB and audit reports

`[tmdb]` accepts `id`, `season`, `movie`, `offset`, `languages`, and the flags `write_ids`, `write_date`, `write_title`, `write_summary`, and `write_synopsis`. The binary includes a public TMDB read token as its default, so no token setup is required. Set `TMDB_TOKEN` to override it; `token_env` selects a different environment variable for the override. Series or movie metadata is reused across the batch. Titles and summaries use `TagLanguage` and `TagLanguageIETF` for each language. Episode numbering comes from the `[episodes]` ID plus `offset`. Covers, TMDB episode groups, and filenames generated from TMDB titles are not implemented because the supplied scripts do not use them.

Each MKV gets an adjacent `.audit.json` containing the plan, configuration snapshot, executed commands and exit codes, chapter source, and selected fonts. Each episode gets a unique `muxcraft-ID` workdir; existing intermediate files are never reused. Existing outputs are refused. An MKV is written to a temporary file in the output directory and published via hard link without overwriting; the output filesystem must support hard links. A failure leaves intermediates and audit data for investigation. `clean_work_dirs = true` removes only that episode's workdir after success. `continue_on_error = true` continues after an execution failure but still exits with an error. Preflight errors stop the entire batch.

## Coverage and validation

The local sample configurations preserve episode-specific differences from the original workflows. For 86, chapters use MPLS; FFMS2/VapourSynth indexing is unnecessary when the video is copied. The Frieren sample uses TMDB season 1 and episodes 29–38; verify the catalog and adjust `season` or `offset` if needed.

Tests cover grouped variables, episode order, chapter forms, variables and globs, subtitle shifts, deletions, and additions, negative shifts, ASS tags, video source options, multilingual XML, and truncated MPLS. The media test generates a two-episode batch with FFmpeg and checks a real mux, TrueHD, synthetic PGS, flags, chapters, track tags, audit reports, and copied bitstream hashes. With `MUXCRAFT_TEST_FONT`, it checks font attachments. That test requires the media tools on `PATH`. `MUXCRAFT_TEST_FONT=/path/to/font.ttf cargo test real_font` enables the local font test.

The implementation supports the scripts' CFR workflow. It does not provide VFR timecodes, glyph searching, audio/video trimming and concatenation, every encoder, or Python interpretation. Real BDMVs, disc PGS, and all TrueHD/DTS variants still need validation with representative files.

Implementation references: [mkvmerge manual](https://mkvtoolnix.download/doc/mkvmerge.html), [OpenType name table](https://learn.microsoft.com/en-us/typography/opentype/spec/name), [FontCollector](https://github.com/moi15moi/FontCollector), [TMDB authentication](https://developer.themoviedb.org/docs/authentication-application), and [MKV timestamp conversion](https://moi15moi.github.io/VideoTimestamps/reference/fps_timestamps/).
