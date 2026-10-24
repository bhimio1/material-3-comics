//! The library: series, chapters with read progress, categories, history
//! and tracking, in SQLite at `~/.local/share/m3-comics/library.db`.
//!
//! Series are stored as soon as they are opened (so their chapters can be
//! read and remembered); `in_library` marks the ones the user added.

use crate::config::ReadingMode;
use crate::model::{now, ChapterInfo, MangaInfo, Status};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS manga (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    key TEXT NOT NULL,
    title TEXT NOT NULL,
    cover TEXT,
    author TEXT NOT NULL DEFAULT '',
    artist TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    genres TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'unknown',
    web_url TEXT,
    in_library INTEGER NOT NULL DEFAULT 0,
    added_at INTEGER NOT NULL DEFAULT 0,
    last_update INTEGER NOT NULL DEFAULT 0,
    reader_mode TEXT,
    UNIQUE (source, key)
);
CREATE TABLE IF NOT EXISTS chapter (
    id INTEGER PRIMARY KEY,
    manga_id INTEGER NOT NULL REFERENCES manga(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    name TEXT NOT NULL,
    number REAL NOT NULL DEFAULT -1,
    scanlator TEXT NOT NULL DEFAULT '',
    uploaded INTEGER NOT NULL DEFAULT 0,
    source_order INTEGER NOT NULL DEFAULT 0,
    web_url TEXT,
    read INTEGER NOT NULL DEFAULT 0,
    bookmark INTEGER NOT NULL DEFAULT 0,
    last_page INTEGER NOT NULL DEFAULT 0,
    pages INTEGER NOT NULL DEFAULT 0,
    fetched_at INTEGER NOT NULL DEFAULT 0,
    read_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE (manga_id, key)
);
CREATE INDEX IF NOT EXISTS chapter_manga ON chapter(manga_id);
CREATE INDEX IF NOT EXISTS chapter_fetched ON chapter(fetched_at);
CREATE INDEX IF NOT EXISTS chapter_read_at ON chapter(read_at);
CREATE TABLE IF NOT EXISTS category (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    sort INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS manga_category (
    manga_id INTEGER NOT NULL REFERENCES manga(id) ON DELETE CASCADE,
    category_id INTEGER NOT NULL REFERENCES category(id) ON DELETE CASCADE,
    PRIMARY KEY (manga_id, category_id)
);
CREATE TABLE IF NOT EXISTS track (
    manga_id INTEGER NOT NULL REFERENCES manga(id) ON DELETE CASCADE,
    service TEXT NOT NULL,
    remote_id INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    progress INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT '',
    score REAL NOT NULL DEFAULT 0,
    url TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (manga_id, service)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manga {
    pub id: i64,
    pub source: String,
    pub info: MangaInfo,
    pub in_library: bool,
    pub added_at: i64,
    pub last_update: i64,
    pub reader_mode: Option<ReadingMode>,
}

impl Manga {
    pub fn key(&self) -> &str {
        &self.info.key
    }

    pub fn title(&self) -> &str {
        &self.info.title
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chapter {
    pub id: i64,
    pub manga_id: i64,
    pub key: String,
    pub name: String,
    pub number: f32,
    pub scanlator: String,
    pub uploaded: i64,
    /// Higher is newer.
    pub source_order: i64,
    pub web_url: Option<String>,
    pub read: bool,
    pub bookmark: bool,
    pub last_page: i64,
    pub pages: i64,
    /// When an update found it (0: it came with the series).
    pub fetched_at: i64,
    pub read_at: i64,
}

impl Chapter {
    pub fn in_progress(&self) -> bool {
        !self.read && self.last_page > 0
    }
}

#[derive(Clone, Debug, Default)]
pub struct LibraryEntry {
    pub manga: Manga,
    pub unread: i64,
    pub total: i64,
    pub last_read: i64,
    pub latest_upload: i64,
    pub categories: Vec<i64>,
}

#[derive(Clone, Debug)]
pub struct ChapterWithManga {
    pub chapter: Chapter,
    pub manga: Manga,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub manga_id: i64,
    pub service: String,
    pub remote_id: i64,
    pub title: String,
    pub progress: i64,
    pub total: i64,
    pub status: String,
    pub score: f64,
    pub url: String,
}

pub struct Db {
    conn: Mutex<Connection>,
}

const MANGA_COLS: &str = "m.id, m.source, m.key, m.title, m.cover, m.author, m.artist, m.description, m.genres, m.status, m.web_url, m.in_library, m.added_at, m.last_update, m.reader_mode";
const CHAPTER_COLS: &str = "c.id, c.manga_id, c.key, c.name, c.number, c.scanlator, c.uploaded, c.source_order, c.web_url, c.read, c.bookmark, c.last_page, c.pages, c.fetched_at, c.read_at";

fn manga_row(r: &Row, at: usize) -> rusqlite::Result<Manga> {
    let genres: String = r.get(at + 8)?;
    let status: String = r.get(at + 9)?;
    let mode: Option<String> = r.get(at + 14)?;
    Ok(Manga {
        id: r.get(at)?,
        source: r.get(at + 1)?,
        info: MangaInfo {
            key: r.get(at + 2)?,
            title: r.get(at + 3)?,
            cover: r.get(at + 4)?,
            author: r.get(at + 5)?,
            artist: r.get(at + 6)?,
            description: r.get(at + 7)?,
            genres: serde_json::from_str(&genres).unwrap_or_default(),
            status: Status::parse(&status),
            web_url: r.get(at + 10)?,
        },
        in_library: r.get(at + 11)?,
        added_at: r.get(at + 12)?,
        last_update: r.get(at + 13)?,
        reader_mode: mode.as_deref().and_then(ReadingMode::from_key),
    })
}

fn chapter_row(r: &Row, at: usize) -> rusqlite::Result<Chapter> {
    Ok(Chapter {
        id: r.get(at)?,
        manga_id: r.get(at + 1)?,
        key: r.get(at + 2)?,
        name: r.get(at + 3)?,
        number: r.get::<_, f64>(at + 4)? as f32,
        scanlator: r.get(at + 5)?,
        uploaded: r.get(at + 6)?,
        source_order: r.get(at + 7)?,
        web_url: r.get(at + 8)?,
        read: r.get(at + 9)?,
        bookmark: r.get(at + 10)?,
        last_page: r.get(at + 11)?,
        pages: r.get(at + 12)?,
        fetched_at: r.get(at + 13)?,
        read_at: r.get(at + 14)?,
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db { conn: Mutex::new(conn) })
    }

    fn c(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Store what a source said about a series (keeping library state).
    pub fn upsert_manga(&self, source: &str, info: &MangaInfo) -> Result<i64> {
        let c = self.c();
        let genres = serde_json::to_string(&info.genres)?;
        c.execute(
            "INSERT INTO manga (source, key, title, cover, author, artist, description, genres, status, web_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (source, key) DO UPDATE SET
               title = excluded.title,
               cover = COALESCE(excluded.cover, manga.cover),
               author = CASE WHEN excluded.author = '' THEN manga.author ELSE excluded.author END,
               artist = CASE WHEN excluded.artist = '' THEN manga.artist ELSE excluded.artist END,
               description = CASE WHEN excluded.description = '' THEN manga.description ELSE excluded.description END,
               genres = CASE WHEN excluded.genres = '[]' THEN manga.genres ELSE excluded.genres END,
               status = CASE WHEN excluded.status = 'unknown' THEN manga.status ELSE excluded.status END,
               web_url = COALESCE(excluded.web_url, manga.web_url)",
            params![source, info.key, info.title, info.cover, info.author, info.artist, info.description, genres, info.status.as_str(), info.web_url],
        )?;
        Ok(c.query_row("SELECT id FROM manga WHERE source = ?1 AND key = ?2", params![source, info.key], |r| r.get(0))?)
    }

    pub fn manga(&self, id: i64) -> Result<Option<Manga>> {
        Ok(self.c().query_row(&format!("SELECT {MANGA_COLS} FROM manga m WHERE m.id = ?1"), [id], |r| manga_row(r, 0)).optional()?)
    }

    pub fn find_manga(&self, source: &str, key: &str) -> Result<Option<Manga>> {
        Ok(self
            .c()
            .query_row(&format!("SELECT {MANGA_COLS} FROM manga m WHERE m.source = ?1 AND m.key = ?2"), params![source, key], |r| manga_row(r, 0))
            .optional()?)
    }

    /// Keys of a source's series that are in the library, for badges.
    pub fn library_keys(&self, source: &str) -> Result<HashSet<String>> {
        let c = self.c();
        let mut st = c.prepare("SELECT key FROM manga WHERE source = ?1 AND in_library = 1")?;
        let keys = st.query_map([source], |r| r.get(0))?.collect::<rusqlite::Result<HashSet<String>>>()?;
        Ok(keys)
    }

    pub fn set_in_library(&self, id: i64, on: bool) -> Result<()> {
        self.c().execute(
            "UPDATE manga SET in_library = ?2, added_at = CASE WHEN ?2 = 1 AND in_library = 0 THEN ?3 ELSE added_at END WHERE id = ?1",
            params![id, on, now()],
        )?;
        Ok(())
    }

    pub fn set_reader_mode(&self, id: i64, mode: Option<ReadingMode>) -> Result<()> {
        self.c().execute("UPDATE manga SET reader_mode = ?2 WHERE id = ?1", params![id, mode.map(|m| m.key())])?;
        Ok(())
    }

    /// Bring the chapter list in line with the source's (newest first).
    /// Read progress is kept. Returns the ids of chapters that are new
    /// since the last sync (none on the first one).
    pub fn sync_chapters(&self, manga_id: i64, list: &[ChapterInfo]) -> Result<Vec<i64>> {
        let mut c = self.c();
        let tx = c.transaction()?;
        let had: i64 = tx.query_row("SELECT COUNT(*) FROM chapter WHERE manga_id = ?1", [manga_id], |r| r.get(0))?;
        let stamp = now();
        let mut new = Vec::new();
        let n = list.len() as i64;
        let mut keys = HashSet::new();
        for (i, ch) in list.iter().enumerate() {
            if !keys.insert(ch.key.clone()) {
                continue;
            }
            let order = n - i as i64;
            let existing: Option<i64> = tx
                .query_row("SELECT id FROM chapter WHERE manga_id = ?1 AND key = ?2", params![manga_id, ch.key], |r| r.get(0))
                .optional()?;
            match existing {
                Some(id) => {
                    tx.execute(
                        "UPDATE chapter SET name = ?2, number = ?3, scanlator = ?4, uploaded = ?5, source_order = ?6, web_url = ?7 WHERE id = ?1",
                        params![id, ch.name, ch.number as f64, ch.scanlator, ch.uploaded, order, ch.web_url],
                    )?;
                }
                None => {
                    let fetched = if had > 0 { stamp } else { 0 };
                    tx.execute(
                        "INSERT INTO chapter (manga_id, key, name, number, scanlator, uploaded, source_order, web_url, fetched_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![manga_id, ch.key, ch.name, ch.number as f64, ch.scanlator, ch.uploaded, order, ch.web_url, fetched],
                    )?;
                    if had > 0 {
                        new.push(tx.last_insert_rowid());
                    }
                }
            }
        }
        // Chapters the source dropped go, unless the list came back empty
        // (more likely an error than a series without chapters).
        if !list.is_empty() {
            let mut st = tx.prepare("SELECT id, key FROM chapter WHERE manga_id = ?1")?;
            let gone: Vec<i64> = st
                .query_map([manga_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .filter_map(|r| r.ok())
                .filter(|(_, k)| !keys.contains(k))
                .map(|(id, _)| id)
                .collect();
            drop(st);
            for id in gone {
                tx.execute("DELETE FROM chapter WHERE id = ?1", [id])?;
            }
        }
        tx.execute("UPDATE manga SET last_update = ?2 WHERE id = ?1", params![manga_id, stamp])?;
        tx.commit()?;
        Ok(new)
    }

    /// A series' chapters, newest first.
    pub fn chapters(&self, manga_id: i64) -> Result<Vec<Chapter>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT {CHAPTER_COLS} FROM chapter c WHERE c.manga_id = ?1 ORDER BY c.source_order DESC, c.id DESC"))?;
        let rows = st.query_map([manga_id], |r| chapter_row(r, 0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn chapter(&self, id: i64) -> Result<Option<Chapter>> {
        Ok(self.c().query_row(&format!("SELECT {CHAPTER_COLS} FROM chapter c WHERE c.id = ?1"), [id], |r| chapter_row(r, 0)).optional()?)
    }

    /// Remember the page the reader is on; the last page marks it read.
    pub fn set_progress(&self, chapter_id: i64, page: usize, pages: usize) -> Result<bool> {
        let finished = pages > 0 && page + 1 >= pages;
        self.c().execute(
            "UPDATE chapter SET last_page = ?2, pages = ?3, read_at = ?4, read = CASE WHEN ?5 THEN 1 ELSE read END WHERE id = ?1",
            params![chapter_id, page as i64, pages as i64, now(), finished],
        )?;
        Ok(finished)
    }

    pub fn set_read(&self, ids: &[i64], read: bool) -> Result<()> {
        let mut c = self.c();
        let tx = c.transaction()?;
        for id in ids {
            tx.execute("UPDATE chapter SET read = ?2, last_page = CASE WHEN ?2 THEN last_page ELSE 0 END WHERE id = ?1", params![id, read])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_bookmark(&self, ids: &[i64], on: bool) -> Result<()> {
        let mut c = self.c();
        let tx = c.transaction()?;
        for id in ids {
            tx.execute("UPDATE chapter SET bookmark = ?2 WHERE id = ?1", params![id, on])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The library, with unread counts and each series' categories.
    pub fn library(&self) -> Result<Vec<LibraryEntry>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {MANGA_COLS},
               (SELECT COUNT(*) FROM chapter c WHERE c.manga_id = m.id AND c.read = 0),
               (SELECT COUNT(*) FROM chapter c WHERE c.manga_id = m.id),
               (SELECT COALESCE(MAX(read_at), 0) FROM chapter c WHERE c.manga_id = m.id),
               (SELECT COALESCE(MAX(MAX(uploaded, fetched_at)), 0) FROM chapter c WHERE c.manga_id = m.id)
             FROM manga m WHERE m.in_library = 1"
        ))?;
        let mut out = st
            .query_map([], |r| {
                Ok(LibraryEntry { manga: manga_row(r, 0)?, unread: r.get(15)?, total: r.get(16)?, last_read: r.get(17)?, latest_upload: r.get(18)?, categories: Vec::new() })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut st = c.prepare("SELECT manga_id, category_id FROM manga_category")?;
        let pairs = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.filter_map(|r| r.ok()).collect::<Vec<_>>();
        for e in &mut out {
            e.categories = pairs.iter().filter(|(m, _)| *m == e.manga.id).map(|(_, c)| *c).collect();
        }
        Ok(out)
    }

    /// Series in the library, for updating.
    pub fn library_manga(&self) -> Result<Vec<Manga>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT {MANGA_COLS} FROM manga m WHERE m.in_library = 1 ORDER BY m.title COLLATE NOCASE"))?;
        let rows = st.query_map([], |r| manga_row(r, 0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Chapters that updates found, newest first.
    pub fn updates(&self, limit: usize) -> Result<Vec<ChapterWithManga>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {CHAPTER_COLS}, {MANGA_COLS} FROM chapter c JOIN manga m ON m.id = c.manga_id
             WHERE m.in_library = 1 AND c.fetched_at > 0
             ORDER BY c.fetched_at DESC, c.source_order DESC LIMIT ?1"
        ))?;
        let rows = st
            .query_map([limit as i64], |r| Ok(ChapterWithManga { chapter: chapter_row(r, 0)?, manga: manga_row(r, 15)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The last chapter read of each series, most recent first.
    pub fn history(&self, limit: usize) -> Result<Vec<ChapterWithManga>> {
        let c = self.c();
        let mut st = c.prepare(&format!(
            "SELECT {CHAPTER_COLS}, {MANGA_COLS} FROM chapter c JOIN manga m ON m.id = c.manga_id
             WHERE c.read_at > 0 AND c.read_at = (SELECT MAX(read_at) FROM chapter c2 WHERE c2.manga_id = c.manga_id)
             GROUP BY c.manga_id ORDER BY c.read_at DESC LIMIT ?1"
        ))?;
        let rows = st
            .query_map([limit as i64], |r| Ok(ChapterWithManga { chapter: chapter_row(r, 0)?, manga: manga_row(r, 15)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn clear_history(&self, manga_id: Option<i64>) -> Result<()> {
        match manga_id {
            Some(id) => self.c().execute("UPDATE chapter SET read_at = 0 WHERE manga_id = ?1", [id])?,
            None => self.c().execute("UPDATE chapter SET read_at = 0", [])?,
        };
        Ok(())
    }

    pub fn categories(&self) -> Result<Vec<Category>> {
        let c = self.c();
        let mut st = c.prepare("SELECT id, name FROM category ORDER BY sort, id")?;
        let rows = st.query_map([], |r| Ok(Category { id: r.get(0)?, name: r.get(1)? }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn add_category(&self, name: &str) -> Result<i64> {
        let c = self.c();
        let sort: i64 = c.query_row("SELECT COALESCE(MAX(sort), 0) + 1 FROM category", [], |r| r.get(0))?;
        c.execute("INSERT INTO category (name, sort) VALUES (?1, ?2)", params![name.trim(), sort])?;
        Ok(c.last_insert_rowid())
    }

    pub fn rename_category(&self, id: i64, name: &str) -> Result<()> {
        self.c().execute("UPDATE category SET name = ?2 WHERE id = ?1", params![id, name.trim()])?;
        Ok(())
    }

    pub fn delete_category(&self, id: i64) -> Result<()> {
        self.c().execute("DELETE FROM category WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Move a category one place up (-1) or down (+1).
    pub fn move_category(&self, id: i64, delta: i64) -> Result<()> {
        let cats = self.categories()?;
        let Some(i) = cats.iter().position(|c| c.id == id) else { return Ok(()) };
        let j = i as i64 + delta;
        if j < 0 || j >= cats.len() as i64 {
            return Ok(());
        }
        let mut order: Vec<i64> = cats.iter().map(|c| c.id).collect();
        order.swap(i, j as usize);
        let mut c = self.c();
        let tx = c.transaction()?;
        for (sort, id) in order.iter().enumerate() {
            tx.execute("UPDATE category SET sort = ?2 WHERE id = ?1", params![id, sort as i64])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_manga_categories(&self, manga_id: i64, cats: &[i64]) -> Result<()> {
        let mut c = self.c();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM manga_category WHERE manga_id = ?1", [manga_id])?;
        for cat in cats {
            tx.execute("INSERT OR IGNORE INTO manga_category (manga_id, category_id) VALUES (?1, ?2)", params![manga_id, cat])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn manga_categories(&self, manga_id: i64) -> Result<Vec<i64>> {
        let c = self.c();
        let mut st = c.prepare("SELECT category_id FROM manga_category WHERE manga_id = ?1")?;
        let rows = st.query_map([manga_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn track(&self, manga_id: i64, service: &str) -> Result<Option<Track>> {
        Ok(self
            .c()
            .query_row(
                "SELECT manga_id, service, remote_id, title, progress, total, status, score, url FROM track WHERE manga_id = ?1 AND service = ?2",
                params![manga_id, service],
                |r| {
                    Ok(Track {
                        manga_id: r.get(0)?,
                        service: r.get(1)?,
                        remote_id: r.get(2)?,
                        title: r.get(3)?,
                        progress: r.get(4)?,
                        total: r.get(5)?,
                        status: r.get(6)?,
                        score: r.get(7)?,
                        url: r.get(8)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_track(&self, t: &Track) -> Result<()> {
        self.c().execute(
            "INSERT OR REPLACE INTO track (manga_id, service, remote_id, title, progress, total, status, score, url) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![t.manga_id, t.service, t.remote_id, t.title, t.progress, t.total, t.status, t.score, t.url],
        )?;
        Ok(())
    }

    pub fn remove_track(&self, manga_id: i64, service: &str) -> Result<()> {
        self.c().execute("DELETE FROM track WHERE manga_id = ?1 AND service = ?2", params![manga_id, service])?;
        Ok(())
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.c().query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.c().execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(())
    }

    /// Forget series that aren't in the library and were never read.
    pub fn prune(&self) -> Result<usize> {
        Ok(self.c().execute(
            "DELETE FROM manga WHERE in_library = 0 AND NOT EXISTS (SELECT 1 FROM chapter c WHERE c.manga_id = manga.id AND (c.read_at > 0 OR c.read = 1))",
            [],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(key: &str, title: &str) -> MangaInfo {
        MangaInfo { key: key.into(), title: title.into(), author: "A".into(), genres: vec!["G".into()], status: Status::Ongoing, ..Default::default() }
    }

    fn ch(key: &str, n: f32) -> ChapterInfo {
        ChapterInfo { key: key.into(), name: format!("Chapter {n}"), number: n, ..Default::default() }
    }

    #[test]
    fn upsert_keeps_library_state_and_known_fields() {
        let db = Db::open_in_memory().unwrap();
        let id = db.upsert_manga("mangadex", &info("k", "T")).unwrap();
        db.set_in_library(id, true).unwrap();
        // A browse result carries less detail; it mustn't wipe what we have.
        let again = db.upsert_manga("mangadex", &MangaInfo { key: "k".into(), title: "T2".into(), ..Default::default() }).unwrap();
        assert_eq!(id, again);
        let m = db.manga(id).unwrap().unwrap();
        assert!(m.in_library && m.added_at > 0);
        assert_eq!(m.info.title, "T2");
        assert_eq!(m.info.author, "A");
        assert_eq!(m.info.genres, vec!["G"]);
        assert_eq!(m.info.status, Status::Ongoing);
        assert_eq!(db.find_manga("mangadex", "k").unwrap().unwrap().id, id);
        assert!(db.library_keys("mangadex").unwrap().contains("k"));
        db.set_reader_mode(id, Some(ReadingMode::Webtoon)).unwrap();
        assert_eq!(db.manga(id).unwrap().unwrap().reader_mode, Some(ReadingMode::Webtoon));
    }

    #[test]
    fn chapter_sync_progress_and_updates() {
        let db = Db::open_in_memory().unwrap();
        let id = db.upsert_manga("s", &info("k", "T")).unwrap();
        db.set_in_library(id, true).unwrap();
        assert!(db.sync_chapters(id, &[ch("b", 2.0), ch("a", 1.0)]).unwrap().is_empty(), "first sync isn't an update");
        let chapters = db.chapters(id).unwrap();
        assert_eq!(chapters.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), vec!["b", "a"]);
        let a = chapters[1].id;
        assert!(!db.set_progress(a, 3, 10).unwrap());
        assert!(db.chapter(a).unwrap().unwrap().in_progress());
        assert!(db.set_progress(a, 9, 10).unwrap());
        assert!(db.chapter(a).unwrap().unwrap().read);

        let new = db.sync_chapters(id, &[ch("c", 3.0), ch("b", 2.0), ch("a", 1.0)]).unwrap();
        assert_eq!(new.len(), 1);
        let updates = db.updates(10).unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].chapter.key, "c");
        assert!(db.chapter(a).unwrap().unwrap().read, "progress survives a sync");

        let lib = db.library().unwrap();
        assert_eq!((lib[0].unread, lib[0].total), (2, 3));
        db.set_read(&[new[0]], true).unwrap();
        assert_eq!(db.library().unwrap()[0].unread, 1);

        // Dropped chapters go, empty lists are ignored.
        db.sync_chapters(id, &[ch("c", 3.0)]).unwrap();
        assert_eq!(db.chapters(id).unwrap().len(), 1);
        db.sync_chapters(id, &[]).unwrap();
        assert_eq!(db.chapters(id).unwrap().len(), 1);
    }

    #[test]
    fn history_is_one_row_per_series() {
        let db = Db::open_in_memory().unwrap();
        let m1 = db.upsert_manga("s", &info("1", "One")).unwrap();
        let m2 = db.upsert_manga("s", &info("2", "Two")).unwrap();
        db.sync_chapters(m1, &[ch("b", 2.0), ch("a", 1.0)]).unwrap();
        db.sync_chapters(m2, &[ch("x", 1.0)]).unwrap();
        let c1 = db.chapters(m1).unwrap();
        db.set_progress(c1[1].id, 0, 5).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        db.set_progress(c1[0].id, 1, 5).unwrap();
        let h = db.history(10).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].chapter.key, "b");
        // m2 was never read or added: pruned. m1 has been read: kept.
        assert_eq!(db.prune().unwrap(), 1);
        assert!(db.manga(m2).unwrap().is_none());
        assert!(db.manga(m1).unwrap().is_some());
        db.clear_history(Some(m1)).unwrap();
        assert!(db.history(10).unwrap().is_empty());
    }

    #[test]
    fn categories_and_tracks() {
        let db = Db::open_in_memory().unwrap();
        let m = db.upsert_manga("s", &info("1", "One")).unwrap();
        db.set_in_library(m, true).unwrap();
        let a = db.add_category("Reading").unwrap();
        let b = db.add_category("Done").unwrap();
        db.move_category(b, -1).unwrap();
        assert_eq!(db.categories().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), vec![b, a]);
        db.set_manga_categories(m, &[a, b]).unwrap();
        assert_eq!(db.library().unwrap()[0].categories.len(), 2);
        db.delete_category(b).unwrap();
        assert_eq!(db.manga_categories(m).unwrap(), vec![a]);
        db.rename_category(a, " Now ").unwrap();
        assert_eq!(db.categories().unwrap()[0].name, "Now");

        let t = Track { manga_id: m, service: "anilist".into(), remote_id: 30002, title: "Berserk".into(), progress: 3, ..Default::default() };
        db.set_track(&t).unwrap();
        assert_eq!(db.track(m, "anilist").unwrap(), Some(t));
        db.remove_track(m, "anilist").unwrap();
        assert_eq!(db.track(m, "anilist").unwrap(), None);
        db.set_meta("last_update", "5").unwrap();
        assert_eq!(db.meta("last_update").as_deref(), Some("5"));
    }
}
