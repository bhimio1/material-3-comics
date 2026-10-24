//! Where series come from. Every source answers the same questions
//! (browse, search, details, chapters, pages); the library stores what
//! they answer under the source's id.
//!
//! - `local`: comic files and folders on this computer.
//! - `mangadex`: MangaDex's public API, built in.
//! - `suwayomi:<id>`: a source of a Suwayomi server. Suwayomi runs Mihon's
//!   extensions, so every Mihon source works through it.

pub mod local;
pub mod mangadex;
pub mod suwayomi;

use crate::config::Config;
use crate::model::{ChapterInfo, MangaInfo, MangaList, PageRef};
use anyhow::{bail, Result};
use std::io::Read;
use std::sync::{Arc, RwLock};

pub const USER_AGENT: &str = concat!("m3-comics/", env!("CARGO_PKG_VERSION"), " (+https://github.com/bhimio1/material-3-comics)");
/// Biggest image we download.
const MAX_IMAGE: u64 = 48 << 20;

pub trait Source: Send + Sync {
    fn id(&self) -> String;
    fn name(&self) -> String;
    /// ISO 639-1 code, or "all" / "local".
    fn lang(&self) -> String;
    fn icon_url(&self) -> Option<String> {
        None
    }
    fn supports_latest(&self) -> bool {
        true
    }
    fn nsfw(&self) -> bool {
        false
    }
    fn popular(&self, page: u32) -> Result<MangaList>;
    fn latest(&self, page: u32) -> Result<MangaList>;
    fn search(&self, query: &str, page: u32) -> Result<MangaList>;
    fn details(&self, key: &str) -> Result<MangaInfo>;
    /// Newest first, as sources list them.
    fn chapters(&self, key: &str) -> Result<Vec<ChapterInfo>>;
    fn pages(&self, manga_key: &str, chapter_key: &str) -> Result<Vec<PageRef>>;
}

pub fn agent() -> ureq::Agent {
    material_3_ui::http::agent(USER_AGENT, true)
}

/// GET a URL and parse its JSON, with error statuses as `HttpError`s.
pub fn get_json(agent: &ureq::Agent, url: &str, headers: &[(String, String)]) -> Result<serde_json::Value> {
    let mut req = agent.get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let resp = material_3_ui::http::send(req, None)?;
    Ok(serde_json::from_reader(resp.into_reader().take(32 << 20))?)
}

pub fn get_bytes(agent: &ureq::Agent, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>> {
    let mut req = agent.get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let resp = material_3_ui::http::send(req, None)?;
    let mut out = Vec::new();
    resp.into_reader().take(MAX_IMAGE).read_to_end(&mut out)?;
    if out.is_empty() {
        bail!("The server sent an empty image");
    }
    Ok(out)
}

/// The image bytes of a page, wherever it lives.
pub fn fetch_page(agent: &ureq::Agent, page: &PageRef) -> Result<Vec<u8>> {
    match page {
        PageRef::Url { url, headers } => {
            // Image servers drop connections now and then; try twice.
            match get_bytes(agent, url, headers) {
                Ok(b) => Ok(b),
                Err(e) if material_3_ui::http::status_of(&e).is_some_and(|s| s < 500) => Err(e),
                Err(_) => get_bytes(agent, url, headers),
            }
        }
        PageRef::Archive { path, entry } => crate::archive::read(path, entry),
        PageRef::File(path) => Ok(std::fs::read(path)?),
    }
}

/// The sources this app knows: the built-in ones plus whatever the
/// configured Suwayomi server offers.
pub struct Sources {
    pub local: Arc<local::Local>,
    pub mangadex: Arc<mangadex::MangaDex>,
    pub suwayomi: RwLock<Option<Arc<suwayomi::Client>>>,
    /// The server's sources, as last listed.
    pub remote: RwLock<Vec<Arc<suwayomi::RemoteSource>>>,
}

impl Sources {
    pub fn new(cfg: &Config) -> Arc<Self> {
        let s = Arc::new(Sources {
            local: Arc::new(local::Local::new(cfg.local_folders())),
            mangadex: Arc::new(mangadex::MangaDex::new(cfg.mangadex_languages(), cfg.show_nsfw)),
            suwayomi: RwLock::new(None),
            remote: RwLock::new(Vec::new()),
        });
        s.configure(cfg);
        s
    }

    /// Pick up changed settings (folders, languages, the server).
    pub fn configure(&self, cfg: &Config) {
        self.local.set_folders(cfg.local_folders());
        self.mangadex.set_options(cfg.mangadex_languages(), cfg.show_nsfw);
        let client = cfg.suwayomi_url.trim();
        let new = (!client.is_empty()).then(|| Arc::new(suwayomi::Client::new(client, &cfg.suwayomi_user, &cfg.suwayomi_password)));
        let changed = {
            let cur = self.suwayomi.read().ok();
            let cur = cur.as_ref().and_then(|c| c.as_ref().map(|c| (c.base.clone(), c.auth.clone())));
            cur != new.as_ref().map(|c| (c.base.clone(), c.auth.clone()))
        };
        if changed {
            if let Ok(mut s) = self.suwayomi.write() {
                *s = new;
            }
            if let Ok(mut r) = self.remote.write() {
                r.clear();
            }
        }
    }

    pub fn server(&self) -> Option<Arc<suwayomi::Client>> {
        self.suwayomi.read().ok().and_then(|s| s.clone())
    }

    /// Ask the server for its sources (blocking).
    pub fn refresh_remote(&self) -> Result<usize> {
        let Some(client) = self.server() else {
            bail!("No Suwayomi server is set up");
        };
        let list = client.sources()?;
        let n = list.len();
        let list = list.into_iter().map(|info| Arc::new(suwayomi::RemoteSource::new(client.clone(), info))).collect();
        if let Ok(mut r) = self.remote.write() {
            *r = list;
        }
        Ok(n)
    }

    /// Every source, built-in first.
    pub fn all(&self) -> Vec<Arc<dyn Source>> {
        let mut v: Vec<Arc<dyn Source>> = vec![self.local.clone(), self.mangadex.clone()];
        if let Ok(r) = self.remote.read() {
            v.extend(r.iter().map(|s| s.clone() as Arc<dyn Source>));
        }
        v
    }

    /// A source by id. Suwayomi sources work before the server's list has
    /// been loaded, since the server only needs the ids.
    pub fn get(&self, id: &str) -> Option<Arc<dyn Source>> {
        match id {
            local::ID => Some(self.local.clone()),
            mangadex::ID => Some(self.mangadex.clone()),
            _ => {
                let remote_id = id.strip_prefix(suwayomi::PREFIX)?;
                if let Some(s) = self.remote.read().ok()?.iter().find(|s| s.info.id == remote_id) {
                    return Some(s.clone());
                }
                let client = self.server()?;
                Some(Arc::new(suwayomi::RemoteSource::new(client, suwayomi::SourceInfo { id: remote_id.to_string(), name: format!("Suwayomi source {remote_id}"), ..Default::default() })))
            }
        }
    }

    /// A display name, even for sources that aren't available right now.
    pub fn name_of(&self, id: &str) -> String {
        match self.get(id) {
            Some(s) => s.name(),
            None if id.starts_with(suwayomi::PREFIX) => "Suwayomi (not connected)".into(),
            None => id.to_string(),
        }
    }
}

/// "English", "Japanese", … for language codes sources use.
pub fn language_name(code: &str) -> String {
    let name = match code {
        "all" => "All languages",
        "local" => "On this computer",
        "en" => "English",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh" | "zh-hans" => "Chinese (Simplified)",
        "zh-hk" | "zh-hant" => "Chinese (Traditional)",
        "es" => "Spanish",
        "es-la" | "es-419" => "Spanish (Latin America)",
        "pt" => "Portuguese",
        "pt-br" => "Portuguese (Brazil)",
        "fr" => "French",
        "de" => "German",
        "it" => "Italian",
        "ru" => "Russian",
        "id" => "Indonesian",
        "vi" => "Vietnamese",
        "th" => "Thai",
        "tr" => "Turkish",
        "ar" => "Arabic",
        "pl" => "Polish",
        "uk" => "Ukrainian",
        "fil" | "tl" => "Filipino",
        "hi" => "Hindi",
        "ms" => "Malay",
        "nl" => "Dutch",
        "cs" => "Czech",
        "hu" => "Hungarian",
        "ro" => "Romanian",
        "bg" => "Bulgarian",
        "el" => "Greek",
        "he" => "Hebrew",
        "fa" => "Persian",
        "bn" => "Bengali",
        "my" => "Burmese",
        "mn" => "Mongolian",
        "ca" => "Catalan",
        "sv" => "Swedish",
        "fi" => "Finnish",
        "da" => "Danish",
        "no" | "nb" => "Norwegian",
        "other" => "Other",
        _ => return code.to_string(),
    };
    name.to_string()
}
