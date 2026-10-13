use crate::{font_metadata, subtitle::Ass};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Request {
    pub name: String,
    pub weight: u16,
    pub italic: bool,
}
#[derive(Debug, Clone)]
pub struct Face {
    pub source: PathBuf,
    pub family: String,
    pub aliases: BTreeSet<String>,
    pub exact: BTreeSet<String>,
    pub weight: u16,
    pub italic: bool,
    pub variable: bool,
}
#[derive(Debug, Default)]
pub struct FontIndex {
    pub faces: Vec<Face>,
}
#[derive(Debug, Serialize)]
pub struct CollectedFont {
    pub source: PathBuf,
    pub path: PathBuf,
    pub requested: Vec<String>,
    pub sha256: String,
}
fn key(s: &str) -> String {
    s.trim().trim_start_matches('@').to_lowercase()
}
fn request(style: &BTreeMap<String, String>) -> Request {
    Request {
        name: style.get("Fontname").cloned().unwrap_or("Arial".into()),
        weight: if style.get("Bold").is_some_and(|b| b != "0") {
            700
        } else {
            400
        },
        italic: style.get("Italic").is_some_and(|b| b != "0"),
    }
}
pub fn requests(doc: &Ass) -> Result<BTreeSet<Request>> {
    let styles = doc.styles()?;
    let styles: BTreeMap<_, _> = styles
        .into_iter()
        .map(|s| (s.get("Name").cloned().unwrap_or_default(), request(&s)))
        .collect();
    let style_col = doc
        .event_format
        .iter()
        .position(|s| s.eq_ignore_ascii_case("Style"))
        .unwrap();
    let text_col = doc.event_format.len() - 1;
    let blocks = regex::Regex::new(r"\{([^}]*)\}|([^{}]+)")?;
    let tags = regex::Regex::new(r"\\(fn|b|i|r)([^\\}()]*)")?;
    let mut result = BTreeSet::new();
    let mut missing_styles = BTreeSet::new();
    for line in &doc.lines {
        if !line.starts_with("Dialogue:") {
            continue;
        }
        let f = doc.event(line)?;
        let name = f[style_col].trim_start_matches([' ', '\t', '*']);
        let base = styles.get(name).cloned();
        if base.is_none() && missing_styles.insert(name.to_owned()) {
            crate::ui::warning(&format!(
                "ASS style '{name}' is missing; skipping its implicit font."
            ));
        }
        let mut current = base.clone();
        let mut reset_base = base.clone();
        for block in blocks.captures_iter(&f[text_col]) {
            if let Some(body) = block.get(1) {
                // Includes font changes inside \t(...) so animated variants are attached.
                for tag in tags.captures_iter(body.as_str()) {
                    let value = tag[2].trim();
                    match &tag[1] {
                        "fn" => {
                            if value.is_empty() {
                                current = reset_base.clone();
                            } else {
                                let mut font = current.clone().unwrap_or(Request {
                                    name: value.into(),
                                    weight: 400,
                                    italic: false,
                                });
                                font.name = value.into();
                                current = Some(font);
                            }
                        }
                        "b" => {
                            if let Some(font) = current.as_mut() {
                                if value.is_empty() {
                                    font.weight = reset_base.as_ref().map_or(400, |r| r.weight);
                                } else if let Ok(n) = value.parse::<i32>() {
                                    font.weight = match n {
                                        0 => 400,
                                        1 | -1 => 700,
                                        _ => n.clamp(100, 900) as u16,
                                    };
                                }
                            }
                        }
                        "i" => {
                            if let Some(font) = current.as_mut() {
                                if value.is_empty() {
                                    font.italic = reset_base.as_ref().is_some_and(|r| r.italic);
                                } else if let Ok(n) = value.parse::<i32>() {
                                    font.italic = n != 0;
                                }
                            }
                        }
                        "r" => {
                            reset_base = if value.is_empty() {
                                base.clone()
                            } else {
                                let style = styles.get(value).cloned();
                                if style.is_none() && missing_styles.insert(value.to_owned()) {
                                    crate::ui::warning(&format!(
                                        "ASS style '{value}' is missing; skipping its implicit font."
                                    ));
                                }
                                style
                            };
                            current = reset_base.clone();
                        }
                        _ => {}
                    }
                    // Conservative for animated and consecutive overrides.
                    if body.as_str().contains(r"\t(")
                        && let Some(font) = current.clone()
                    {
                        result.insert(font);
                    }
                }
            } else if block.get(2).is_some_and(|s| !s.as_str().trim().is_empty())
                && let Some(font) = current.clone()
            {
                result.insert(font);
            }
        }
    }
    Ok(result)
}
impl FontIndex {
    pub fn scan(dirs: &[PathBuf], system: bool) -> Result<Self> {
        let mut dirs = dirs.to_vec();
        if system {
            #[cfg(target_os = "linux")]
            {
                dirs.extend([
                    PathBuf::from("/usr/share/fonts"),
                    PathBuf::from("/usr/local/share/fonts"),
                    PathBuf::from("/run/current-system/sw/share/fonts"),
                ]);
            }
            #[cfg(target_os = "macos")]
            {
                dirs.extend([
                    PathBuf::from("/Library/Fonts"),
                    PathBuf::from("/System/Library/Fonts"),
                ]);
            }
            #[cfg(target_os = "windows")]
            {
                if let Ok(win) = std::env::var("WINDIR") {
                    dirs.push(PathBuf::from(win).join("Fonts"));
                }
            }
            if let Some(home) = std::env::var_os("HOME") {
                let h = PathBuf::from(home);
                dirs.extend([
                    h.join(".fonts"),
                    h.join(".local/share/fonts"),
                    h.join("Library/Fonts"),
                ]);
            }
        }
        let mut paths = BTreeSet::new();
        for dir in dirs {
            if !dir.exists() {
                ensure!(system, "Font directory not found: {}", dir.display());
                continue;
            }
            for entry in walkdir::WalkDir::new(&dir).follow_links(true) {
                let entry = entry.with_context(|| format!("Reading fonts in {}", dir.display()))?;
                if entry.file_type().is_file()
                    && entry.path().extension().is_some_and(|s| {
                        ["ttf", "otf", "ttc", "otc"]
                            .contains(&s.to_string_lossy().to_lowercase().as_str())
                    })
                {
                    paths.insert(entry.path().canonicalize()?);
                }
            }
        }
        let mut faces = Vec::new();
        for path in paths {
            let data = std::fs::read(&path)?;
            for f in font_metadata::parse(&data)
                .with_context(|| format!("Invalid font: {}", path.display()))?
            {
                let family = f
                    .family
                    .unwrap_or_else(|| path.file_stem().unwrap().to_string_lossy().into());
                faces.push(Face {
                    source: path.clone(),
                    family,
                    aliases: f.aliases,
                    exact: f.exact,
                    weight: f.weight,
                    italic: f.italic,
                    variable: f.variable,
                });
            }
        }
        Ok(Self { faces })
    }
    pub fn select(&self, req: &Request) -> Option<&Face> {
        let name = key(&req.name);
        self.faces
            .iter()
            .filter(|f| f.aliases.contains(&name))
            .min_by_key(|f| {
                (
                    if f.italic == req.italic { 0 } else { 1000 },
                    f.weight.abs_diff(req.weight),
                    if f.exact.contains(&name) { 0 } else { 1 },
                    &f.source,
                )
            })
    }
    pub fn collect(&self, requests: &BTreeSet<Request>, dir: &Path) -> Result<Vec<CollectedFont>> {
        let mut sources: BTreeMap<PathBuf, (&Face, Vec<String>)> = BTreeMap::new();
        for req in requests {
            if let Some(face) = self.select(req) {
                if face.weight != req.weight || face.italic != req.italic {
                    crate::ui::warning(&format!(
                        "Font {} requested weight {}, italic {}; found {} weight {}, italic {}",
                        req.name, req.weight, req.italic, face.family, face.weight, face.italic
                    ));
                }
                sources
                    .entry(face.source.clone())
                    .or_insert((face, Vec::new()))
                    .1
                    .push(format!(
                        "{} / {} / italic={}",
                        req.name, req.weight, req.italic
                    ));
            } else {
                crate::ui::warning(&format!(
                    "Font '{}' was not found (weight {}, italic {}).",
                    req.name, req.weight, req.italic
                ));
            }
        }
        std::fs::create_dir_all(dir)?;
        let mut hashes = BTreeSet::new();
        let mut names = BTreeMap::new();
        let mut result = Vec::new();
        for (source, (face, requested)) in sources {
            let data = std::fs::read(&source)?;
            let hash = Sha256::digest(&data)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if !hashes.insert(hash.clone()) {
                continue;
            }
            let stem = organized_name(face);
            let extension = source.extension().unwrap().to_string_lossy().to_lowercase();
            let mut name = format!("{stem}.{extension}");
            if names.get(&name).is_some_and(|h| h != &hash) {
                name = format!("{stem}-{}.{extension}", &hash[..12]);
            }
            names.insert(name.clone(), hash.clone());
            let path = dir.join(name);
            std::fs::write(&path, &data)?;
            result.push(CollectedFont {
                source,
                path,
                requested,
                sha256: hash,
            });
        }
        Ok(result)
    }
}
pub fn organized_name(face: &Face) -> String {
    let clean: String = face
        .family
        .chars()
        .map(|c| {
            if r#"/\<>:*?|""#.contains(c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut name = clean
        .split_whitespace()
        .map(|part| {
            if part.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()) {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            } else {
                part.to_owned()
            }
        })
        .collect::<String>();
    if name.is_empty() {
        name = "Font".into();
    }
    if face.variable {
        name.push_str("-Variable");
        if face.italic {
            name.push_str("Italic");
        }
    } else {
        let weight = match face.weight {
            100 => "Thin".into(),
            200 => "ExtraLight".into(),
            300 => "Light".into(),
            400 => String::new(),
            500 => "Medium".into(),
            600 => "SemiBold".into(),
            700 => "Bold".into(),
            800 => "ExtraBold".into(),
            900 => "Black".into(),
            n => n.to_string(),
        };
        if !weight.is_empty() {
            name.push('-');
            name.push_str(&weight);
        }
        if face.italic {
            if weight.is_empty() {
                name.push('-');
            }
            name.push_str("Italic");
        }
    }
    name
}
