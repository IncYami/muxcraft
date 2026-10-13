use anyhow::{Context, Result, bail, ensure};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

fn yes() -> bool {
    true
}
fn output() -> String {
    "{show}.{ep}.mkv".into()
}
fn tools(s: &str) -> String {
    s.into()
}
fn fps() -> String {
    "24000/1001".into()
}
fn is_zero(value: &i64) -> bool {
    *value == 0
}

#[derive(Debug, Serialize, Clone)]
pub struct Config {
    pub project: Project,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    #[serde(default)]
    pub tools: Tools,
    pub episodes: Vec<Episode>,
    pub video: Vec<Track>,
    #[serde(default)]
    pub audio: Vec<Track>,
    #[serde(default)]
    pub subtitles: Vec<Track>,
    #[serde(default)]
    pub pgs: Vec<Track>,
    #[serde(default)]
    pub fonts: FontConfig,
    pub tmdb: Option<Tmdb>,
    #[serde(default)]
    pub mux_args: Vec<String>,
    #[serde(skip)]
    pub episode_configs: BTreeMap<String, Box<Config>>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    project: Project,
    #[serde(default)]
    vars: BTreeMap<String, Variable>,
    #[serde(default)]
    tools: Tools,
    episodes: IndexMap<String, RawEpisode>,
    #[serde(default)]
    chapters: BTreeMap<String, ChapterSpec>,
    #[serde(rename = "VideoTrack", default)]
    video_track: IndexMap<String, RawVideo>,
    #[serde(rename = "AudioTrack", default)]
    audio_track: IndexMap<String, RawAudio>,
    #[serde(default)]
    subtitles: Option<toml::Table>,
    #[serde(default)]
    pgs: IndexMap<String, SourceOrPgs>,
    #[serde(default)]
    mux: IndexMap<String, MuxSettings>,
    tmdb: Option<Tmdb>,
    #[serde(default)]
    mux_args: Vec<String>,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SourceOrTrack {
    Source(String),
    Track(Box<RawSubtitle>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SourceOrPgs {
    Source(String),
    Track(RawPgs),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPgs {
    source: String,
    #[serde(default)]
    track: usize,
    #[serde(default)]
    only: Vec<String>,
    #[serde(default)]
    except: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVideo {
    source: String,
    #[serde(default)]
    track: usize,
    #[serde(default)]
    audio: bool,
    #[serde(default)]
    subtitles: bool,
    #[serde(default)]
    attachments: bool,
    #[serde(default)]
    global_tags: bool,
    #[serde(default)]
    chapters: bool,
    colour: Option<VideoColour>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    only: Vec<String>,
    #[serde(default)]
    except: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAudio {
    source: String,
    codec: String,
    #[serde(default)]
    track: usize,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    only: Vec<String>,
    #[serde(default)]
    except: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSubtitle {
    source: String,
    #[serde(default)]
    track: usize,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    only: Vec<String>,
    #[serde(default)]
    except: Vec<String>,
    #[serde(default)]
    overrides: BTreeMap<String, TrackOverride>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    clean: Vec<String>,
    ycbcr_matrix: Option<String>,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct VideoColour {
    pub range: Option<u8>,
    pub primaries: Option<u8>,
    pub transfer: Option<u8>,
    pub matrix: Option<u8>,
}

impl RawVideo {
    fn into_track(self, id: String) -> Track {
        let mut track = Track::new(id, self.source);
        track.track = self.track;
        track.audio = self.audio;
        track.subtitles = self.subtitles;
        track.attachments = self.attachments;
        track.global_tags = self.global_tags;
        track.chapters = self.chapters;
        track.colour = self.colour;
        track.args = self.args;
        track.only = self.only;
        track.except = self.except;
        track
    }
}
impl RawAudio {
    fn into_track(self, id: String) -> Track {
        let mut track = Track::new(id, self.source);
        track.codec = self.codec;
        track.track = self.track;
        track.args = self.args;
        track.only = self.only;
        track.except = self.except;
        track
    }
}
impl RawSubtitle {
    fn into_track(self, id: String) -> Track {
        let mut track = Track::new(id, self.source);
        track.track = self.track;
        track.kind = self.kind;
        track.only = self.only;
        track.except = self.except;
        track.overrides = self.overrides;
        track.headers = self.headers;
        track.clean = self.clean;
        track.ycbcr_matrix = self.ycbcr_matrix;
        track.args = self.args;
        track
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MuxSettings {
    name: String,
    language: String,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    forced: bool,
    #[serde(default)]
    hearing_impaired: bool,
    #[serde(default)]
    delay_ms: i64,
    #[serde(default)]
    tags: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct SubtitleProcessing {
    clean_styles: bool,
    clean_garbage: bool,
    clean_extradata: bool,
    clean_comments: bool,
    ycbcr_matrix: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum SubtitleShift {
    Frames(i64),
    Table(toml::Table),
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum SubtitleShiftValue {
    Frames(i64),
    Correction(SubtitleShiftEpisode),
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default, deny_unknown_fields)]
struct SubtitleShiftEpisode {
    frames: i64,
    ranges: Vec<SubtitleShiftRange>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct SubtitleShiftRange {
    start: i64,
    end: Option<i64>,
    frames: i64,
}

impl From<SubtitleShiftEpisode> for Correction {
    fn from(value: SubtitleShiftEpisode) -> Self {
        Self {
            shift: value.frames,
            ranges: value.ranges.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<SubtitleShiftValue> for Correction {
    fn from(value: SubtitleShiftValue) -> Self {
        match value {
            SubtitleShiftValue::Frames(frames) => Self {
                shift: frames,
                ..Self::default()
            },
            SubtitleShiftValue::Correction(correction) => correction.into(),
        }
    }
}
impl From<SubtitleShiftRange> for Range {
    fn from(value: SubtitleShiftRange) -> Self {
        Self {
            start: value.start,
            end: value.end,
            shift: value.frames,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Variable {
    Literal(String),
    Derived(DerivedVariable),
    ByValue(BTreeMap<String, Vec<String>>),
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DerivedVariable {
    from: String,
    offset: i64,
    pad: usize,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub show: String,
    pub out_dir: String,
    pub work_dir: String,
    #[serde(default = "output")]
    pub output: String,
    #[serde(default)]
    pub title: String,
    #[serde(default = "fps")]
    pub fps: String,
    #[serde(default)]
    pub clean_work_dirs: bool,
    #[serde(default)]
    pub continue_on_error: bool,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Tools {
    pub ffmpeg: String,
    pub ffprobe: String,
    pub flac: String,
    pub mkvmerge: String,
}
impl Default for Tools {
    fn default() -> Self {
        Self {
            ffmpeg: tools("ffmpeg"),
            ffprobe: tools("ffprobe"),
            flac: tools("flac"),
            mkvmerge: tools("mkvmerge"),
        }
    }
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Episode {
    pub id: String,
    pub source: EpisodeSource,
    pub cut: Option<CutSpec>,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    pub chapters: Option<ChapterSpec>,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CutSpec {
    pub start_chapter: usize,
    pub end_chapter: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCutEpisode {
    source: String,
    start_chapter: usize,
    end_chapter: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawEpisode {
    Single(String),
    Parts(Vec<String>),
    Cut(RawCutEpisode),
}

impl RawEpisode {
    fn into_source_and_cut(self) -> (EpisodeSource, Option<CutSpec>) {
        match self {
            Self::Single(source) => (EpisodeSource::Single(source), None),
            Self::Parts(parts) => (EpisodeSource::Parts(parts), None),
            Self::Cut(cut) => (
                EpisodeSource::Single(cut.source),
                Some(CutSpec {
                    start_chapter: cut.start_chapter,
                    end_chapter: cut.end_chapter,
                }),
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CutPlan {
    pub playlist: Option<PathBuf>,
    pub start_chapter: usize,
    pub end_chapter: Option<usize>,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum EpisodeSource {
    Single(String),
    Parts(Vec<String>),
}
impl EpisodeSource {
    fn parts(&self) -> Vec<&str> {
        match self {
            Self::Single(path) => vec![path],
            Self::Parts(paths) => paths.iter().map(String::as_str).collect(),
        }
    }
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum ChapterSpec {
    File(String),
    Names(Vec<String>),
    Frames(Vec<ChapterEntry>),
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ChapterEntry {
    pub frame: i64,
    pub name: String,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Track {
    #[serde(default)]
    pub id: String,
    pub source: String,
    #[serde(default)]
    pub track: usize,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub subtitles: bool,
    #[serde(default)]
    pub attachments: bool,
    #[serde(default)]
    pub global_tags: bool,
    #[serde(default)]
    pub chapters: bool,
    pub colour: Option<VideoColour>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub forced: bool,
    #[serde(default)]
    pub hearing_impaired: bool,
    #[serde(default)]
    pub delay_ms: i64,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    #[serde(default)]
    pub codec: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub except: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, TrackOverride>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clean: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shift_table: Option<ShiftTable>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shift_frames: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ycbcr_matrix: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deletes: Vec<DeleteRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additions: Vec<AddLines>,
}
impl Track {
    fn new(id: String, source: String) -> Self {
        Self {
            id,
            source,
            track: 0,
            audio: false,
            subtitles: false,
            attachments: false,
            global_tags: false,
            chapters: false,
            colour: None,
            name: String::new(),
            language: String::new(),
            default: false,
            forced: false,
            hearing_impaired: false,
            delay_ms: 0,
            args: Vec::new(),
            tags: BTreeMap::new(),
            codec: String::new(),
            kind: String::new(),
            only: Vec::new(),
            except: Vec::new(),
            overrides: BTreeMap::new(),
            headers: BTreeMap::new(),
            clean: Vec::new(),
            shift_table: None,
            shift_frames: 0,
            ycbcr_matrix: None,
            deletes: Vec::new(),
            additions: Vec::new(),
        }
    }
}
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TrackOverride {
    pub source: Option<String>,
}
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ShiftTable {
    pub default: Correction,
    pub episodes: BTreeMap<String, Correction>,
}
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Correction {
    pub shift: i64,
    pub ranges: Vec<Range>,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub start: i64,
    pub end: Option<i64>,
    #[serde(default)]
    pub shift: i64,
}
impl Range {
    pub fn contains(&self, frame: i64) -> bool {
        frame >= self.start && self.end.is_none_or(|end| frame < end)
    }
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DeleteRange {
    #[serde(default)]
    pub episodes: Vec<String>,
    pub start: i64,
    pub end: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AddLines {
    #[serde(default)]
    pub episodes: Vec<String>,
    pub lines: Vec<String>,
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct FontConfig {
    pub enabled: bool,
    pub dirs: Vec<String>,
    pub use_system_fonts: bool,
    pub search_current_dir: bool,
    pub search_subtitle_dirs: bool,
}
impl Default for FontConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dirs: vec![],
            use_system_fonts: true,
            search_current_dir: true,
            search_subtitle_dirs: true,
        }
    }
}
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Tmdb {
    pub id: u32,
    #[serde(default = "one")]
    pub season: u32,
    #[serde(default)]
    pub movie: bool,
    #[serde(default)]
    pub offset: i32,
    #[serde(default = "languages")]
    pub languages: Vec<String>,
    #[serde(default = "token_env")]
    pub token_env: String,
    #[serde(default = "yes")]
    pub write_ids: bool,
    #[serde(default = "yes")]
    pub write_date: bool,
    #[serde(default = "yes")]
    pub write_title: bool,
    #[serde(default = "yes")]
    pub write_summary: bool,
    #[serde(default = "yes")]
    pub write_synopsis: bool,
}
fn one() -> u32 {
    1
}
fn languages() -> Vec<String> {
    vec!["en-US".into()]
}
fn token_env() -> String {
    "TMDB_TOKEN".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct EpisodePlan {
    pub episode: String,
    pub source: PathBuf,
    pub sources: Vec<PathBuf>,
    pub output: PathBuf,
    pub work: PathBuf,
    pub title: String,
    pub tracks: Vec<TrackPlan>,
    pub cut: Option<CutPlan>,
    #[serde(skip)]
    pub vars: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct TrackPlan {
    pub id: String,
    pub category: String,
    pub source: PathBuf,
    pub name: String,
    pub language: String,
    pub track: usize,
}

impl Track {
    pub fn effective(&self, ep: &str) -> Self {
        let mut result = self.clone();
        if let Some(o) = self.overrides.get(ep)
            && let Some(v) = &o.source
        {
            result.source = v.clone();
        }
        result
    }
    pub fn enabled(&self, ep: &str) -> bool {
        (self.only.is_empty() || self.only.iter().any(|e| e == ep))
            && !self.except.iter().any(|e| e == ep)
    }
}

pub fn expand(input: &str, vars: &BTreeMap<String, String>) -> Result<String> {
    let re = regex::Regex::new(r"\{([A-Za-z_][A-Za-z_0-9]*)\}")?;
    let mut result = input.to_owned();
    let mut seen = BTreeSet::new();
    for _ in 0..64 {
        if !re.is_match(&result) {
            return Ok(result);
        }
        ensure!(
            seen.insert(result.clone()),
            "Circular variable reference: {input}"
        );
        let mut error = None;
        result = re
            .replace_all(&result, |c: &regex::Captures| {
                vars.get(&c[1]).cloned().unwrap_or_else(|| {
                    error = Some(c[1].to_owned());
                    c[0].to_owned()
                })
            })
            .into_owned();
        if let Some(name) = error {
            bail!("Unknown variable {{{name}}} in {input}");
        }
    }
    bail!("Variable expansion exceeded 64 levels: {input}")
}
pub fn absolute(base: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    let full = if p.is_absolute() {
        p.to_owned()
    } else {
        base.join(p)
    };
    let mut result = PathBuf::new();
    for component in full.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component.as_os_str()),
        }
    }
    result
}

// Literal brackets in release names are common. Only * and ? are wildcards.
pub fn resolve(
    base: &Path,
    input: &str,
    vars: &BTreeMap<String, String>,
    check: bool,
) -> Result<PathBuf> {
    let expanded = expand(input, vars)?;
    let path = absolute(base, &expanded);
    if !check {
        return Ok(path);
    }
    if !expanded.contains(['*', '?']) {
        ensure!(path.is_file(), "File not found: {}", path.display());
        return path.canonicalize().map_err(Into::into);
    }
    let mut root = PathBuf::new();
    for component in path.components() {
        if component.as_os_str().to_string_lossy().contains(['*', '?']) {
            break;
        }
        root.push(component.as_os_str());
    }
    let pattern = format!(
        "^{}$",
        regex::escape(&path.to_string_lossy())
            .replace(r"\*", r"[^/\\]*")
            .replace(r"\?", r"[^/\\]")
    );
    let re = regex::Regex::new(&pattern)?;
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(&root) {
        let entry = entry.with_context(|| format!("Searching in {}", root.display()))?;
        if entry.file_type().is_file() && re.is_match(&entry.path().to_string_lossy()) {
            paths.push(entry.into_path());
        }
    }
    paths.sort();
    ensure!(
        paths.len() == 1,
        "Glob must match exactly one file: {expanded}; found {}: {paths:?}",
        paths.len()
    );
    paths.remove(0).canonicalize().map_err(Into::into)
}
impl Config {
    pub fn for_episode(&self, id: &str) -> &Config {
        self.episode_configs
            .get(id)
            .map(Box::as_ref)
            .unwrap_or(self)
    }
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut root: toml::Value = toml::from_str(&content).context("Invalid TOML")?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let episodes = root
            .get("episodes")
            .and_then(toml::Value::as_table)
            .context("Missing [episodes]")?
            .clone();
        let mut expanded_episodes = toml::map::Map::new();
        let mut overlays = BTreeMap::new();
        for (id, source) in &episodes {
            if let Some(file) = source
                .as_str()
                .filter(|s| s.to_ascii_lowercase().ends_with(".toml"))
            {
                let include_path = if Path::new(file).is_absolute() {
                    PathBuf::from(file)
                } else {
                    base.join(file)
                };
                let include_text = std::fs::read_to_string(&include_path).with_context(|| {
                    format!(
                        "Reading episode config for {id} at {}",
                        include_path.display()
                    )
                })?;
                let overlay: toml::Value = toml::from_str(&include_text).with_context(|| {
                    format!("Invalid episode config {}", include_path.display())
                })?;
                let overlay_episodes = overlay
                    .get("episodes")
                    .and_then(toml::Value::as_table)
                    .with_context(|| {
                        format!("{} needs an [episodes] table", include_path.display())
                    })?;
                ensure!(
                    overlay_episodes.len() == 1 && overlay_episodes.contains_key(id),
                    "{} must define exactly the referenced episode {id}",
                    include_path.display()
                );
                let actual = overlay_episodes[id].clone();
                ensure!(
                    !actual
                        .as_str()
                        .is_some_and(|s| s.to_ascii_lowercase().ends_with(".toml")),
                    "{} cannot include another episode config",
                    include_path.display()
                );
                expanded_episodes.insert(id.clone(), actual);
                overlays.insert(id.clone(), (overlay, include_path));
            } else {
                expanded_episodes.insert(id.clone(), source.clone());
            }
        }
        root.as_table_mut()
            .unwrap()
            .insert("episodes".into(), toml::Value::Table(expanded_episodes));
        let mut configs = BTreeMap::new();
        for (id, (overlay, include_path)) in overlays {
            let mut per_episode = root.clone();
            merge_config(&mut per_episode, &overlay)?;
            if let Some(table) = per_episode
                .get_mut("episodes")
                .and_then(toml::Value::as_table_mut)
            {
                table.insert(id.clone(), root["episodes"][&id].clone());
            }
            disable_episode_tracks(&mut per_episode, &id, &root)?;
            configs.insert(
                id.clone(),
                Box::new(Self::load_value(per_episode).with_context(|| {
                    format!("Episode {id} config from {}", include_path.display())
                })?),
            );
        }
        let mut cfg = Self::load_value(root)?;
        cfg.episode_configs = configs;
        Ok(cfg)
    }

    fn load_value(value: toml::Value) -> Result<Self> {
        let mut raw: RawConfig = value.try_into().context("Invalid TOML configuration")?;
        let ids: BTreeSet<_> = raw.episodes.keys().cloned().collect();
        let mut vars = BTreeMap::new();
        let mut grouped = BTreeMap::new();
        for (name, value) in raw.vars {
            ensure!(
                !["config_dir", "show", "ep", "episodes", "source"].contains(&name.as_str()),
                "Reserved variable name: {name}"
            );
            match value {
                Variable::Literal(value) => {
                    vars.insert(name, value);
                }
                Variable::Derived(spec) => {
                    ensure!(spec.from == "ep", "Variable {name} can only derive from ep");
                    ensure!(
                        (1..=10).contains(&spec.pad),
                        "Variable {name} pad must be between 1 and 10"
                    );
                    let mut assignments = BTreeMap::new();
                    for id in &ids {
                        let number = id.parse::<i64>().with_context(|| {
                            format!("Variable {name} needs a numeric episode ID: {id}")
                        })?;
                        let shifted = number.checked_add(spec.offset).with_context(|| {
                            format!("Variable {name} overflows for episode {id}")
                        })?;
                        ensure!(
                            shifted > 0,
                            "Variable {name} must be positive for episode {id}"
                        );
                        assignments
                            .insert(id.clone(), format!("{shifted:0width$}", width = spec.pad));
                    }
                    grouped.insert(name, assignments);
                }
                Variable::ByValue(groups) => {
                    let mut assignments = BTreeMap::new();
                    for (value, episodes) in groups {
                        for id in episodes {
                            ensure!(
                                ids.contains(&id),
                                "Variable {name} references unknown episode {id}"
                            );
                            ensure!(
                                assignments.insert(id.clone(), value.clone()).is_none(),
                                "Variable {name} assigns episode {id} more than once"
                            );
                        }
                    }
                    grouped.insert(name, assignments);
                }
            }
        }
        for id in raw.chapters.keys() {
            ensure!(ids.contains(id), "Chapters reference unknown episode {id}");
        }
        let mut chapters = raw.chapters;
        let episodes = raw
            .episodes
            .into_iter()
            .map(|(id, raw_episode)| {
                let (source, cut) = raw_episode.into_source_and_cut();
                Episode {
                    vars: grouped
                        .iter()
                        .filter_map(|(name, values)| {
                            values.get(&id).map(|v| (name.clone(), v.clone()))
                        })
                        .collect(),
                    chapters: chapters.remove(&id),
                    id,
                    source,
                    cut,
                }
            })
            .collect();
        let mut video = Vec::new();
        for (number, spec) in raw.video_track {
            ensure!(
                number.chars().all(|c| c.is_ascii_digit()),
                "VideoTrack key must be numeric: {number}"
            );
            let track = spec.into_track(format!("VideoTrack.{number}"));
            ensure!(
                !track.source.trim().is_empty(),
                "VideoTrack.{number} has an empty source"
            );
            video.push(track);
        }
        let mut audio = Vec::new();
        for (number, spec) in raw.audio_track {
            ensure!(
                number.chars().all(|c| c.is_ascii_digit()),
                "AudioTrack key must be numeric: {number}"
            );
            let track = spec.into_track(format!("AudioTrack.{number}"));
            audio.push(track);
        }
        let mut subtitles = Vec::new();
        let mut fonts = FontConfig::default();
        if let Some(mut entries) = raw.subtitles {
            fonts = entries
                .remove("fonts")
                .map(toml::Value::try_into)
                .transpose()?
                .unwrap_or_default();
            let processing: SubtitleProcessing = entries
                .remove("processing")
                .map(toml::Value::try_into)
                .transpose()?
                .unwrap_or_default();
            let shift_entries: toml::Table = entries
                .remove("shift")
                .map(toml::Value::try_into)
                .transpose()?
                .unwrap_or_default();
            let delete_entries: toml::Table = entries
                .remove("delete")
                .map(toml::Value::try_into)
                .transpose()?
                .unwrap_or_default();
            let add_entries: toml::Table = entries
                .remove("add")
                .map(toml::Value::try_into)
                .transpose()?
                .unwrap_or_default();
            for (key, value) in entries {
                ensure!(
                    !key.contains('.'),
                    "Subtitle ID must not contain a dot: {key}"
                );
                let mut track = match value.try_into::<SourceOrTrack>()? {
                    SourceOrTrack::Source(source) => Track::new(key, source),
                    SourceOrTrack::Track(track) => (*track).into_track(key),
                };
                let is_plain = track.kind == "srt"
                    || (track.kind.is_empty()
                        && track.source.to_ascii_lowercase().ends_with(".srt"));
                if !is_plain {
                    for (enabled, operation) in [
                        (processing.clean_styles, "styles"),
                        (processing.clean_garbage, "garbage"),
                        (processing.clean_extradata, "extradata"),
                        (processing.clean_comments, "comments"),
                    ] {
                        if enabled && !track.clean.iter().any(|op| op == operation) {
                            track.clean.push(operation.into());
                        }
                    }
                    if track.ycbcr_matrix.is_none() {
                        track.ycbcr_matrix = processing.ycbcr_matrix.clone();
                    }
                }
                subtitles.push(track);
            }
            for (key, value) in shift_entries {
                let track = subtitles
                    .iter_mut()
                    .find(|t| t.id == key)
                    .with_context(|| format!("Shift references unknown subtitle {key}"))?;
                match value.try_into::<SubtitleShift>()? {
                    SubtitleShift::Frames(frames) => track.shift_frames = frames,
                    SubtitleShift::Table(mut table) => {
                        if table.contains_key("frames") || table.contains_key("ranges") {
                            let correction: SubtitleShiftEpisode =
                                toml::Value::Table(table).try_into()?;
                            track.shift_table = Some(ShiftTable {
                                default: correction.into(),
                                episodes: BTreeMap::new(),
                            });
                            continue;
                        }
                        let default: SubtitleShiftValue = table
                            .remove("default")
                            .with_context(|| format!("Shift {key} needs a default value"))?
                            .try_into()?;
                        let episodes = table
                            .into_iter()
                            .map(|(episode, value)| {
                                ensure!(
                                    ids.contains(&episode),
                                    "Shift {key} references unknown episode {episode}"
                                );
                                let correction: SubtitleShiftValue = value.try_into()?;
                                Ok((episode, correction.into()))
                            })
                            .collect::<Result<BTreeMap<_, _>>>()?;
                        track.shift_table = Some(ShiftTable {
                            default: default.into(),
                            episodes,
                        });
                    }
                }
            }
            for (key, value) in delete_entries {
                let track = subtitles
                    .iter_mut()
                    .find(|t| t.id == key)
                    .with_context(|| format!("Delete references unknown subtitle {key}"))?;
                track.deletes = value.try_into()?;
            }
            for (key, value) in add_entries {
                let track = subtitles
                    .iter_mut()
                    .find(|t| t.id == key)
                    .with_context(|| format!("Add references unknown subtitle {key}"))?;
                track.additions = value.try_into()?;
            }
        }
        let mut pgs = Vec::new();
        for (id, spec) in raw.pgs {
            ensure!(!id.contains('.'), "PGS ID must not contain a dot: {id}");
            let mut track = match spec {
                SourceOrPgs::Source(source) => Track::new(id, source),
                SourceOrPgs::Track(spec) => {
                    let mut track = Track::new(id, spec.source);
                    track.track = spec.track;
                    track.only = spec.only;
                    track.except = spec.except;
                    track.args = spec.args;
                    track
                }
            };
            ensure!(
                !track.source.trim().is_empty(),
                "PGS {} has an empty source",
                track.id
            );
            track.kind = "pgs".into();
            pgs.push(track);
        }
        let mut known = BTreeSet::new();
        for track in video
            .iter_mut()
            .chain(&mut audio)
            .chain(&mut subtitles)
            .chain(&mut pgs)
        {
            ensure!(
                known.insert(track.id.clone()),
                "Duplicate track ID: {}",
                track.id
            );
            let settings = raw
                .mux
                .shift_remove(&track.id)
                .with_context(|| format!("[mux] is missing track {}", track.id))?;
            track.name = settings.name;
            track.language = settings.language;
            track.default = settings.default;
            track.forced = settings.forced;
            track.hearing_impaired = settings.hearing_impaired;
            track.delay_ms = settings.delay_ms;
            track.tags = settings.tags;
        }
        ensure!(
            raw.mux.is_empty(),
            "[mux] references unknown tracks: {:?}",
            raw.mux.keys().collect::<Vec<_>>()
        );
        let cfg = Self {
            project: raw.project,
            vars,
            tools: raw.tools,
            episodes,
            video,
            audio,
            subtitles,
            pgs,
            fonts,
            tmdb: raw.tmdb,
            mux_args: raw.mux_args,
            episode_configs: BTreeMap::new(),
        };
        cfg.validate()?;
        Ok(cfg)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.episodes.is_empty(), "Batch has no episodes");
        crate::subtitle::Fps::parse(&self.project.fps)?;
        let mut ids = BTreeSet::new();
        for ep in &self.episodes {
            ensure!(
                !ep.id.is_empty()
                    && ep
                        .id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "Invalid episode ID: {}",
                ep.id
            );
            ensure!(ids.insert(&ep.id), "Duplicate episode: {}", ep.id);
            let parts = ep.source.parts();
            ensure!(!parts.is_empty(), "Episode {} has no source files", ep.id);
            ensure!(
                parts.iter().all(|p| !p.trim().is_empty()),
                "Episode {} has an empty source path",
                ep.id
            );
            if let Some(ChapterSpec::File(path)) = &ep.chapters {
                ensure!(!path.is_empty(), "Empty chapter path for episode {}", ep.id);
                ensure!(
                    ["txt", "xml"].contains(
                        &Path::new(path)
                            .extension()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_lowercase()
                            .as_str()
                    ),
                    "Chapter file for episode {} must be TXT or XML",
                    ep.id
                );
            }
        }
        for ep in &self.episodes {
            let Some(cut) = &ep.cut else { continue };
            let id = &ep.id;
            ensure!(cut.start_chapter > 0, "Cut {id} starts before chapter 1");
            ensure!(
                cut.end_chapter.is_none_or(|end| end > cut.start_chapter),
                "Cut {id} has an invalid end chapter"
            );
        }
        let mut tracks = BTreeSet::new();
        ensure!(!self.video.is_empty(), "Batch has no video tracks");
        for t in self
            .video
            .iter()
            .chain(&self.audio)
            .chain(&self.subtitles)
            .chain(&self.pgs)
        {
            ensure!(
                !t.source.trim().is_empty(),
                "Track {} has an empty source",
                t.id
            );
            ensure!(
                !t.id.is_empty()
                    && t.id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
                    && tracks.insert(&t.id),
                "Invalid or duplicate track ID: {}",
                t.id
            );
            for key in t
                .overrides
                .keys()
                .chain(t.only.iter())
                .chain(t.except.iter())
            {
                ensure!(
                    ids.contains(key),
                    "{} references unknown episode {key}",
                    t.id
                );
            }
            for ep in &self.episodes {
                let eff = t.effective(&ep.id);
                for op in &eff.clean {
                    ensure!(
                        ["styles", "garbage", "extradata", "comments"].contains(&op.as_str()),
                        "Unknown clean operation: {op}"
                    );
                }
            }
            for deletion in &t.deletes {
                for ep in &deletion.episodes {
                    ensure!(ids.contains(ep), "Delete references unknown episode: {ep}");
                }
                ensure!(
                    deletion.start >= 0 && deletion.end.is_none_or(|end| end > deletion.start),
                    "Invalid delete range"
                );
            }
            for addition in &t.additions {
                for ep in &addition.episodes {
                    ensure!(ids.contains(ep), "Add references unknown episode: {ep}");
                }
                ensure!(!addition.lines.is_empty(), "Add has no lines: {}", t.id);
            }
        }
        for t in &self.audio {
            ensure!(
                ["flac", "copy"].contains(&t.codec.as_str()),
                "Invalid audio codec: {}",
                t.codec
            );
        }
        for t in &self.video {
            if let Some(colour) = &t.colour {
                ensure!(
                    colour.range.is_some()
                        || colour.primaries.is_some()
                        || colour.transfer.is_some()
                        || colour.matrix.is_some(),
                    "{} has an empty colour table",
                    t.id
                );
                ensure!(
                    colour.range.is_none_or(|range| range <= 3),
                    "{} has an invalid colour range",
                    t.id
                );
                for (field, flag) in [
                    (colour.range, "--colour-range"),
                    (colour.primaries, "--colour-primaries"),
                    (colour.transfer, "--colour-transfer-characteristics"),
                    (colour.matrix, "--colour-matrix"),
                ] {
                    ensure!(
                        field.is_none() || !t.args.iter().any(|arg| arg == flag),
                        "{} sets {flag} in both colour and args",
                        t.id
                    );
                }
            }
        }
        for ep in &self.episodes {
            let chapter_sources: Vec<_> = self
                .video
                .iter()
                .filter(|track| track.chapters && track.enabled(&ep.id))
                .map(|track| track.id.as_str())
                .collect();
            ensure!(
                chapter_sources.len() <= 1 && (chapter_sources.is_empty() || ep.chapters.is_none()),
                "Episode {} has multiple chapter sources; use either [chapters] or chapters = true in one VideoTrack",
                ep.id
            );
        }
        for t in &self.subtitles {
            ensure!(
                ["", "ass", "srt"].contains(&t.kind.as_str()),
                "Invalid subtitle type: {}",
                t.kind
            );
            ensure!(
                !t.source.to_ascii_lowercase().ends_with(".sup"),
                "PGS subtitle {} belongs in [pgs]",
                t.id
            );
            if let Some(matrix) = &t.ycbcr_matrix {
                ensure!(
                    [
                        "auto", "None", "TV.601", "PC.601", "TV.709", "PC.709", "TV.240M",
                        "PC.240M", "TV.FCC", "PC.FCC"
                    ]
                    .contains(&matrix.as_str()),
                    "Invalid YCbCr matrix for {}: {matrix}",
                    t.id
                );
                ensure!(
                    !t.headers.contains_key("YCbCr Matrix"),
                    "Subtitle {} sets YCbCr Matrix twice",
                    t.id
                );
            }
            let plain_srt = t.kind == "srt"
                || (t.kind.is_empty() && t.source.to_ascii_lowercase().ends_with(".srt"));
            if plain_srt {
                ensure!(
                    t.ycbcr_matrix.is_none(),
                    "Plain SRT subtitle {} cannot use ycbcr_matrix",
                    t.id
                );
            }
        }
        for table in self
            .subtitles
            .iter()
            .filter_map(|track| track.shift_table.as_ref())
        {
            for ep in table.episodes.keys() {
                ensure!(ids.contains(ep), "Shift references unknown episode: {ep}");
            }
            for c in std::iter::once(&table.default).chain(table.episodes.values()) {
                for r in &c.ranges {
                    ensure!(
                        r.start >= 0 && r.end.is_none_or(|end| end > r.start),
                        "Invalid shift range"
                    );
                }
            }
        }
        if let Some(t) = &self.tmdb {
            ensure!(!t.languages.is_empty(), "TMDB has no languages");
        }
        Ok(())
    }
    pub fn plan(&self, base: &Path, check: bool) -> Result<Vec<EpisodePlan>> {
        if self.episode_configs.is_empty() {
            return self.plan_local(base, check);
        }
        let mut plans = Vec::new();
        let mut outputs = BTreeSet::new();
        let mut works = BTreeSet::new();
        for ep in &self.episodes {
            let effective = self.for_episode(&ep.id);
            let mut isolated = effective.clone();
            isolated.episode_configs.clear();
            isolated.episodes.retain(|e| e.id == ep.id);
            let mut current = isolated.plan_local(base, check)?;
            for plan in &current {
                ensure!(
                    outputs.insert(plan.output.clone()),
                    "Two episodes produce {}",
                    plan.output.display()
                );
                ensure!(works.insert(plan.work.clone()), "Duplicate workdir");
            }
            plans.append(&mut current);
        }
        Ok(plans)
    }
    fn plan_local(&self, base: &Path, check: bool) -> Result<Vec<EpisodePlan>> {
        let mut results = Vec::new();
        let mut outputs = BTreeSet::new();
        let mut works = BTreeSet::new();
        for ep in &self.episodes {
            let mut vars = self.vars.clone();
            vars.extend(ep.vars.clone());
            vars.insert("config_dir".into(), base.to_string_lossy().into());
            vars.insert("show".into(), self.project.show.clone());
            vars.insert("ep".into(), ep.id.clone());
            let sources = ep
                .source
                .parts()
                .into_iter()
                .map(|p| resolve(base, p, &vars, check))
                .collect::<Result<Vec<_>>>()?;
            ensure!(
                sources.iter().collect::<BTreeSet<_>>().len() == sources.len(),
                "Episode {} repeats a source file",
                ep.id
            );
            let cut = ep
                .cut
                .as_ref()
                .map(|spec| {
                    ensure!(sources.len() == 1, "Cut {} requires one M2TS source", ep.id);
                    ensure!(
                        sources[0]
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("m2ts")),
                        "Cut {} requires an M2TS source",
                        ep.id
                    );
                    Ok(CutPlan {
                        playlist: if sources[0].is_file() {
                            Some(crate::chapters::discover_cut_playlist(&sources[0])?)
                        } else {
                            None
                        },
                        start_chapter: spec.start_chapter,
                        end_chapter: spec.end_chapter,
                    })
                })
                .transpose()?;
            let work = absolute(base, &expand(&self.project.work_dir, &vars)?).join(&ep.id);
            ensure!(works.insert(work.clone()), "Duplicate workdir");
            let source = if sources.len() == 1 {
                sources[0].clone()
            } else {
                work.join(sources[0].file_name().context("Source has no filename")?)
                    .with_extension("joined.mkv")
            };
            vars.insert("episodes".into(), source.to_string_lossy().into());
            let out_dir = absolute(base, &expand(&self.project.out_dir, &vars)?);
            let name = expand(&self.project.output, &vars)?;
            ensure!(
                Path::new(&name).file_name().and_then(|x| x.to_str()) == Some(name.as_str())
                    && name.ends_with(".mkv"),
                "output must be an .mkv filename without directories: {name}"
            );
            let output = out_dir.join(name);
            ensure!(
                outputs.insert(output.clone()),
                "Two episodes produce {}",
                output.display()
            );
            let mut tracks = Vec::new();
            for (category, list) in [
                ("video", self.video.as_slice()),
                ("audio", self.audio.as_slice()),
                ("subtitle", self.subtitles.as_slice()),
                ("subtitle", self.pgs.as_slice()),
            ] {
                for original in list {
                    if !original.enabled(&ep.id) {
                        continue;
                    }
                    let t = original.effective(&ep.id);
                    tracks.push(TrackPlan {
                        id: t.id,
                        category: category.into(),
                        source: resolve(
                            base,
                            &t.source,
                            &vars,
                            check
                                && !(sources.len() > 1
                                    && expand(&t.source, &vars)? == source.to_string_lossy()),
                        )?,
                        name: expand(&t.name, &vars)?,
                        language: t.language,
                        track: t.track,
                    });
                }
            }
            ensure!(
                tracks.first().is_some_and(|t| t.category == "video"),
                "Episode {} has no video",
                ep.id
            );
            if cut.is_some() {
                ensure!(
                    tracks[0].source == source,
                    "Cut {} requires its first VideoTrack to use the [episodes] source",
                    ep.id
                );
                let v = self.video.iter().find(|t| t.id == tracks[0].id).unwrap();
                ensure!(
                    !v.audio && !v.subtitles,
                    "Cut {} requires separate AudioTrack and PGS tracks",
                    ep.id
                );
                for tp in &tracks {
                    if tp.category == "video" && tp.id != tracks[0].id && tp.source == source {
                        bail!(
                            "Cut {} supports only one video track from the shared M2TS",
                            ep.id
                        );
                    }
                    if tp.category == "audio" && tp.source == source {
                        let a = self.audio.iter().find(|t| t.id == tp.id).unwrap();
                        ensure!(
                            a.codec == "flac",
                            "Cut {} requires FLAC for audio from the shared M2TS",
                            ep.id
                        );
                    }
                }
            }
            results.push(EpisodePlan {
                episode: ep.id.clone(),
                source,
                sources,
                output,
                work,
                title: expand(&self.project.title, &vars)?,
                tracks,
                cut,
                vars,
            });
        }
        Ok(results)
    }
}

fn merge_config(base: &mut toml::Value, overlay: &toml::Value) -> Result<()> {
    let target = base.as_table_mut().context("Config root must be a table")?;
    let additions = overlay
        .as_table()
        .context("Episode config root must be a table")?;
    for (section, value) in additions {
        if section == "episodes" {
            continue;
        }
        match (target.get_mut(section), value) {
            (Some(toml::Value::Table(dst)), toml::Value::Table(src)) if section == "subtitles" => {
                merge_subtitles(dst, src)
            }
            (Some(toml::Value::Table(dst)), toml::Value::Table(src)) => merge_table(dst, src),
            (Some(dst), src) => *dst = src.clone(),
            (None, src) => {
                target.insert(section.clone(), src.clone());
            }
        }
    }
    Ok(())
}

fn merge_subtitles(
    dst: &mut toml::map::Map<String, toml::Value>,
    src: &toml::map::Map<String, toml::Value>,
) {
    for (key, value) in src {
        if key == "shift"
            && let (Some(toml::Value::Table(target)), toml::Value::Table(overrides)) =
                (dst.get_mut(key), value)
        {
            for (id, correction) in overrides {
                target.insert(id.clone(), correction.clone());
            }
        } else {
            match (dst.get_mut(key), value) {
                (Some(toml::Value::Table(d)), toml::Value::Table(s)) => merge_table(d, s),
                (Some(d), s) => *d = s.clone(),
                (None, s) => {
                    dst.insert(key.clone(), s.clone());
                }
            }
        }
    }
}

fn merge_table(
    dst: &mut toml::map::Map<String, toml::Value>,
    src: &toml::map::Map<String, toml::Value>,
) {
    for (key, value) in src {
        match (dst.get_mut(key), value) {
            (Some(toml::Value::Table(d)), toml::Value::Table(s)) => merge_table(d, s),
            (Some(d), s) => *d = s.clone(),
            (None, s) => {
                dst.insert(key.clone(), s.clone());
            }
        }
    }
}

fn disable_episode_tracks(
    config: &mut toml::Value,
    episode: &str,
    base: &toml::Value,
) -> Result<()> {
    let root = config
        .as_table_mut()
        .context("Config root must be a table")?;
    let disabled = ["VideoTrack", "AudioTrack", "subtitles", "pgs"];
    let mut ids = Vec::new();
    for section in disabled {
        let Some(table) = root.get_mut(section).and_then(toml::Value::as_table_mut) else {
            continue;
        };
        let keys: Vec<String> = table
            .iter()
            .filter_map(|(k, v)| (v == &toml::Value::Boolean(false)).then_some(k.clone()))
            .collect();
        for key in keys {
            let id = if section == "VideoTrack" || section == "AudioTrack" {
                format!("{section}.{key}")
            } else {
                key.clone()
            };
            ensure!(
                base.get(section)
                    .and_then(toml::Value::as_table)
                    .is_some_and(|t| t.contains_key(&key)),
                "Episode {episode} disables unknown {id}"
            );
            table.remove(&key);
            ids.push(id);
            if section == "subtitles" {
                for op in ["shift", "delete", "add"] {
                    if let Some(t) = table.get_mut(op).and_then(toml::Value::as_table_mut) {
                        t.remove(&key);
                    }
                }
            }
        }
    }
    if let Some(mux) = root.get_mut("mux").and_then(toml::Value::as_table_mut) {
        for id in ids {
            mux.remove(&id);
        }
    }
    let _ = episode;
    Ok(())
}
