use crate::{
    chapters::escape,
    config::{Episode, Tmdb},
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::collections::BTreeMap;

// Public TMDB read token used as the default when no environment override is set.
const DEFAULT_TMDB_TOKEN: &str = include_str!("tmdb_read_token.txt");

pub struct Client {
    http: reqwest::blocking::Client,
    token: String,
    media: BTreeMap<String, Value>,
}
impl Client {
    pub fn new(config: &Tmdb) -> Result<Self> {
        let token = std::env::var(&config.token_env)
            .unwrap_or_else(|_| DEFAULT_TMDB_TOKEN.trim().to_owned());
        ensure!(!token.trim().is_empty(), "TMDB token is empty");
        Ok(Self {
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            token,
            media: BTreeMap::new(),
        })
    }
    fn get(&self, path: &str, language: &str) -> Result<Value> {
        self.http
            .get(format!("https://api.themoviedb.org/3/{path}"))
            .bearer_auth(&self.token)
            .query(&[
                ("language", language),
                ("append_to_response", "external_ids"),
            ])
            .send()?
            .error_for_status()?
            .json()
            .context("Invalid TMDB JSON")
    }
    pub fn tags(&mut self, config: &Tmdb, episode: &Episode) -> Result<String> {
        let prefix = if config.movie { "movie" } else { "tv" };
        let number = if config.movie {
            0
        } else {
            episode.id.parse::<i64>()? + config.offset as i64
        };
        ensure!(
            config.movie || number > 0,
            "Invalid TMDB number for {}: {number}; check the episode ID and TMDB offset",
            episode.id
        );
        let mut languages = Vec::new();
        for lang in &config.languages {
            let key = format!("{prefix}/{}:{lang}", config.id);
            if !self.media.contains_key(&key) {
                let media = self.get(&format!("{prefix}/{}", config.id), lang)?;
                self.media.insert(key.clone(), media);
            }
            let media = self.media[&key].clone();
            let ep = if config.movie {
                None
            } else {
                Some(self.get(
                    &format!("tv/{}/season/{}/episode/{number}", config.id, config.season),
                    lang,
                )?)
            };
            languages.push((lang.clone(), media, ep));
        }
        Ok(make_tags(config, &languages))
    }
}
fn simple(name: &str, value: &str, lang: Option<&str>) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut s = format!(
        "<Simple><Name>{}</Name><String>{}</String>",
        escape(name),
        escape(value)
    );
    if let Some(lang) = lang {
        let iso = match lang.split('-').next().unwrap_or("und") {
            "en" => "eng",
            "ja" => "jpn",
            "pt" => "por",
            "es" => "spa",
            "fr" => "fre",
            "de" => "ger",
            "ru" => "rus",
            "it" => "ita",
            "zh" => "chi",
            "ar" => "ara",
            "da" => "dan",
            "el" => "gre",
            "fi" => "fin",
            "he" => "heb",
            "ko" => "kor",
            "nb" | "no" => "nor",
            "nl" => "dut",
            "pl" => "pol",
            "th" => "tha",
            "tr" => "tur",
            "id" => "ind",
            "ms" => "may",
            "vi" => "vie",
            _ => "und",
        };
        s.push_str(&format!(
            "<TagLanguage>{iso}</TagLanguage><TagLanguageIETF>{}</TagLanguageIETF>",
            escape(lang)
        ));
    }
    s.push_str("</Simple>");
    s
}
fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
pub fn make_tags(config: &Tmdb, entries: &[(String, Value, Option<Value>)]) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Tags><Tag><Targets><TargetTypeValue>50</TargetTypeValue></Targets>",
    );
    if let Some((_, media, episode)) = entries.first() {
        if config.write_ids {
            xml.push_str(&simple(
                "TMDB",
                &format!(
                    "{}/{}",
                    if config.movie { "movie" } else { "tv" },
                    config.id
                ),
                None,
            ));
            xml.push_str(&simple(
                "IMDB",
                string(&media["external_ids"], "imdb_id"),
                None,
            ));
            if !config.movie
                && let Some(id) = media["external_ids"]["tvdb_id"].as_u64()
            {
                xml.push_str(&simple("TVDB", &id.to_string(), None));
            }
        }
        if config.write_date {
            xml.push_str(&simple(
                "DATE_RELEASED",
                if config.movie {
                    string(media, "release_date")
                } else {
                    episode
                        .as_ref()
                        .map(|e| string(e, "air_date"))
                        .unwrap_or("")
                },
                None,
            ));
        }
    }
    for (lang, media, episode) in entries {
        if config.write_summary {
            xml.push_str(&simple("SUMMARY", string(media, "overview"), Some(lang)));
        }
        if let Some(ep) = episode {
            if config.write_title {
                xml.push_str(&simple("DESCRIPTION", string(ep, "name"), Some(lang)));
            }
            if config.write_synopsis {
                xml.push_str(&simple("SYNOPSIS", string(ep, "overview"), Some(lang)));
            }
        }
    }
    xml.push_str("</Tag></Tags>\n");
    xml
}
pub fn track_tags(tags: &BTreeMap<String, String>) -> String {
    let mut xml = String::from("<Tags><Tag><Targets/>");
    for (name, value) in tags {
        xml.push_str(&simple(name, value, None));
    }
    xml.push_str("</Tag></Tags>");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_token_is_used_without_an_environment_override() {
        let config: Tmdb =
            toml::from_str("id = 1\ntoken_env = 'MUXCRAFT_TMDB_TEST_UNSET_TOKEN_2C19F3'").unwrap();
        let client = Client::new(&config).unwrap();
        assert_eq!(client.token, DEFAULT_TMDB_TOKEN.trim());
    }
}
