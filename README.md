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
    (import muxcraftSource {
      inherit pkgs;
      revision = muxcraftSource.rev;
    })
  ];
}
```

Run `sudo nixos-rebuild switch` to install or update muxcraft. Nix fetches the current `main` commit when it refreshes its Git cache, so future rebuilds can pick up new commits without editing this configuration. This builds against your NixOS `pkgs` and adds FFmpeg, FLAC, MKVToolNix, and CA certificates through the package wrapper. You can also test the local package with `nix-build default.nix`.

### Release archives

Every successful push to `main` builds Linux x86_64, Windows x86_64, and macOS arm64 binaries and publishes a [GitHub Release](https://github.com/IncYami/muxcraft/releases) with a `SHA256SUMS` file. The release identifier uses the UTC build date and the source commit, such as `2026.10.12-g1a2b3c4`; `muxcraft -V` prints the same identifier. Previous releases remain available. A manual workflow run on another branch builds the same archives as workflow artifacts without publishing a release. The archives contain muxcraft only: install FFmpeg, ffprobe, FLAC, and MKVToolNix separately and make them available on `PATH` (or configure `[tools]`). The Nix package includes these tools in its runtime path.

## Build from source

Building requires Rust and Cargo. Running requires **FFmpeg, ffprobe, FLAC, and MKVToolNix** on `PATH`, or explicit paths under `[tools]`. The binary does not require Python, VapourSynth, FontCollector, or installed Rust crates on the machine where it runs. Media conversions still use the external tools; rewriting the orchestration in Rust does not make FLAC encoding faster.

```sh
cargo build --release --locked
./target/release/muxcraft my-batch.toml --mode plan --no-check
./target/release/muxcraft my-batch.toml --mode check
./target/release/muxcraft my-batch.toml
./target/release/muxcraft my-batch.toml --verbose
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

`--mode plan` prints JSON to stdout with resolved paths and track order; normally it requires existing files and unambiguous globs. `--no-check` shows a template plan when the source media is unavailable. `--mode check` validates tracks and FPS, processes subtitles in memory, and checks fonts and chapters without encoding audio or making an MKV. A run checks the batch before its first mux. The CLI prints each episode's steps, exact external commands, hash values, warnings, durations, and completion summary on stderr. TTY runs show progress bars or spinners; redirected output uses plain text. `--verbose` adds tool diagnostics. No audit or log file is created. `--episodes 01,02` selects episode IDs; the default runs all episodes in order. See the [configuration reference](docs/configuration.md) for every field and default.

The local `examples/` directory contains sample configurations but is excluded from the GitHub distribution because its media paths are machine-specific. The generic committed examples in [docs/configs](docs/configs/) run as template plans without media files. Missing ASS styles and fonts produce warnings and do not stop the batch. Ambiguous chapters remain an error.

## TOML syntax

Use single-quoted strings by default; they preserve backslashes literally. Use double quotes only when TOML escape sequences are needed. TOML bare keys are suitable for numeric episode IDs: write `01` in `[episodes]`, `[chapters]`, and per-episode correction tables. These keys remain strings, including the leading zero. Episode IDs written as values are strings, for example `episodes = ['01']`. Quote dotted keys such as `'VideoTrack.01'` inside `[mux]`, and quote any key containing spaces.

Relative paths start at the TOML directory. `{config_dir}`, `{show}`, `{ep}`, and `{episodes}` expand to the config directory, show, current episode ID, and resolved episode source. A string in `[vars]` applies to every episode. A grouped table such as `[vars.DISC_ID]` assigns values to episode IDs; references must exist and appear once. Variable references may refer to other variables; unknown and circular references are errors.

## Quick configuration example

```toml
[project]
show = 'Example Series'
out_dir = 'output'
work_dir = 'work'
output = '{show}.{ep}.mkv'
fps = '24000/1001'

[episodes]
01 = 'media/episode-01.mkv'
02 = 'media/episode-02.mkv'

[VideoTrack.01]
source = '{episodes}'
track = 0

[mux]
'VideoTrack.01' = { name = 'Video', language = 'en-US', default = true }
```

For complete examples of batch remuxing, joined clips, chapter cuts, and per-episode overrides, see [docs/configs](docs/configs/). The [configuration reference](docs/configuration.md) documents every option.

Save the example as `batch.toml` and replace the media paths with your files. Run `muxcraft batch.toml --mode plan --no-check` to inspect the configuration, then `muxcraft batch.toml --mode check` to validate the media. Run `muxcraft batch.toml` to create the MKVs.

## Tracks and subtitles

| Setting | Behavior |
| --- | --- |
| `[VideoTrack.01]`, `[VideoTrack.02]` | Copy the selected type-relative video tracks. Set `source = '{episodes}'` when video comes from the current episode; each track can also use a different path. |
| `audio`, `subtitles`, `attachments` in `[VideoTrack.01]` | `true` keeps all source tracks or attachments of that category; `false` excludes them. Each defaults to `false`. The selected video track is controlled by `track`. |
| `global_tags`, `chapters` in `[VideoTrack.01]` | `true` keeps the source file's global tags or chapters; `false` excludes them. Both default to `false`. |
| `colour = { range = 1, primaries = 1, transfer = 1, matrix = 1 }` | Sets the MKVToolNix video color metadata for the selected video track. Omit a field to leave that property as supplied by the source. Values are numeric MKVToolNix/Matroska codes. |
| `[AudioTrack.01]`, `codec = 'flac'` | FFmpeg extracts `0:a:track` first. If necessary, FFmpeg then prepares a PCM Wave64 file. The standalone `flac` executable encodes at level 8 with `--verify`. No normalization or resampling is configured. |
| `[AudioTrack.02]`, `codec = 'copy'` | FFmpeg extracts the original audio bitstream to a separate file for muxing. PCM is written as WAV. |
| `.ass` or `.srt` subtitle source | The resolved file extension selects subtitle processing. ASS is edited in Rust while preserving dialogue text, tags, and fields; SRT supports shifts, deletions, and additions and remains SRT. |
| `[pgs]` | FFmpeg extracts a PGS stream from a container to `.sup` with stream copy, then MKVToolNix muxes it. An existing `.sup` is used directly. Text subtitle processing does not apply. |

FLAC tracks receive `ENCODER` (the FLAC version) and `ENCODER_SETTINGS` (the encoder options) as Matroska track tags. Tags explicitly supplied in `[mux]` take precedence over these generated values. Copy tracks do not receive encoding tags.

The `.01`, `.02` suffixes identify output tracks; `track` is a separate **zero-based index within the source media type**. The program resolves MKVToolNix track IDs; FFmpeg selects audio by type as the scripts do. Explicit tracks follow TOML order: videos, audios, then subtitles. Audio and subtitle tracks kept by a video source follow that video's track.

Every video and audio table must declare `source`; audio tables must also declare `codec`. `[mux]` gives each track its `name`, `language`, `default`, `forced`, `hearing_impaired`, `delay_ms`, and `tags`. Every track needs a `[mux]` entry. Dotted IDs must be quoted there: `'AudioTrack.01'`. The source track tables accept `args` (individual mkvmerge arguments, without a shell). Root-level `mux_args` adds global mkvmerge arguments. Languages keep their IETF code, such as `pt-BR` or `es-419`. Track flags are set explicitly.

`audio = true` and `subtitles = true` preserve all tracks of those types from the video source, including their source names and languages. Use separate `[AudioTrack.xx]` or `[subtitles]` entries to select and configure individual tracks. `attachments = true` retains source attachments; fonts collected from ASS files are also attached when `[subtitles.fonts]` is enabled. Use either `chapters = true` for chapters embedded in one video source or `[chapters]` for that episode; combining them is rejected. `global_tags = true` retains source global tags alongside any generated TMDB tags.

Paths containing `*` or `?` are searched automatically after variable expansion. Both wildcards match within one path segment. Square brackets are literal, which is useful for release names like `[EA]`. The pattern must match exactly one file. Recursive `**` is unsupported; enumerate directories or use variables.

`[subtitles]` maps a text subtitle ID to a path or to an inline table when it needs `track`, `only`, `except`, `overrides`, or other options. The resolved `.ass` or `.srt` file extension selects its type, including when the configured source uses a glob. SRT remains SRT; muxcraft does not convert subtitle formats.

`[pgs]` maps each image subtitle ID directly to a container or `.sup` file. A string selects the first PGS stream; use `{ source = '{episodes}', track = 1 }` to select the second PGS stream from a container. `track` is zero-based among PGS streams. `only`, `except`, and `args` are available in the inline form. PGS tracks are muxed after text subtitles and use the same `[mux]` naming and language settings. Container extraction uses FFmpeg stream copy, with no automatic delay preservation. Text cleaning, shifts, deletions, and additions do not apply to PGS; use `delay_ms` in `[mux]` for timing.

`[subtitles.processing]` sets shared `clean_garbage`, `clean_extradata`, and `ycbcr_matrix` options for ASS. Set `truncate_video = { trimmed = true }` to remove cues starting at or after the selected first video track's end and clamp crossing cues to that end. `trimmed` defaults to `true` when the table is present; `false` removes any cue extending beyond the end. Set `comment = true` to write removed ASS dialogue as `Comment` events with its original timing. A crossing dialogue is retained as a comment and, when `trimmed = true`, also as a clamped dialogue; with `trimmed = false`, only the comment remains. Existing ASS comments keep their original timing and are not duplicated. `comment` defaults to `false`. SRT has no standard comment event, so this option has no effect on SRT files. It applies after shifts, deletions, and additions to ASS and SRT. Cut episodes use the copied video frame count at the configured rational FPS, joined episodes sum their source video stream durations, and normal episodes use the selected video stream duration. The cutoff is represented in milliseconds; ASS endpoints are quantized down to its centisecond precision. When the option is absent, subtitle timing is unchanged. `ycbcr_matrix = 'auto'` reads the video's color matrix and range with ffprobe. BT.709 limited range becomes `TV.709`; BT.709 full range becomes `PC.709`. An unsupported or unknown matrix becomes `None` with a warning; an unspecified range assumes limited range with a warning. You may set `None`, `TV.709`, or `PC.709` explicitly. This writes the ASS header; it does not recolor subtitle pixels. Individual subtitle tables may still set `clean`, `headers`, or `ycbcr_matrix` when needed. See [libass color matrix notes](https://github.com/libass/libass/blob/master/libass/ass_types.h).

Cleaning can remove the Aegisub project and extradata sections. Missing styles are reported and never cause an unrelated style to be retained as a font fallback.


### Episode-specific tracks

Use an episode config when a subtitle source or track set differs for one episode:

```toml
# batch.toml
[episodes]
01 = 'episodes/01.toml'
02 = 'disc/episode-02.m2ts'

[subtitles]
US = 'subtitles/{ep}.ass'
CR12 = 'subtitles/commentary.ass'
```

```toml
# episodes/01.toml
[episodes]
01 = 'disc/episode-01.m2ts'

[subtitles]
US = 'subtitles/episode-01-revised.ass'
CR12 = false

[subtitles.shift]
US = 0
```

Episode `01` inherits the other batch tracks and settings. `false` disables that subtitle for the episode, including its mux settings and subtitle edits. The older `only`, `except`, and source `overrides` options remain available for simple track inclusion rules.

### Subtitle timing and edits

`[subtitles.shift]` maps each subtitle ID to a frame count or a correction table. A batch-wide correction can use `default` and episode IDs as bare numeric keys, such as `US = { default = 24, 01 = 0 }`; an episode value replaces the default. In an episode config file, use a direct correction such as `US = { frames = 24, ranges = [{ start = 2374, end = 18628, frames = 72 }] }` without nesting it under the episode ID. Shift ranges are half-open `[start, end)` and use the starting frame before correction; omitting `end` extends to the end. Overlapping range shifts add together. Unknown episode IDs are errors.

Timing uses rational CFR FPS and centisecond-centered ASS frame boundaries, following Aegisub. SRT keeps milliseconds when no shift is applied; shifts use the same frame centering. If a shift would move either boundary before zero, both boundaries are set to zero and the line is retained. There is no out-of-bounds setting.

`[subtitles.delete]` removes lines whose starting frame is in the specified half-open range **after shifting**. `[subtitles.add]` inserts complete ASS Dialogue lines or SRT blocks at their final timestamps. Operations run in the fixed order shift, delete, add. Each subtitle ID maps to a list; `episodes` limits an item to those episodes and may be omitted to apply it to all episodes:

```toml
[subtitles]
US = '{raw}/US/{ep}.ass'

[subtitles.shift]
US = { default = 24, 01 = { frames = 24, ranges = [{ start = 2374, end = 18628, frames = 72 }] } }

[subtitles.delete]
US = [{ episodes = ['01'], start = 50000 }]

[subtitles.add]
US = [{ episodes = ['01'], lines = [
  'Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Added line',
] }]
```

Omitting `episodes` applies an item to all episodes. SRT additions take complete SRT blocks in multiline TOML strings. Additions are not shifted by earlier corrections. Added events are sorted by start time while preserving the relative order of equal times; SRT lines are renumbered.

## Chapters

`[chapters]` uses the same episode IDs as `[episodes]`. An omitted ID produces no chapters. Names-only arrays such as `01 = ['Opening', 'Ending']` read times from the matching BDMV playlist, so use them only when the source has an unambiguous MPLS. Use explicit frame entries or a TXT/XML chapter file for ordinary MKV sources. Four value forms are supported:

- `01 = ['AVANT', 'OPENING']` reads chapter times from the matching BDMV playlist and applies the listed names. The count must match the playlist marks.
- `02 = []` reads BDMV times and uses generic names such as `Chapter 01`.
- `00 = [{ frame = 0, name = 'Part One' }, { frame = 4260, name = 'Part Two' }]` creates chapters at the specified frames using the project's FPS.
- `03 = '{raw}/chapters.txt'` passes an existing simple OGM chapter text file to MKVToolNix; XML is also supported.

Generated chapters use temporary `chapters.txt` files in the simple OGM text format. The native MPLS parser finds the clip in `BDMV/PLAYLIST`, anchors marks to the playlist IN time or an earlier first mark, and converts 45 kHz ticks directly to milliseconds without snapping chapter times to video frames. To discard a terminal chapter mark, muxcraft estimates the video's frame count from its ffprobe video-stream duration and the configured FPS. If the stream duration is unavailable, it tries the container duration, then the playlist's `OUT - IN` duration. Marks within the last 50 frames of that duration are ignored; this does not trim the video. A difference of more than two frames between video and playlist durations produces a warning; durations and the cutoff source appear in the CLI. Explicit `{ frame = ... }` chapters use the configured FPS. If matching playlists have different chapter results, muxcraft reports an ambiguity. It handles one clip and does not concatenate playlists. For an episode with multiple source clips, supply frame chapters or a TXT/XML chapter file.

For a Blu-ray containing several episodes in one H.264 M2TS, put the source and chapter interval together in `[episodes]`:

```toml
[episodes]
01 = { source = '{bdmv}/BDMV/STREAM/00002.m2ts', start_chapter = 1, end_chapter = 6 }
02 = { source = '{bdmv}/BDMV/STREAM/00002.m2ts', start_chapter = 6 }
```

Chapter numbers are one-based; `end_chapter` is exclusive. Omitting it uses the playlist OUT time. Muxcraft searches the sibling `BDMV/PLAYLIST` directory for single-clip MPLS files referencing the M2TS. Identical playlists are interchangeable; different chapter timings cause an ambiguity error. Each internal boundary must be within one frame of a keyframe. Audio from the shared M2TS requires `codec = 'flac'`. For each track, one FFmpeg read splits the decoded audio: a timestamp-trimmed branch writes the cut PCM WAV, while a sample-count-trimmed branch streams raw PCM to the source hash. Muxcraft independently reads the saved WAV and compares its PCM hash and byte count with that source hash before standalone FLAC encoding. Internal chapter cuts must contain the full requested audio sample range. At playlist OUT, the source can end a few samples before the requested time; the CLI reports both the requested sample endpoint and the decoded endpoint. The H.264 video is copied without re-encoding for the calculated frame count, excluding the next episode's keyframe at an internal boundary. A single FFmpeg tee invocation writes the cut MKV and hashes source H.264 picture units; muxcraft then independently reads and hashes the saved cut MKV. The copied frame count and SHA-256 comparisons of H.264 picture units must pass before muxing. The CLI shows the chosen playlist, frame count, sample intervals, and source/cut hashes. This verifies the extracted interval against the chosen cut boundaries; it cannot establish that the playlist chapters themselves identify the intended episode.

For cut episodes, `[chapters]` names only the marks inside that episode, and its first chapter starts at zero. PGS from the shared M2TS is selected as complete display sets, with timestamps rebased to the episode. An active display at either boundary or a display that depends on objects from before the cut causes an error. Muxcraft compares SHA-256 hashes of the selected source PGS segment types and payloads with the saved cut SUP, checks the segment count, and verifies each rebased timestamp. No track is rehashed from the final MKV.

The operation flowcharts show each tool and intermediate file:

| Source mode | View | Editable source |
| --- | --- | --- |
| One file, no cut | [Single-source mux](docs/single-source-flow.svg) | [Mermaid](docs/single-source-flow.mmd) · [Graphviz DOT](docs/single-source-flow.dot) |
| Ordered clips joined into one episode | [Multi-part join](docs/multi-part-flow.svg) | [Mermaid](docs/multi-part-flow.mmd) · [Graphviz DOT](docs/multi-part-flow.dot) |
| One M2TS cut at chapter boundaries | [Chapter cut](docs/chapter-cut-flow.svg) | [Mermaid](docs/chapter-cut-flow.mmd) · [Graphviz DOT](docs/chapter-cut-flow.dot) |

The video, audio, and PGS hash comparisons shown in the chapter-cut flow apply to that mode. For any episode, an execution failure prints the error to the CLI and retains its workdir; `continue_on_error` controls whether the batch proceeds. Preflight failures stop the batch before processing.

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

Fonts are matched by family, full name, or PostScript name, then weight and italic. The metadata reader supports Unicode name records; legacy fonts with only non-Unicode names may not match an ASS request. An approximate variant produces a warning. Whole TTF/OTF/TTC/OTC files are copied, including variable fonts, without glyph conversion, subsetting, creating collections, or editing internal names. Attachment names use patterns such as `Family-BoldItalic.ttf` or `Family.ttf`. Identical content is deduplicated by SHA-256; filename collisions gain a hash suffix. Selection is deterministic, and source paths appear in the CLI.

This collector does not reproduce every libass substitution rule, check glyph coverage, or select particular variable font instances.

## TMDB and output files

`[tmdb]` accepts `id`, `season`, `movie`, `offset`, `languages`, and the flags `write_ids`, `write_date`, `write_title`, `write_summary`, and `write_synopsis`. The binary includes a public TMDB read token as its default, so no token setup is required. Set `TMDB_TOKEN` to override it; `token_env` selects a different environment variable for the override. Series or movie metadata is reused across the batch. Titles and summaries use `TagLanguage` and `TagLanguageIETF` for each language. Episode numbering comes from the `[episodes]` ID plus `offset`. Covers, TMDB episode groups, and filenames generated from TMDB titles are not implemented because the supplied scripts do not use them.

Each episode gets a workdir named by its ID, such as `01`; existing intermediate files are never reused. Cut video files keep the source stem with an MKV extension, such as `00002.mkv` for `00002.m2ts`. Existing outputs are refused. An MKV is written under a hidden name in the output directory, such as `.Example.01.mkv`, and published via hard link without overwriting; the output filesystem must support hard links. A failure leaves intermediates for investigation. `clean_work_dirs = true` removes only that episode's workdir after success. `continue_on_error = true` continues after an execution failure but still exits with an error. Preflight errors stop the entire batch.

## Coverage and validation

The committed generic examples exercise these configuration forms without relying on personal media paths.

Tests cover grouped variables, episode order, chapter forms, variables and globs, subtitle shifts, deletions, and additions, negative shifts, ASS tags, video source options, multilingual XML, and truncated MPLS. The media test generates a two-episode batch with FFmpeg and checks a real mux, TrueHD, synthetic PGS, flags, chapters, track tags, CLI output, and copied bitstream hashes. With `MUXCRAFT_TEST_FONT`, it checks font attachments. That test requires the media tools on `PATH`. `MUXCRAFT_TEST_FONT=/path/to/font.ttf cargo test real_font` enables the local font test.

The implementation supports CFR timing, joins ordered source clips, and can cut an M2TS at chapter boundaries. It does not provide VFR timecodes, glyph searching, arbitrary audio/video edits, every encoder, or Python interpretation. Real BDMVs, disc PGS, and all TrueHD/DTS variants still need validation with representative files.

Implementation references: [mkvmerge manual](https://mkvtoolnix.download/doc/mkvmerge.html), [OpenType name table](https://learn.microsoft.com/en-us/typography/opentype/spec/name), [FontCollector](https://github.com/moi15moi/FontCollector), [TMDB authentication](https://developer.themoviedb.org/docs/authentication-application), and [MKV timestamp conversion](https://moi15moi.github.io/VideoTimestamps/reference/fps_timestamps/).
