//! Tracking reading progress on AniList (https://anilist.co), like Mihon's
//! trackers. Sign-in uses AniList's implicit grant with its "pin" page:
//! AniList shows the access token, the user pastes it here, and it goes
//! to the keyring. Tokens last a year.
//!
//! Signing in needs an AniList API client (Settings → Developer on
//! anilist.co, redirect URL `https://anilist.co/api/v2/oauth/pin`); its id
//! goes in the preferences.

use crate::db::Track;
use anyhow::{anyhow, bail, Result};
use material_3_ui::secrets::Secrets;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::Arc;

pub const SERVICE: &str = "anilist";
pub const PIN_REDIRECT: &str = "https://anilist.co/api/v2/oauth/pin";
const TOKEN_KEY: &str = "anilist-token";

#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub id: i64,
    pub title: String,
    pub cover: Option<String>,
    pub chapters: Option<i64>,
    pub status: String,
    pub format: String,
    pub year: Option<i64>,
    pub url: String,
}

/// AniList's list statuses.
pub const STATUSES: &[(&str, &str)] = &[
    ("CURRENT", "Reading"),
    ("PLANNING", "Plan to read"),
    ("COMPLETED", "Completed"),
    ("PAUSED", "Paused"),
    ("DROPPED", "Dropped"),
    ("REPEATING", "Rereading"),
];

pub fn status_label(s: &str) -> &str {
    STATUSES.iter().find(|(k, _)| *k == s).map(|(_, l)| *l).unwrap_or(s)
}

pub fn auth_url(client_id: &str) -> String {
    format!("https://anilist.co/api/v2/oauth/authorize?client_id={}&response_type=token", client_id.trim())
}

/// Pull the token out of whatever was pasted: the bare token, or the
/// whole redirect URL (`…#access_token=…&token_type=Bearer`).
pub fn parse_token(pasted: &str) -> Option<String> {
    let s = pasted.trim();
    let token = match s.find("access_token=") {
        Some(i) => s[i + "access_token=".len()..].split('&').next().unwrap_or(""),
        None => s,
    };
    (token.len() > 20 && !token.contains(char::is_whitespace)).then(|| token.to_string())
}

pub struct AniList {
    agent: ureq::Agent,
    api: String,
    secrets: Arc<dyn Secrets>,
}

impl AniList {
    pub fn new(secrets: Arc<dyn Secrets>) -> Self {
        Self::with_api(secrets, "https://graphql.anilist.co")
    }

    pub fn with_api(secrets: Arc<dyn Secrets>, api: &str) -> Self {
        AniList { agent: crate::sources::agent(), api: api.to_string(), secrets }
    }

    pub fn token(&self) -> Option<String> {
        self.secrets.get(TOKEN_KEY).ok().flatten().filter(|t| !t.is_empty())
    }

    pub fn signed_in(&self) -> bool {
        self.token().is_some()
    }

    pub fn save_token(&self, token: &str) -> Result<()> {
        self.secrets.set(TOKEN_KEY, "Comics: AniList", token)
    }

    pub fn sign_out(&self) -> Result<()> {
        self.secrets.delete(TOKEN_KEY)
    }

    fn query(&self, query: &str, variables: Value) -> Result<Value> {
        let mut req = self.agent.post(&self.api).set("Content-Type", "application/json").set("Accept", "application/json");
        if let Some(t) = self.token() {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        let body = json!({ "query": query, "variables": variables }).to_string();
        let resp = match material_3_ui::http::send(req, Some(&body)) {
            Ok(r) => r,
            Err(e) => {
                // AniList answers errors with JSON bodies; show their message.
                if let Some(h) = e.downcast_ref::<material_3_ui::http::HttpError>() {
                    if h.status == 401 || h.status == 400 && h.body.contains("Invalid token") {
                        bail!("AniList didn't accept the sign-in; sign in again");
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(&h.body) {
                        if let Some(m) = v["errors"][0]["message"].as_str() {
                            bail!("AniList: {m}");
                        }
                    }
                }
                return Err(e);
            }
        };
        let v: Value = serde_json::from_reader(resp.into_reader().take(8 << 20))?;
        if let Some(m) = v["errors"][0]["message"].as_str() {
            bail!("AniList: {m}");
        }
        Ok(v["data"].clone())
    }

    /// The signed-in user's name.
    pub fn viewer(&self) -> Result<String> {
        let d = self.query("query { Viewer { id name } }", json!({}))?;
        d["Viewer"]["name"].as_str().map(str::to_string).ok_or_else(|| anyhow!("AniList didn't say who you are"))
    }

    pub fn search(&self, text: &str) -> Result<Vec<Media>> {
        let d = self.query(
            "query ($q: String) { Page(perPage: 25) { media(search: $q, type: MANGA) {
                id title { userPreferred english romaji } coverImage { medium } chapters status format startDate { year } siteUrl } } }",
            json!({ "q": text }),
        )?;
        Ok(d["Page"]["media"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|m| Media {
                        id: m["id"].as_i64().unwrap_or(0),
                        title: ["english", "userPreferred", "romaji"].iter().find_map(|k| m["title"][k].as_str()).unwrap_or_default().to_string(),
                        cover: m["coverImage"]["medium"].as_str().map(str::to_string),
                        chapters: m["chapters"].as_i64(),
                        status: m["status"].as_str().unwrap_or_default().to_string(),
                        format: m["format"].as_str().unwrap_or_default().to_string(),
                        year: m["startDate"]["year"].as_i64(),
                        url: m["siteUrl"].as_str().unwrap_or_default().to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Link a series to an AniList entry: read the user's list entry for
    /// it (or start one as "Reading").
    pub fn link(&self, manga_id: i64, media: &Media) -> Result<Track> {
        let d = self.query(
            "query ($id: Int) { Media(id: $id) { mediaListEntry { progress status score(format: POINT_10_DECIMAL) } } }",
            json!({ "id": media.id }),
        )?;
        let entry = &d["Media"]["mediaListEntry"];
        let mut t = Track {
            manga_id,
            service: SERVICE.into(),
            remote_id: media.id,
            title: media.title.clone(),
            progress: entry["progress"].as_i64().unwrap_or(0),
            total: media.chapters.unwrap_or(0),
            status: entry["status"].as_str().unwrap_or("CURRENT").to_string(),
            score: entry["score"].as_f64().unwrap_or(0.0),
            url: media.url.clone(),
        };
        if entry.is_null() {
            t = self.save(&t)?;
        }
        Ok(t)
    }

    /// Write progress, status and score back; returns what AniList stored.
    pub fn save(&self, t: &Track) -> Result<Track> {
        let d = self.query(
            "mutation ($id: Int, $progress: Int, $status: MediaListStatus, $score: Float) {
                SaveMediaListEntry(mediaId: $id, progress: $progress, status: $status, score: $score) { progress status score(format: POINT_10_DECIMAL) } }",
            json!({ "id": t.remote_id, "progress": t.progress, "status": t.status, "score": t.score }),
        )?;
        let e = &d["SaveMediaListEntry"];
        Ok(Track {
            progress: e["progress"].as_i64().unwrap_or(t.progress),
            status: e["status"].as_str().unwrap_or(&t.status).to_string(),
            score: e["score"].as_f64().unwrap_or(t.score),
            ..t.clone()
        })
    }
}

/// The progress to send after finishing chapter `number`, if it moves the
/// tracked progress forward. Also completes the entry at the last chapter.
pub fn advance(t: &Track, number: f32) -> Option<Track> {
    if number < 1.0 {
        return None;
    }
    let n = number.floor() as i64;
    if n <= t.progress {
        return None;
    }
    let mut next = Track { progress: n, ..t.clone() };
    if t.total > 0 && n >= t.total {
        next.status = "COMPLETED".into();
    } else if matches!(t.status.as_str(), "PLANNING" | "PAUSED" | "") {
        next.status = "CURRENT".into();
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use material_3_ui::secrets::Memory;
    use material_3_ui::testing::MockServer;

    #[test]
    fn tokens_from_pastes() {
        let t = "def50200aaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(parse_token(t).as_deref(), Some(t));
        assert_eq!(parse_token(&format!("https://anilist.co/api/v2/oauth/pin#access_token={t}&token_type=Bearer&expires_in=31536000")).as_deref(), Some(t));
        assert_eq!(parse_token("short"), None);
        assert_eq!(parse_token("has spaces in it and is long enough"), None);
    }

    #[test]
    fn progress_only_moves_forward() {
        let t = Track { progress: 4, total: 10, status: "PLANNING".into(), ..Default::default() };
        assert_eq!(advance(&t, 3.0), None);
        assert_eq!(advance(&t, 4.5), None);
        let n = advance(&t, 5.0).unwrap();
        assert_eq!((n.progress, n.status.as_str()), (5, "CURRENT"));
        assert_eq!(advance(&t, 10.0).unwrap().status, "COMPLETED");
        assert_eq!(advance(&t, -1.0), None);
    }

    #[test]
    fn search_link_and_save() {
        let s = MockServer::start(|req| {
            let q: Value = serde_json::from_str(&req.body).unwrap();
            let query = q["query"].as_str().unwrap();
            assert_eq!(req.headers.get("authorization").map(String::as_str), Some("Bearer tok"));
            if query.contains("Page") {
                assert_eq!(q["variables"]["q"], "berserk");
                (200u16, r#"{"data":{"Page":{"media":[{"id":30002,"title":{"romaji":"Berserk","english":null},"chapters":null,"status":"RELEASING","format":"MANGA","startDate":{"year":1989},"siteUrl":"https://anilist.co/manga/30002"}]}}}"#.to_string())
            } else if query.contains("mediaListEntry") {
                (200, r#"{"data":{"Media":{"mediaListEntry":null}}}"#.to_string())
            } else if query.contains("SaveMediaListEntry") {
                assert_eq!(q["variables"]["status"], "CURRENT");
                (200, format!(r#"{{"data":{{"SaveMediaListEntry":{{"progress":{},"status":"CURRENT","score":0}}}}}}"#, q["variables"]["progress"]))
            } else if query.contains("Viewer") {
                (200, r#"{"errors":[{"message":"Invalid token","status":400}]}"#.to_string())
            } else {
                (404, String::new())
            }
        });
        let secrets: Arc<dyn Secrets> = Arc::new(Memory::default());
        let al = AniList::with_api(secrets, &s.url);
        assert!(!al.signed_in());
        al.save_token("tok").unwrap();
        assert!(al.signed_in());
        let found = al.search("berserk").unwrap();
        assert_eq!(found[0].title, "Berserk");
        assert_eq!(found[0].year, Some(1989));
        let t = al.link(1, &found[0]).unwrap();
        assert_eq!((t.remote_id, t.progress, t.status.as_str()), (30002, 0, "CURRENT"));
        let saved = al.save(&Track { progress: 7, ..t }).unwrap();
        assert_eq!(saved.progress, 7);
        assert!(al.viewer().unwrap_err().to_string().contains("Invalid token"));
        al.sign_out().unwrap();
        assert!(!al.signed_in());
    }
}
