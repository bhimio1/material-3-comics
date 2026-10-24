//! Mihon's extensions, through a Suwayomi server
//! (https://github.com/Suwayomi/Suwayomi-Server). Suwayomi runs the same
//! extension APKs Mihon does and offers them over GraphQL, so each of its
//! sources shows up here as `suwayomi:<source id>`, and its extensions can
//! be installed, updated and removed from the Extensions tab.
//!
//! Only the fields every Suwayomi release since 1.0 has are asked for.

use super::{get_json, Source};
use crate::model::{ChapterInfo, MangaInfo, MangaList, PageRef, Status};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::io::Read;

pub const PREFIX: &str = "suwayomi:";
pub const DEFAULT_URL: &str = "http://127.0.0.1:4567";

const MANGA_FIELDS: &str = "id title thumbnailUrl author artist description genre status realUrl";
const EXTENSION_FIELDS: &str = "pkgName name lang versionName iconUrl isInstalled hasUpdate isObsolete isNsfw";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub lang: String,
    pub icon_url: Option<String>,
    pub supports_latest: bool,
    pub nsfw: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extension {
    pub pkg: String,
    pub name: String,
    pub lang: String,
    pub version: String,
    pub icon_url: Option<String>,
    pub installed: bool,
    pub has_update: bool,
    pub obsolete: bool,
    pub nsfw: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionAction {
    Install,
    Update,
    Uninstall,
}

pub struct Client {
    /// `http://host:port`, no trailing slash.
    pub base: String,
    /// The `Authorization` header, with basic auth turned on.
    pub auth: Option<String>,
    agent: ureq::Agent,
}

/// GraphQL string literal (JSON's escaping is valid GraphQL).
fn lit(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Suwayomi sends 64-bit ids as strings; older versions as numbers.
fn id_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn int(v: &Value) -> i64 {
    match v {
        Value::String(s) => s.parse().unwrap_or(0),
        Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    }
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

impl Client {
    pub fn new(base: &str, user: &str, password: &str) -> Self {
        let mut base = base.trim().trim_end_matches('/').to_string();
        if !base.contains("://") {
            base = format!("http://{base}");
        }
        let auth = (!user.is_empty()).then(|| format!("Basic {}", base64(format!("{user}:{password}").as_bytes())));
        Client { base, auth, agent: super::agent() }
    }

    pub fn headers(&self) -> Vec<(String, String)> {
        self.auth.iter().map(|a| ("Authorization".to_string(), a.clone())).collect()
    }

    /// A server path (`/api/v1/…`) as a full URL.
    pub fn absolute(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}/{}", self.base, path.trim_start_matches('/'))
        }
    }

    pub fn web_ui(&self) -> String {
        self.base.clone()
    }

    fn gql(&self, query: &str) -> Result<Value> {
        let mut req = self.agent.post(&format!("{}/api/graphql", self.base)).set("Content-Type", "application/json");
        for (k, v) in self.headers() {
            req = req.set(&k, &v);
        }
        let body = json!({ "query": query }).to_string();
        let resp = material_3_ui::http::send(req, Some(&body)).map_err(|e| match material_3_ui::http::status_of(&e) {
            Some(401) => anyhow!("Suwayomi asked for a user name and password"),
            Some(_) => e,
            None => anyhow!("Couldn't reach Suwayomi at {} ({e})", self.base),
        })?;
        let v: Value = serde_json::from_reader(resp.into_reader().take(32 << 20))?;
        if let Some(err) = v["errors"].as_array().and_then(|e| e.first()) {
            bail!("Suwayomi: {}", err["message"].as_str().unwrap_or("request failed"));
        }
        Ok(v["data"].clone())
    }

    /// The server's name and version, to check the connection.
    pub fn about(&self) -> Result<String> {
        let d = self.gql("query { aboutServer { name version } }")?;
        Ok(format!("{} {}", d["aboutServer"]["name"].as_str().unwrap_or("Suwayomi"), d["aboutServer"]["version"].as_str().unwrap_or("")).trim().to_string())
    }

    pub fn sources(&self) -> Result<Vec<SourceInfo>> {
        let d = self.gql("query { sources { nodes { id name lang displayName iconUrl supportsLatest isNsfw } } }")?;
        let mut out: Vec<SourceInfo> = d["sources"]["nodes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| SourceInfo {
                        id: id_str(&s["id"]),
                        name: s["displayName"].as_str().or(s["name"].as_str()).unwrap_or_default().to_string(),
                        lang: s["lang"].as_str().unwrap_or_default().to_string(),
                        icon_url: s["iconUrl"].as_str().filter(|u| !u.is_empty()).map(|u| self.absolute(u)),
                        supports_latest: s["supportsLatest"].as_bool().unwrap_or(true),
                        nsfw: s["isNsfw"].as_bool().unwrap_or(false),
                    })
                    // Suwayomi lists its own local source as id 0.
                    .filter(|s| s.id != "0")
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| a.lang.cmp(&b.lang).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(out)
    }

    fn parse_extensions(&self, list: &Value) -> Vec<Extension> {
        let mut out: Vec<Extension> = list
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|e| Extension {
                        pkg: e["pkgName"].as_str().unwrap_or_default().to_string(),
                        name: e["name"].as_str().unwrap_or_default().trim_start_matches("Tachiyomi: ").to_string(),
                        lang: e["lang"].as_str().unwrap_or_default().to_string(),
                        version: e["versionName"].as_str().unwrap_or_default().to_string(),
                        icon_url: e["iconUrl"].as_str().filter(|u| !u.is_empty()).map(|u| self.absolute(u)),
                        installed: e["isInstalled"].as_bool().unwrap_or(false),
                        has_update: e["hasUpdate"].as_bool().unwrap_or(false),
                        obsolete: e["isObsolete"].as_bool().unwrap_or(false),
                        nsfw: e["isNsfw"].as_bool().unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }

    /// The extensions the server knows, without asking the repositories.
    pub fn extensions(&self) -> Result<Vec<Extension>> {
        let d = self.gql(&format!("query {{ extensions {{ nodes {{ {EXTENSION_FIELDS} }} }} }}"))?;
        Ok(self.parse_extensions(&d["extensions"]["nodes"]))
    }

    /// Refresh the list from the extension repositories, then list them.
    pub fn fetch_extensions(&self) -> Result<Vec<Extension>> {
        let d = self.gql(&format!("mutation {{ fetchExtensions(input: {{}}) {{ extensions {{ {EXTENSION_FIELDS} }} }} }}"))?;
        Ok(self.parse_extensions(&d["fetchExtensions"]["extensions"]))
    }

    pub fn change_extension(&self, pkg: &str, action: ExtensionAction) -> Result<()> {
        let patch = match action {
            ExtensionAction::Install => "install: true",
            ExtensionAction::Update => "update: true",
            ExtensionAction::Uninstall => "uninstall: true",
        };
        self.gql(&format!("mutation {{ updateExtension(input: {{ id: {}, patch: {{ {patch} }} }}) {{ extension {{ pkgName isInstalled }} }} }}", lit(pkg)))?;
        Ok(())
    }

    fn manga(&self, m: &Value) -> MangaInfo {
        MangaInfo {
            key: id_str(&m["id"]),
            title: m["title"].as_str().unwrap_or_default().to_string(),
            cover: m["thumbnailUrl"].as_str().filter(|u| !u.is_empty()).map(|u| self.absolute(u)),
            author: m["author"].as_str().unwrap_or_default().to_string(),
            artist: m["artist"].as_str().unwrap_or_default().to_string(),
            description: m["description"].as_str().unwrap_or_default().trim().to_string(),
            genres: m["genre"].as_array().map(|g| g.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
            status: Status::parse(m["status"].as_str().unwrap_or("")),
            web_url: m["realUrl"].as_str().filter(|u| !u.is_empty()).map(str::to_string),
        }
    }

    fn browse(&self, source: &str, kind: &str, query: Option<&str>, page: u32) -> Result<MangaList> {
        let q = query.map(|q| format!(", query: {}", lit(q))).unwrap_or_default();
        let d = self.gql(&format!(
            "mutation {{ fetchSourceManga(input: {{ source: {}, type: {kind}, page: {}{q} }}) {{ hasNextPage mangas {{ {MANGA_FIELDS} }} }} }}",
            lit(source),
            page.max(1)
        ))?;
        let r = &d["fetchSourceManga"];
        Ok(MangaList {
            items: r["mangas"].as_array().map(|a| a.iter().map(|m| self.manga(m)).collect()).unwrap_or_default(),
            has_next: r["hasNextPage"].as_bool().unwrap_or(false),
        })
    }

    fn manga_id(key: &str) -> Result<i64> {
        key.parse().map_err(|_| anyhow!("Not a Suwayomi series: {key}"))
    }

    pub fn details(&self, key: &str) -> Result<MangaInfo> {
        let d = self.gql(&format!("mutation {{ fetchManga(input: {{ id: {} }}) {{ manga {{ {MANGA_FIELDS} }} }} }}", Self::manga_id(key)?))?;
        Ok(self.manga(&d["fetchManga"]["manga"]))
    }

    pub fn chapters(&self, key: &str) -> Result<Vec<ChapterInfo>> {
        let d = self.gql(&format!(
            "mutation {{ fetchChapters(input: {{ mangaId: {} }}) {{ chapters {{ id name chapterNumber scanlator uploadDate sourceOrder realUrl }} }} }}",
            Self::manga_id(key)?
        ))?;
        let mut list: Vec<(i64, ChapterInfo)> = d["fetchChapters"]["chapters"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|c| {
                        let uploaded = int(&c["uploadDate"]);
                        (
                            int(&c["sourceOrder"]),
                            ChapterInfo {
                                key: id_str(&c["id"]),
                                name: c["name"].as_str().unwrap_or_default().to_string(),
                                number: c["chapterNumber"].as_f64().unwrap_or(-1.0) as f32,
                                scanlator: c["scanlator"].as_str().unwrap_or_default().to_string(),
                                // Milliseconds.
                                uploaded: if uploaded > 100_000_000_000 { uploaded / 1000 } else { uploaded },
                                web_url: c["realUrl"].as_str().filter(|u| !u.is_empty()).map(str::to_string),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Suwayomi numbers chapters from the oldest (1) up.
        list.sort_by_key(|(order, _)| std::cmp::Reverse(*order));
        Ok(list.into_iter().map(|(_, c)| c).collect())
    }

    pub fn pages(&self, chapter_key: &str) -> Result<Vec<PageRef>> {
        let id: i64 = chapter_key.parse().map_err(|_| anyhow!("Not a Suwayomi chapter: {chapter_key}"))?;
        let d = self.gql(&format!("mutation {{ fetchChapterPages(input: {{ chapterId: {id} }}) {{ pages }} }}"))?;
        let headers = self.headers();
        let pages: Vec<PageRef> = d["fetchChapterPages"]["pages"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).map(|p| PageRef::Url { url: self.absolute(p), headers: headers.clone() }).collect())
            .unwrap_or_default();
        if pages.is_empty() {
            bail!("The source returned no pages for this chapter");
        }
        Ok(pages)
    }

    /// Plain REST GET, for checking the server is up.
    pub fn ping(&self) -> bool {
        get_json(&self.agent, &format!("{}/api/v1/settings/about", self.base), &self.headers()).is_ok() || self.about().is_ok()
    }
}

/// One of the server's sources.
pub struct RemoteSource {
    client: std::sync::Arc<Client>,
    pub info: SourceInfo,
}

impl RemoteSource {
    pub fn new(client: std::sync::Arc<Client>, info: SourceInfo) -> Self {
        RemoteSource { client, info }
    }
}

impl Source for RemoteSource {
    fn id(&self) -> String {
        format!("{PREFIX}{}", self.info.id)
    }

    fn name(&self) -> String {
        self.info.name.clone()
    }

    fn lang(&self) -> String {
        self.info.lang.clone()
    }

    fn icon_url(&self) -> Option<String> {
        self.info.icon_url.clone()
    }

    fn supports_latest(&self) -> bool {
        self.info.supports_latest
    }

    fn nsfw(&self) -> bool {
        self.info.nsfw
    }

    fn popular(&self, page: u32) -> Result<MangaList> {
        self.client.browse(&self.info.id, "POPULAR", None, page)
    }

    fn latest(&self, page: u32) -> Result<MangaList> {
        self.client.browse(&self.info.id, "LATEST", None, page)
    }

    fn search(&self, query: &str, page: u32) -> Result<MangaList> {
        self.client.browse(&self.info.id, "SEARCH", Some(query), page)
    }

    fn details(&self, key: &str) -> Result<MangaInfo> {
        self.client.details(key)
    }

    fn chapters(&self, key: &str) -> Result<Vec<ChapterInfo>> {
        self.client.chapters(key)
    }

    fn pages(&self, _manga_key: &str, chapter_key: &str) -> Result<Vec<PageRef>> {
        self.client.pages(chapter_key)
    }

    fn image_headers(&self) -> Vec<(String, String)> {
        self.client.headers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use material_3_ui::testing::MockServer;
    use std::sync::Arc;

    fn reply(body: &str) -> String {
        let q: Value = serde_json::from_str(body).unwrap();
        let q = q["query"].as_str().unwrap().to_string();
        if q.contains("sources {") {
            r#"{"data":{"sources":{"nodes":[
                {"id":"0","name":"Local source","lang":"localsourcelang","displayName":"Local source"},
                {"id":"2499283573021220255","name":"MangaDex","lang":"en","displayName":"MangaDex (EN)","iconUrl":"/api/v1/extension/icon/md.png","supportsLatest":true,"isNsfw":false},
                {"id":123,"name":"Another","lang":"de","displayName":"Another","iconUrl":"","supportsLatest":false,"isNsfw":true}]}}}"#.into()
        } else if q.contains("fetchSourceManga") {
            assert!(q.contains(r#"source: "123""#) && q.contains("type: SEARCH") && q.contains(r#"query: "a \"b\"""#), "{q}");
            r#"{"data":{"fetchSourceManga":{"hasNextPage":true,"mangas":[{"id":7,"title":"T","thumbnailUrl":"/api/v1/manga/7/thumbnail","genre":["X"],"status":"ON_HIATUS"}]}}}"#.into()
        } else if q.contains("fetchChapters") {
            r#"{"data":{"fetchChapters":{"chapters":[
                {"id":1,"name":"Ch 1","chapterNumber":1,"sourceOrder":1,"uploadDate":"1700000000000"},
                {"id":2,"name":"Ch 2","chapterNumber":2,"sourceOrder":2,"uploadDate":"1700000100000","scanlator":"S"}]}}}"#.into()
        } else if q.contains("fetchChapterPages") {
            r#"{"data":{"fetchChapterPages":{"pages":["/api/v1/manga/7/chapter/2/page/0","/api/v1/manga/7/chapter/2/page/1"]}}}"#.into()
        } else if q.contains("updateExtension") {
            assert!(q.contains(r#"id: "eu.kanade.x""#) && q.contains("uninstall: true"));
            r#"{"data":{"updateExtension":{"extension":{"pkgName":"eu.kanade.x","isInstalled":false}}}}"#.into()
        } else if q.contains("extensions") {
            r#"{"data":{"extensions":{"nodes":[{"pkgName":"b","name":"Tachiyomi: Beta","lang":"en","versionName":"1.4.2","iconUrl":"/i.png","isInstalled":true,"hasUpdate":true},{"pkgName":"a","name":"Alpha","lang":"all","versionName":"1.0","isInstalled":false}]}}}"#.into()
        } else {
            r#"{"errors":[{"message":"Unknown query"}]}"#.into()
        }
    }

    #[test]
    fn talks_graphql() {
        let s = MockServer::start(|req| {
            assert_eq!(req.path, "/api/graphql");
            assert_eq!(req.headers.get("authorization").map(String::as_str), Some("Basic dXNlcjpwYXNz"));
            (200u16, reply(&req.body))
        });
        let c = Arc::new(Client::new(&format!("{}/", s.url), "user", "pass"));
        let sources = c.sources().unwrap();
        assert_eq!(sources.len(), 2, "Suwayomi's own local source is left out");
        assert_eq!(sources[0].lang, "de");
        assert_eq!(sources[1].id, "2499283573021220255");
        assert_eq!(sources[1].icon_url, Some(format!("{}/api/v1/extension/icon/md.png", s.url)));
        assert_eq!(sources[0].icon_url, None);

        let src = RemoteSource::new(c.clone(), sources[0].clone());
        assert_eq!(src.id(), "suwayomi:123");
        assert!(!src.supports_latest());
        let list = src.search("a \"b\"", 1).unwrap();
        assert!(list.has_next);
        assert_eq!(list.items[0].key, "7");
        assert_eq!(list.items[0].status, Status::Hiatus);
        assert_eq!(list.items[0].cover, Some(format!("{}/api/v1/manga/7/thumbnail", s.url)));

        let chapters = src.chapters("7").unwrap();
        assert_eq!(chapters[0].name, "Ch 2", "newest first");
        assert_eq!(chapters[0].uploaded, 1_700_000_100);
        let pages = src.pages("7", "2").unwrap();
        assert_eq!(pages.len(), 2);
        assert!(matches!(&pages[0], PageRef::Url { url, headers } if url.ends_with("/page/0") && headers[0].0 == "Authorization"));

        let ext = c.extensions().unwrap();
        assert_eq!(ext[0].name, "Alpha");
        assert_eq!(ext[1].name, "Beta");
        assert!(ext[1].has_update);
        c.change_extension("eu.kanade.x", ExtensionAction::Uninstall).unwrap();
        assert!(src.details("not-a-number").is_err());
        assert!(c.about().unwrap_err().to_string().contains("Unknown query"));
    }

    #[test]
    fn helpers() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(Client::new("localhost:4567", "", "").base, "http://localhost:4567");
        assert!(Client::new("x", "", "").auth.is_none());
        assert!(Client::new("http://127.0.0.1:9", "", "").sources().unwrap_err().to_string().contains("Couldn't reach"));
    }
}
