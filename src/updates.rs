//! Checking the library for new chapters: from the app (Updates → refresh,
//! or on start), or headless with `m3-comics update` (for a systemd timer).
//! Each source is asked one series at a time; different sources in
//! parallel.

use crate::db::{Chapter, Db, Manga};
use crate::model::{now, Status};
use crate::sources::Sources;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const LAST_UPDATE_KEY: &str = "last_library_update";

#[derive(Default)]
pub struct Report {
    pub checked: usize,
    /// Series with new chapters, and those chapters.
    pub new: Vec<(Manga, Vec<Chapter>)>,
    /// (series title, error)
    pub errors: Vec<(String, String)>,
}

impl Report {
    pub fn new_chapters(&self) -> usize {
        self.new.iter().map(|(_, c)| c.len()).sum()
    }

    /// "3 new chapters in One Piece and Berserk".
    pub fn summary(&self) -> String {
        let n = self.new_chapters();
        if n == 0 {
            return "No new chapters".into();
        }
        let titles: Vec<&str> = self.new.iter().map(|(m, _)| m.title()).collect();
        let list = match titles.len() {
            1 => titles[0].to_string(),
            2 => format!("{} and {}", titles[0], titles[1]),
            k => format!("{}, {} and {} more", titles[0], titles[1], k - 2),
        };
        format!("{n} new chapter{} in {list}", if n == 1 { "" } else { "s" })
    }
}

/// Live progress, for the Updates page.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub current: Mutex<String>,
}

/// Refresh one series: details and chapters. Returns the new chapters.
pub fn refresh(db: &Db, sources: &Sources, manga: &Manga) -> anyhow::Result<Vec<Chapter>> {
    let source = sources.get(&manga.source).ok_or_else(|| anyhow::anyhow!("{} isn't available", sources.name_of(&manga.source)))?;
    // Details can fail where chapters work (some sources), so don't stop there.
    if let Ok(mut info) = source.details(manga.key()) {
        info.key = manga.key().to_string();
        if info.title.is_empty() {
            info.title = manga.title().to_string();
        }
        db.upsert_manga(&manga.source, &info)?;
    }
    let list = source.chapters(manga.key())?;
    let new = db.sync_chapters(manga.id, &list)?;
    Ok(new.into_iter().filter_map(|id| db.chapter(id).ok().flatten()).collect())
}

/// Check every series in the library.
pub fn update_library(db: &Arc<Db>, sources: &Arc<Sources>, progress: &Arc<Progress>) -> Report {
    // Mihon-style: series marked completed rarely get chapters; still check
    // them, but last.
    let mut manga = db.library_manga().unwrap_or_default();
    manga.sort_by_key(|m| m.info.status == Status::Completed);
    progress.total.store(manga.len(), Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let mut by_source: BTreeMap<String, Vec<Manga>> = BTreeMap::new();
    for m in manga {
        by_source.entry(m.source.clone()).or_default().push(m);
    }
    let report = Arc::new(Mutex::new(Report::default()));
    std::thread::scope(|s| {
        for (_, list) in by_source {
            let report = report.clone();
            s.spawn(move || {
                for m in list {
                    if let Ok(mut c) = progress.current.lock() {
                        *c = m.title().to_string();
                    }
                    let result = refresh(db, sources, &m);
                    progress.done.fetch_add(1, Ordering::Relaxed);
                    let mut r = report.lock().unwrap_or_else(|e| e.into_inner());
                    r.checked += 1;
                    match result {
                        Ok(new) if !new.is_empty() => r.new.push((m, new)),
                        Ok(_) => {}
                        Err(e) => r.errors.push((m.title().to_string(), e.to_string())),
                    }
                }
            });
        }
    });
    let _ = db.set_meta(LAST_UPDATE_KEY, &now().to_string());
    Arc::try_unwrap(report).map(|m| m.into_inner().unwrap_or_default()).unwrap_or_default()
}

/// Whether the library is due for an automatic check.
pub fn due(db: &Db, interval_hours: u32) -> bool {
    if interval_hours == 0 {
        return false;
    }
    let last: i64 = db.meta(LAST_UPDATE_KEY).and_then(|s| s.parse().ok()).unwrap_or(0);
    now() - last >= interval_hours as i64 * 3600
}

pub fn notify(report: &Report) {
    if report.new_chapters() == 0 {
        return;
    }
    let n = material_3_ui::notify::Notification {
        body: report.summary(),
        icon: "material_3_comics".into(),
        desktop_entry: Some("material_3_comics".into()),
        ..material_3_ui::notify::Notification::new("Comics", "New chapters")
    };
    let _ = material_3_ui::notify::send(&n);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MangaInfo;

    #[test]
    fn summaries() {
        let m = |t: &str| Manga { info: MangaInfo { title: t.into(), ..Default::default() }, ..Default::default() };
        let mut r = Report::default();
        assert_eq!(r.summary(), "No new chapters");
        r.new.push((m("A"), vec![Chapter::default()]));
        assert_eq!(r.summary(), "1 new chapter in A");
        r.new.push((m("B"), vec![Chapter::default(), Chapter::default()]));
        assert_eq!(r.summary(), "3 new chapters in A and B");
        r.new.push((m("C"), vec![Chapter::default()]));
        assert_eq!(r.summary(), "4 new chapters in A, B and 1 more");
    }

    #[test]
    fn updates_local_series() {
        let dir = tempfile::tempdir().unwrap();
        let series = dir.path().join("S");
        std::fs::create_dir_all(&series).unwrap();
        crate::archive::write_cbz(&series.join("1.cbz"), &[("1.jpg".into(), vec![0xFF, 0xD8])], None).unwrap();
        let cfg = crate::config::Config { library_folders: vec![dir.path().display().to_string()], ..Default::default() };
        let sources = Sources::new(&cfg);
        let db = Arc::new(Db::open_in_memory().unwrap());
        let key = series.display().to_string();
        let id = db.upsert_manga("local", &MangaInfo { key: key.clone(), title: "S".into(), ..Default::default() }).unwrap();
        db.set_in_library(id, true).unwrap();
        let progress = Arc::new(Progress::default());
        let r = update_library(&db, &sources, &progress);
        assert_eq!((r.checked, r.new_chapters()), (1, 0), "the first check just fills in chapters");
        crate::archive::write_cbz(&series.join("2.cbz"), &[("1.jpg".into(), vec![0xFF, 0xD8])], None).unwrap();
        let r = update_library(&db, &sources, &progress);
        assert_eq!(r.new_chapters(), 1);
        assert_eq!(r.new[0].1[0].name, "2");
        assert_eq!(progress.done.load(Ordering::Relaxed), 1);
        assert!(!due(&db, 12));
        assert!(!due(&db, 0));
    }
}
