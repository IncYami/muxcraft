# Configuration reference

Muxcraft reads TOML. Unknown fields in structured tables are errors. Strings are single-quoted by convention; double-quoted strings are useful when TOML escapes are needed. Numeric episode keys may be bare (`01`) in `[episodes]`, `[chapters]`, and episode correction tables; TOML exposes them as string keys, preserving leading zeroes. IDs used as values remain strings (`episodes = ['01']`). Quote dotted keys in `[mux]` and keys containing spaces.

Relative paths are resolved from the main configuration file's directory, including paths in episode overlay files. `{config_dir}`, `{show}`, `{ep}`, and `{episodes}` expand to the config directory, project show, current episode ID, and episode source. In a multipart episode, `{episodes}` is the joined work file. Variables may reference other variables; unknown, circular, and reserved variable names are errors.

Templates that pass `--mode plan --no-check` are in [configs](configs/): [simple batch](configs/simple-batch.toml), [joined clips](configs/joined-parts.toml), [chapter cut](configs/chapter-cut.toml), and [per-episode overlay](configs/per-episode-overlay.toml). Replace their placeholder media paths before using `--mode check` or running a mux.

## Root tables

| Table/key | Type | Default | Meaning and constraints |
| --- | --- | --- | --- |
| `[project].show` | string | required | Show name; available as `{show}`. |
| `[project].out_dir` | path string | required | Output directory, relative to config directory when not absolute. |
| `[project].work_dir` | path string | required | Work directory; one subdirectory per episode ID. |
| `[project].output` | string | `'{show}.{ep}.mkv'` | Output filename template. Must resolve to a basename ending in `.mkv`. |
| `[project].title` | string | empty | Container title template. |
| `[project].fps` | string | `'24000/1001'` | Positive rational CFR rate accepted by the FPS parser. |
| `[project].audit` | boolean | `true` | Write an episode audit JSON in the work directory and beside the output MKV. Set `false` to write neither file; terminal output remains available. |
| `[project].clean_work_dirs` | boolean | `false` | Remove an episode work directory after successful output. |
| `[project].continue_on_error` | boolean | `false` | Continue after an episode execution failure; batch still exits with an error. |
| `[tools].ffmpeg` | string | `'ffmpeg'` | FFmpeg executable. |
| `[tools].ffprobe` | string | `'ffprobe'` | ffprobe executable. |
| `[tools].flac` | string | `'flac'` | FLAC encoder executable. |
| `[tools].mkvmerge` | string | `'mkvmerge'` | MKVToolNix muxer executable. |
| `[vars]` | string or table | empty | String values are global variables. `{ from = 'ep', offset = N, pad = N }` derives a padded numeric episode value; `from` must be `ep`, pad 1–10, and each result positive. A nested `[vars.NAME]` groups a string value to episode IDs. Reserved names: `config_dir`, `show`, `ep`, `episodes`, `source`. |
| `[episodes]` | string, string array, or table | required | Ordered episode map. IDs contain ASCII letters/digits, `-`, `_`; IDs must be unique. A string is one source; an array is ordered clips to join. A table requires `source`, `start_chapter`, and optional `end_chapter` to cut an M2TS at one-based chapter boundaries; end is exclusive. A `.toml` string names an episode overlay file. At least one episode is required. |
| `[chapters]` | string, string array, or table array | empty | Per-episode chapters: file path (`.txt`/`.xml`), names array (BDMV playlist times), or `{ frame, name }` entries. Referenced episode IDs must exist. Use either configured chapters or `chapters = true` on a video track, not both. |
| `mux_args` | string array | empty | Additional global mkvmerge arguments, each as one argument. |

`[tools]` values may be executable names found on `PATH` or explicit executable paths.

## Track sources and mux metadata

Every video and audio source has an ID formed from the table suffix, such as `VideoTrack.01` or `AudioTrack.02`. Every configured track must have one matching `[mux]` entry; quote dotted IDs there. Track IDs across video, audio, subtitles, and PGS must be unique. Track source paths must be nonempty. `only` and `except` episode lists are mutually interpreted as enable filters; references must name configured episode IDs.

| Track field | Type | Default | Applies to / behavior |
| --- | --- | --- | --- |
| `source` | string | required | Video, audio, subtitle, and PGS sources. Supports variables and a glob that must match exactly one file. `*` and `?` match within one path segment; `**` is unsupported and square brackets are literal. |
| `track` | nonnegative integer | `0` | Zero-based index within source type. For video/audio it selects that media type; PGS selects PGS streams; subtitle inline source selects text tracks. |
| `audio`, `subtitles`, `attachments` | boolean | `false` | Video track only: include all source tracks or attachments of that kind. |
| `global_tags`, `chapters` | boolean | `false` | Video track only: retain source global tags or embedded chapters. |
| `colour` | table | absent | Video metadata fields `range`, `primaries`, `transfer`, `matrix` are optional numeric MKVToolNix codes; at least one must be set. `range` is 0–3. A given field cannot also be supplied in `args`. |
| `codec` | string | required for audio | `flac` (encode) or `copy` (copy audio bitstream). |
| `args` | string array | `[]` | Per-track mkvmerge arguments, one item per argument. For PGS, applied to the muxed track. |
| `only`, `except` | string array | `[]` | Enable only the listed episodes, or enable all except listed episodes. IDs must exist. |
| `name`, `language` | string | required | `[mux]` fields: output track name and language/IETF language tag. |
| `default`, `forced`, `hearing_impaired` | boolean | `false` | `[mux]` track flags. |
| `delay_ms` | integer | `0` | `[mux]` track delay in milliseconds. |
| `tags` | string map | `{}` | `[mux]` extra Matroska track tags. |

Video sources use `[VideoTrack.N]`; audio uses `[AudioTrack.N]` and requires `codec`. Text subtitles use `[subtitles]` with each ID mapped to a source string or inline table. PGS uses `[pgs]`, each ID mapped to a container/`.sup` source or inline table. Subtitle and PGS IDs cannot contain dots. For inline text subtitles, fields are `source`, `track`, `only`, `except`, `overrides`, `headers`, `clean`, `ycbcr_matrix`, and `args`; `overrides` maps episode ID to `{ source = '...' }`. Inline PGS fields are `source`, `track`, `only`, `except`, `args`.

Each track has its metadata under `[mux]`:

```toml
[mux]
'VideoTrack.01' = { name = 'Video', language = 'en-US', default = true }
```

## Subtitle processing

`[subtitles.fonts]` defaults to enabled font collection with current-directory, subtitle-directory, and system font searches enabled; `dirs` is empty. All fields are optional: `enabled`, `dirs` (string array), `use_system_fonts`, `search_current_dir`, `search_subtitle_dirs`.

`[subtitles.processing]` supports `clean_garbage` (false), `clean_extradata` (false), `ycbcr_matrix` (absent), and `truncate_video` (absent). Its `truncate_video` table has `trimmed` (true) and `comment` (false). When set, cues starting at or after the video end are removed; crossing cues are clamped when `trimmed = true`, otherwise any cue extending past the end is removed. With `comment = true`, removed ASS dialogue is kept as a timed Comment event; crossing dialogue can also remain as a clamped dialogue. SRT has no comment event. Truncation runs after shifts, deletions, and additions and uses selected video duration; it does not trim video.

For each subtitle ID, `[subtitles.shift]` accepts an integer frame shift or a correction table. A batch correction table requires `default` and may specify episode IDs as keys; episode corrections replace the default. An episode overlay can provide `{ frames = N, ranges = [...] }` directly. Each range has required `start` (nonnegative frame), optional `end` (exclusive and greater than start), and `frames` (shift, default 0). Shifts use rational CFR frame timing. Overlapping range shifts add.

`[subtitles.delete]` maps IDs to arrays of `{ episodes = [], start, end }`: start is required, nonnegative, `end` optional, and ranges are half-open; omitted episodes applies to all episodes. `[subtitles.add]` maps IDs to arrays of `{ episodes = [], lines }`, where lines is a nonempty string array of complete ASS Dialogue lines or SRT blocks. Edits run in order: shift, delete, add. Added lines use final timestamps.

For inline subtitle tracks, `headers` is a string map, `clean` accepts `garbage` and/or `extradata`, and `ycbcr_matrix` accepts `auto`, `None`, `TV.601`, `PC.601`, `TV.709`, `PC.709`, `TV.240M`, `PC.240M`, `TV.FCC`, or `PC.FCC`. Shared processing defaults apply when omitted; per-track values configure that subtitle. `overrides` replaces its source for an episode. `only` and `except` remain supported for track filtering.

## Episode overlays

An `[episodes]` value ending in `.toml` loads an overlay. That file must define exactly the same one episode ID and its actual non-TOML source; overlays cannot include other overlays. Its settings are merged over the batch settings for that episode. Tables merge recursively; subtitle shift entries merge by ID. A boolean `false` under `VideoTrack`, `AudioTrack`, `subtitles`, or `pgs` disables that inherited track and removes its mux settings for this episode. The main batch still defines any track that is enabled for other episodes. Paths in both files resolve relative to the main batch configuration directory.

## TMDB

`[tmdb]` is optional. `id` is required (`u32`); `season` defaults to `1`, `movie` to `false`, `offset` to `0`, `languages` to `['en-US']`, and `token_env` to `'TMDB_TOKEN'`. `write_ids`, `write_date`, `write_title`, `write_summary`, and `write_synopsis` all default to `true`. An empty languages list is invalid. The `TMDB_TOKEN` environment variable overrides the built-in read token; `token_env` changes the variable name.
