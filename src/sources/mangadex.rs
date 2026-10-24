//! MangaDex, through its public API (https://api.mangadex.org/docs/).
//! No account is needed to browse and read.

use super::{get_json, Source};
use crate::model::{chapter_title, ChapterInfo, MangaInfo, MangaList, PageRef, Status};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::sync::RwLock;

pub const ID: &str = "mangadex";
const PAGE_SIZE: u32 = 24;
const FEED_SIZE: u32 = 500;
/// The API refuses offsets past this.
const MAX_OFFSET: u32 = 10_000;

struct Options {
    languages: Vec<String>,
    nsfw: bool,
    data_saver: bool,
}

pub struct MangaDex {
    agent: ureq::Agent,
    api: String,
    uploads: String,
    options: RwLock<Options>,
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// The English text of a localized map, else any.
fn localized(v: &Value, prefer: &[String]) -> String {
    let Some(map) = v.as_object() else { return String::new() };
    for lang in prefer.iter().map(String::as_str).chain(["en", "ja-ro", "ja"]) {
        if let Some(s) = map.get(lang).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            return s.to_string();
        }
    }
    map.values().find_map(Value::as_str).unwrap_or_default().to_string()
}

fn timestamp(s: Option<&str>) -> i64 {
    s.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d| d.timestamp()).unwrap_or(0)
}

impl MangaDex {
    pub fn new(languages: Vec<String>, nsfw: bool) -> Self {
        Self::with_urls(languages, nsfw, "https://api.mangadex.org", "https://uploads.mangadex.org")
    }

    pub fn with_urls(languages: Vec<String>, nsfw: bool, api: &str, uploads: &str) -> Self {
        MangaDex {
            agent: super::agent(),
            api: api.trim_end_matches('/').to_string(),
            uploads: uploads.trim_end_matches('/').to_string(),
            options: RwLock::new(Options { languages, nsfw, data_saver: false }),
        }
    }

    pub fn set_options(&self, languages: Vec<String>, nsfw: bool) {
        if let Ok(mut o) = self.options.write() {
            o.languages = languages;
            o.nsfw = nsfw;
        }
    }

    pub fn set_data_saver(&self, on: bool) {
        if let Ok(mut o) = self.options.write() {
            o.data_saver = on;
        }
    }

    fn languages(&self) -> Vec<String> {
        self.options.read().map(|o| o.languages.clone()).unwrap_or_default()
    }

    fn ratings(&self, all: bool) -> String {
        let nsfw = all || self.options.read().map(|o| o.nsfw).unwrap_or(false);
        let mut r = vec!["safe", "suggestive"];
        if nsfw {
            r.extend(["erotica", "pornographic"]);
        }
        r.iter().map(|r| format!("&contentRating[]={r}")).collect()
    }

    fn language_filter(&self, param: &str) -> String {
        self.languages().iter().map(|l| format!("&{param}[]={}", enc(l))).collect()
    }

    fn list(&self, order: &str, title: Option<&str>, page: u32) -> Result<MangaList> {
        let offset = page.saturating_sub(1) * PAGE_SIZE;
        if offset >= MAX_OFFSET {
            return Ok(MangaList::default());
        }
        let mut url = format!(
            "{}/manga?limit={PAGE_SIZE}&offset={offset}&includes[]=cover_art&includes[]=author&hasAvailableChapters=true{}{}{order}",
            self.api,
            self.ratings(false),
            self.language_filter("availableTranslatedLanguage"),
        );
        if let Some(t) = title {
            url.push_str(&format!("&title={}", enc(t)));
        }
        let v = get_json(&self.agent, &url, &[])?;
        let items = v["data"].as_array().map(|a| a.iter().map(|m| self.manga(m)).collect()).unwrap_or_default();
        let total = v["total"].as_u64().unwrap_or(0) as u32;
        Ok(MangaList { items, has_next: offset + PAGE_SIZE < total.min(MAX_OFFSET) })
    }

    fn manga(&self, m: &Value) -> MangaInfo {
        let id = m["id"].as_str().unwrap_or_default().to_string();
        let a = &m["attributes"];
        let langs = self.languages();
        let mut title = localized(&a["title"], &langs);
        // Some series only have their English title among the alternatives.
        if a["title"].get("en").is_none() {
            if let Some(alt) = a["altTitles"].as_array().and_then(|alts| alts.iter().find_map(|t| t.get("en").and_then(Value::as_str))) {
                title = alt.to_string();
            }
        }
        let rels = m["relationships"].as_array().cloned().unwrap_or_default();
        let names = |kind: &str| {
            let mut v: Vec<String> = rels
                .iter()
                .filter(|r| r["type"] == kind)
                .filter_map(|r| r["attributes"]["name"].as_str().map(str::to_string))
                .collect();
            v.dedup();
            v.join(", ")
        };
        let cover = rels
            .iter()
            .find(|r| r["type"] == "cover_art")
            .and_then(|r| r["attributes"]["fileName"].as_str())
            .map(|f| format!("{}/covers/{id}/{f}.512.jpg", self.uploads));
        let genres = a["tags"]
            .as_array()
            .map(|tags| tags.iter().map(|t| localized(&t["attributes"]["name"], &[])).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        MangaInfo {
            title,
            cover,
            author: names("author"),
            artist: names("artist"),
            description: localized(&a["description"], &langs).trim().to_string(),
            genres,
            status: Status::parse(a["status"].as_str().unwrap_or("")),
            web_url: Some(format!("https://mangadex.org/title/{id}")),
            key: id,
        }
    }
}

impl Source for MangaDex {
    fn id(&self) -> String {
        ID.into()
    }

    fn name(&self) -> String {
        "MangaDex".into()
    }

    fn lang(&self) -> String {
        "all".into()
    }

    fn icon_url(&self) -> Option<String> {
        Some("https://mangadex.org/favicon.ico".into())
    }

    fn popular(&self, page: u32) -> Result<MangaList> {
        self.list("&order[followedCount]=desc", None, page)
    }

    fn latest(&self, page: u32) -> Result<MangaList> {
        self.list("&order[latestUploadedChapter]=desc", None, page)
    }

    fn search(&self, query: &str, page: u32) -> Result<MangaList> {
        // A pasted MangaDex link opens that series.
        if let Some(id) = query.trim().strip_prefix("https://mangadex.org/title/").map(|r| r.split('/').next().unwrap_or("")) {
            if !id.is_empty() {
                return Ok(MangaList { items: vec![self.details(id)?], has_next: false });
            }
        }
        self.list("&order[relevance]=desc", Some(query), page)
    }

    fn details(&self, key: &str) -> Result<MangaInfo> {
        let url = format!("{}/manga/{}?includes[]=cover_art&includes[]=author&includes[]=artist", self.api, enc(key));
        let v = get_json(&self.agent, &url, &[])?;
        Ok(self.manga(&v["data"]))
    }

    fn chapters(&self, key: &str) -> Result<Vec<ChapterInfo>> {
        let mut out = Vec::new();
        let mut offset = 0;
        loop {
            let url = format!(
                "{}/manga/{}/feed?limit={FEED_SIZE}&offset={offset}&includes[]=scanlation_group&order[volume]=desc&order[chapter]=desc&order[publishAt]=desc{}{}",
                self.api,
                enc(key),
                self.ratings(true),
                self.language_filter("translatedLanguage"),
            );
            let v = get_json(&self.agent, &url, &[])?;
            let data = v["data"].as_array().cloned().unwrap_or_default();
            for c in &data {
                let a = &c["attributes"];
                // Chapters hosted elsewhere (official sites) can't be read here.
                if a["externalUrl"].is_string() && a["pages"].as_u64().unwrap_or(0) == 0 {
                    continue;
                }
                let id = c["id"].as_str().unwrap_or_default().to_string();
                let groups: Vec<String> = c["relationships"]
                    .as_array()
                    .map(|r| r.iter().filter(|r| r["type"] == "scanlation_group").filter_map(|r| r["attributes"]["name"].as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let number = a["chapter"].as_str();
                out.push(ChapterInfo {
                    name: chapter_title(a["volume"].as_str(), number, a["title"].as_str()),
                    number: number.and_then(|n| n.parse().ok()).unwrap_or(-1.0),
                    scanlator: groups.join(" & "),
                    uploaded: timestamp(a["publishAt"].as_str().or(a["readableAt"].as_str())),
                    web_url: Some(format!("https://mangadex.org/chapter/{id}")),
                    key: id,
                });
            }
            offset += FEED_SIZE;
            let total = v["total"].as_u64().unwrap_or(0) as u32;
            if data.is_empty() || offset >= total {
                break;
            }
        }
        Ok(out)
    }

    fn pages(&self, _manga_key: &str, chapter_key: &str) -> Result<Vec<PageRef>> {
        let url = format!("{}/at-home/server/{}", self.api, enc(chapter_key));
        let v = get_json(&self.agent, &url, &[])?;
        let base = v["baseUrl"].as_str().ok_or_else(|| anyhow!("MangaDex didn't say where the pages are"))?;
        let hash = v["chapter"]["hash"].as_str().unwrap_or_default();
        let saver = self.options.read().map(|o| o.data_saver).unwrap_or(false);
        let (dir, files) = if saver { ("data-saver", &v["chapter"]["dataSaver"]) } else { ("data", &v["chapter"]["data"]) };
        let pages: Vec<PageRef> = files
            .as_array()
            .map(|f| {
                f.iter()
                    .filter_map(Value::as_str)
                    .map(|f| PageRef::Url { url: format!("{base}/{dir}/{hash}/{f}"), headers: Vec::new() })
                    .collect()
            })
            .unwrap_or_default();
        if pages.is_empty() {
            return Err(anyhow!("This chapter has no pages on MangaDex"));
        }
        Ok(pages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use material_3_ui::testing::MockServer;

    const MANGA: &str = r#"{"id":"m1","type":"manga","attributes":{
        "title":{"ja-ro":"Yotsuba to!"},"altTitles":[{"ja":"よつばと!"},{"en":"Yotsuba&!"}],
        "description":{"en":"  A girl.  "},"status":"ongoing",
        "tags":[{"attributes":{"name":{"en":"Comedy"}}}]},
        "relationships":[{"type":"author","attributes":{"name":"Azuma"}},{"type":"artist","attributes":{"name":"Azuma"}},
        {"type":"cover_art","attributes":{"fileName":"c.jpg"}}]}"#;

    fn server() -> MockServer {
        MockServer::start(|req| {
            let path = req.path.split('?').next().unwrap_or("").to_string();
            match path.as_str() {
                "/manga" => (200u16, format!(r#"{{"data":[{MANGA}],"total":30}}"#)),
                "/manga/m1" => (200, format!(r#"{{"data":{MANGA}}}"#)),
                "/manga/m1/feed" => (200, r#"{"total":3,"data":[
                    {"id":"c2","attributes":{"volume":"1","chapter":"2","title":"Two","pages":10,"publishAt":"2024-01-02T00:00:00+00:00"},
                     "relationships":[{"type":"scanlation_group","attributes":{"name":"G"}}]},
                    {"id":"cx","attributes":{"chapter":"1.5","externalUrl":"https://x","pages":0}},
                    {"id":"c1","attributes":{"chapter":"1","pages":5}}]}"#.to_string()),
                "/at-home/server/c2" => (200, r#"{"baseUrl":"https://cdn.example","chapter":{"hash":"h","data":["1.png","2.png"],"dataSaver":["1.jpg"]}}"#.to_string()),
                _ => (404, String::new()),
            }
        })
    }

    #[test]
    fn browses_and_reads() {
        let s = server();
        let md = MangaDex::with_urls(vec!["en".into()], false, &s.url, "https://up.example");
        let list = md.popular(1).unwrap();
        assert!(list.has_next);
        let m = &list.items[0];
        assert_eq!(m.title, "Yotsuba&!");
        assert_eq!(m.author, "Azuma");
        assert_eq!(m.cover.as_deref(), Some("https://up.example/covers/m1/c.jpg.512.jpg"));
        assert_eq!(m.description, "A girl.");
        assert_eq!(m.genres, vec!["Comedy"]);
        assert_eq!(m.status, Status::Ongoing);
        let req = &s.requests()[0];
        assert!(req.path.contains("order[followedCount]=desc"));
        assert!(req.path.contains("availableTranslatedLanguage[]=en"));
        assert!(!req.path.contains("pornographic"));

        assert!(!md.search("yotsuba", 2).unwrap().has_next);
        assert!(s.requests()[1].path.contains("title=yotsuba"));

        let chapters = md.chapters("m1").unwrap();
        assert_eq!(chapters.len(), 2, "external chapters are skipped");
        assert_eq!(chapters[0].name, "Vol. 1 Chapter 2: Two");
        assert_eq!(chapters[0].scanlator, "G");
        assert_eq!(chapters[0].uploaded, 1704153600);
        assert_eq!(chapters[1].number, 1.0);

        let pages = md.pages("m1", "c2").unwrap();
        assert_eq!(pages[1], PageRef::Url { url: "https://cdn.example/data/h/2.png".into(), headers: vec![] });
        md.set_data_saver(true);
        assert_eq!(md.pages("m1", "c2").unwrap().len(), 1);
        assert!(md.details("nope").is_err());
    }

    #[test]
    fn links_open_series() {
        let s = server();
        let md = MangaDex::with_urls(vec!["en".into()], true, &s.url, "https://up.example");
        let list = md.search("https://mangadex.org/title/m1/yotsuba", 1).unwrap();
        assert_eq!(list.items[0].key, "m1");
    }
}
