//! Comics on this computer. Laid out like Mihon's local source:
//!
//! ```text
//! ~/Comics/
//!   One Piece/              a series
//!     cover.jpg             optional
//!     details.json          optional, Mihon's format
//!     Ch. 001.cbz           a chapter: CBZ, CBR, CB7, CBT, ZIP, EPUB …
//!     Ch. 002/              … or a folder of images
//!   Watchmen.cbr            a single file is a one-chapter series
//! ```
//!
//! Series keys are absolute paths, so files opened from the file manager
//! work even outside the configured folders.

use super::Source;
use crate::archive::{self, ComicInfo};
use crate::model::{natural_cmp, number_from_name, ChapterInfo, MangaInfo, MangaList, PageRef, Status};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::UNIX_EPOCH;

pub const ID: &str = "local";

pub struct Local {
    folders: RwLock<Vec<PathBuf>>,
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn hidden(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'))
}

/// A folder that is a chapter: it holds images.
fn is_image_folder(path: &Path) -> bool {
    path.is_dir() && !archive::list_folder(path).is_empty()
}

fn is_chapter(path: &Path) -> bool {
    !hidden(path) && ((path.is_file() && archive::is_comic_file(path)) || is_image_folder(path))
}

/// A folder or file that is a series.
fn is_series(path: &Path) -> bool {
    if hidden(path) {
        return false;
    }
    if path.is_file() {
        return archive::is_comic_file(path);
    }
    if !path.is_dir() {
        return false;
    }
    is_image_folder(path) || std::fs::read_dir(path).map(|r| r.flatten().any(|e| is_chapter(&e.path()))).unwrap_or(false)
}

fn display_name(path: &Path) -> String {
    let name = if path.is_file() { path.file_stem() } else { path.file_name() };
    name.map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

/// Mihon's `details.json`.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Details {
    title: Option<String>,
    author: Option<String>,
    artist: Option<String>,
    description: Option<String>,
    genre: Vec<String>,
    status: Option<serde_json::Value>,
}

impl Local {
    pub fn new(folders: Vec<PathBuf>) -> Self {
        Local { folders: RwLock::new(folders) }
    }

    pub fn set_folders(&self, folders: Vec<PathBuf>) {
        if let Ok(mut f) = self.folders.write() {
            *f = folders;
        }
    }

    pub fn folders(&self) -> Vec<PathBuf> {
        self.folders.read().map(|f| f.clone()).unwrap_or_default()
    }

    fn scan(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for dir in self.folders() {
            let Ok(read) = std::fs::read_dir(&dir) else { continue };
            out.extend(read.flatten().map(|e| e.path()).filter(|p| is_series(p)));
        }
        out
    }

    fn list(&self, mut items: Vec<PathBuf>) -> MangaList {
        items.dedup();
        MangaList { items: items.iter().map(|p| self.quick_info(p)).collect(), has_next: false }
    }

    /// Title and cover, without reading metadata files.
    fn quick_info(&self, path: &Path) -> MangaInfo {
        MangaInfo { key: path.display().to_string(), title: display_name(path), cover: cover_of(path), ..Default::default() }
    }

    /// The series that holds a file the user opened: its folder when the
    /// folder holds other chapters, else the file itself.
    pub fn series_for(path: &Path) -> PathBuf {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if path.is_file() || is_image_folder(&path) {
            if let Some(parent) = path.parent() {
                let siblings = std::fs::read_dir(parent).map(|r| r.flatten().filter(|e| is_chapter(&e.path())).count()).unwrap_or(0);
                if siblings > 1 && !is_image_folder(parent) {
                    return parent.to_path_buf();
                }
            }
        }
        path
    }
}

/// `cover.jpg` (or .png, .webp) next to the chapters, else the first page.
fn cover_of(path: &Path) -> Option<String> {
    if path.is_dir() {
        for name in ["cover.jpg", "cover.jpeg", "cover.png", "cover.webp", "Cover.jpg", "Cover.png", "folder.jpg"] {
            let p = path.join(name);
            if p.is_file() {
                return Some(format!("file://{}", p.display()));
            }
        }
    }
    let first = chapter_paths(path).into_iter().next_back()?;
    first_page(&first)
}

fn first_page(chapter: &Path) -> Option<String> {
    if chapter.is_dir() {
        archive::list_folder(chapter).first().map(|p| format!("file://{}", p.display()))
    } else {
        let entries = archive::list(chapter).ok()?;
        entries.first().map(|e| archive::cover_ref(chapter, e))
    }
}

/// The chapters of a series, newest (highest) first.
fn chapter_paths(series: &Path) -> Vec<PathBuf> {
    if series.is_file() {
        return vec![series.to_path_buf()];
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(series)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| is_chapter(p)).collect())
        .unwrap_or_default();
    if out.is_empty() && is_image_folder(series) {
        return vec![series.to_path_buf()];
    }
    out.sort_by(|a, b| natural_cmp(&b.file_name().unwrap_or_default().to_string_lossy(), &a.file_name().unwrap_or_default().to_string_lossy()));
    out
}

impl Source for Local {
    fn id(&self) -> String {
        ID.into()
    }

    fn name(&self) -> String {
        "Local files".into()
    }

    fn lang(&self) -> String {
        "local".into()
    }

    fn popular(&self, _page: u32) -> Result<MangaList> {
        let mut items = self.scan();
        items.sort_by(|a, b| natural_cmp(&display_name(a), &display_name(b)));
        Ok(self.list(items))
    }

    fn latest(&self, _page: u32) -> Result<MangaList> {
        let mut items = self.scan();
        items.sort_by_key(|p| std::cmp::Reverse(mtime(p)));
        Ok(self.list(items))
    }

    fn search(&self, query: &str, _page: u32) -> Result<MangaList> {
        let q = query.to_lowercase();
        let mut items: Vec<PathBuf> = self.scan().into_iter().filter(|p| display_name(p).to_lowercase().contains(&q)).collect();
        items.sort_by(|a, b| natural_cmp(&display_name(a), &display_name(b)));
        Ok(self.list(items))
    }

    fn details(&self, key: &str) -> Result<MangaInfo> {
        let path = PathBuf::from(key);
        if !path.exists() {
            bail!("{} is gone", path.display());
        }
        let mut info = self.quick_info(&path);
        let details: Details = std::fs::read_to_string(path.join("details.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // ComicInfo.xml of the first chapter fills in what details.json lacks.
        let comic = chapter_paths(&path)
            .into_iter()
            .next_back()
            .filter(|c| c.is_file())
            .and_then(|c| archive::read_text(&c, "ComicInfo.xml"))
            .and_then(|x| ComicInfo::parse(&x))
            .unwrap_or_default();
        let pick = |a: Option<String>, b: &str| a.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| b.to_string());
        if let Some(t) = details.title.filter(|t| !t.trim().is_empty()) {
            info.title = t;
        } else if !comic.series.is_empty() && path.is_file() {
            info.title = comic.series.clone();
        }
        info.author = pick(details.author, &comic.writer);
        info.artist = pick(details.artist, &comic.penciller);
        info.description = pick(details.description, &comic.summary);
        info.genres = if details.genre.is_empty() { comic.genre.clone() } else { details.genre };
        info.status = match details.status {
            Some(serde_json::Value::String(s)) => Status::parse(&s),
            Some(serde_json::Value::Number(n)) => Status::parse(&n.to_string()),
            _ => Status::Unknown,
        };
        info.web_url = (!comic.web.is_empty()).then(|| comic.web.clone());
        Ok(info)
    }

    fn chapters(&self, key: &str) -> Result<Vec<ChapterInfo>> {
        let path = PathBuf::from(key);
        if !path.exists() {
            bail!("{} is gone", path.display());
        }
        Ok(chapter_paths(&path)
            .into_iter()
            .map(|p| {
                let name = display_name(&p);
                ChapterInfo {
                    key: p.display().to_string(),
                    number: number_from_name(&name),
                    name,
                    uploaded: mtime(&p),
                    ..Default::default()
                }
            })
            .collect())
    }

    fn pages(&self, _manga_key: &str, chapter_key: &str) -> Result<Vec<PageRef>> {
        let path = PathBuf::from(chapter_key);
        if path.is_dir() {
            return Ok(archive::list_folder(&path).into_iter().map(PageRef::File).collect());
        }
        if !path.is_file() {
            bail!("{} is gone", path.display());
        }
        Ok(archive::list(&path)?.into_iter().map(|entry| PageRef::Archive { path: path.clone(), entry }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cbz(path: &Path, pages: &[&str], info: Option<&ComicInfo>) {
        let pages: Vec<(String, Vec<u8>)> = pages.iter().map(|n| (n.to_string(), vec![0xFF, 0xD8])).collect();
        archive::write_cbz(path, &pages, info.map(|i| i.to_xml()).as_deref()).unwrap();
    }

    #[test]
    fn scans_series_files_and_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let series = root.join("Series A");
        std::fs::create_dir_all(series.join("Ch. 2")).unwrap();
        std::fs::write(series.join("Ch. 2/01.jpg"), b"x").unwrap();
        let info = ComicInfo { summary: "A story.".into(), writer: "Someone".into(), genre: vec!["Action".into()], ..Default::default() };
        cbz(&series.join("Ch. 1.cbz"), &["b.jpg", "a.jpg"], Some(&info));
        cbz(&series.join("Ch. 10.cbz"), &["x.jpg"], None);
        cbz(&root.join("Oneshot.cbz"), &["p1.jpg"], None);
        std::fs::create_dir_all(root.join("Empty")).unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();

        let local = Local::new(vec![root.to_path_buf()]);
        let titles: Vec<String> = local.popular(1).unwrap().items.into_iter().map(|m| m.title).collect();
        assert_eq!(titles, vec!["Oneshot", "Series A"]);
        assert_eq!(local.search("series", 1).unwrap().items.len(), 1);

        let key = series.display().to_string();
        let chapters = local.chapters(&key).unwrap();
        let names: Vec<_> = chapters.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Ch. 10", "Ch. 2", "Ch. 1"]);
        assert_eq!(chapters[1].number, 2.0);

        let details = local.details(&key).unwrap();
        assert_eq!(details.description, "A story.");
        assert_eq!(details.author, "Someone");
        assert_eq!(details.genres, vec!["Action"]);
        assert_eq!(details.cover, Some(archive::cover_ref(&series.join("Ch. 1.cbz"), "a.jpg")));

        let pages = local.pages(&key, &chapters[2].key).unwrap();
        assert_eq!(pages[0], PageRef::Archive { path: series.join("Ch. 1.cbz"), entry: "a.jpg".into() });
        let folder_pages = local.pages(&key, &chapters[1].key).unwrap();
        assert_eq!(folder_pages, vec![PageRef::File(series.join("Ch. 2/01.jpg"))]);
    }

    #[test]
    fn details_json_wins() {
        let dir = tempfile::tempdir().unwrap();
        let series = dir.path().join("s");
        std::fs::create_dir_all(&series).unwrap();
        cbz(&series.join("1.cbz"), &["1.jpg"], None);
        std::fs::write(series.join("cover.png"), b"x").unwrap();
        std::fs::write(series.join("details.json"), r#"{"title":"Real Title","author":"A","genre":["G"],"status":"2"}"#).unwrap();
        let d = Local::new(vec![]).details(&series.display().to_string()).unwrap();
        assert_eq!(d.title, "Real Title");
        assert_eq!(d.status, Status::Completed);
        assert_eq!(d.cover, Some(format!("file://{}", series.join("cover.png").display())));
    }

    #[test]
    fn opened_files_find_their_series() {
        let dir = tempfile::tempdir().unwrap();
        let series = dir.path().join("s");
        std::fs::create_dir_all(&series).unwrap();
        cbz(&series.join("1.cbz"), &["1.jpg"], None);
        assert_eq!(Local::series_for(&series.join("1.cbz")), series.join("1.cbz").canonicalize().unwrap());
        cbz(&series.join("2.cbz"), &["1.jpg"], None);
        assert_eq!(Local::series_for(&series.join("1.cbz")), series.canonicalize().unwrap());
    }
}
