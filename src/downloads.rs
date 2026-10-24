//! Downloading chapters for offline reading. Each chapter becomes one CBZ
//! with a `ComicInfo.xml`, in
//! `<downloads>/<source>/<series>/<chapter>.cbz`, so other comic readers
//! (and the Local files source) can open them too.
//!
//! One chapter downloads at a time, its pages a few at once. The queue is
//! saved, so it carries on after a restart.

use crate::archive::{self, ComicInfo};
use crate::db::{Chapter, Db, Manga};
use crate::model::{format_number, safe_file_name};
use crate::sources::{self, Sources};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

const PAGE_THREADS: usize = 4;

/// Where a series' downloads go. Source ids, not names, so the folder
/// stays put when a source is renamed.
pub fn manga_dir(root: &Path, source: &str, title: &str) -> PathBuf {
    root.join(safe_file_name(&source.replace(':', "-"))).join(safe_file_name(title))
}

pub fn chapter_file_name(chapter: &Chapter) -> String {
    let mut name = chapter.name.clone();
    if !chapter.scanlator.is_empty() {
        name = format!("{name} [{}]", chapter.scanlator);
    }
    format!("{}.cbz", safe_file_name(&name))
}

pub fn chapter_path(root: &Path, manga: &Manga, chapter: &Chapter) -> PathBuf {
    manga_dir(root, &manga.source, manga.title()).join(chapter_file_name(chapter))
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Queued,
    Running { done: usize, total: usize },
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Job {
    pub manga: Manga,
    pub chapter: Chapter,
    pub dest: PathBuf,
    pub state: JobState,
}

pub enum Event {
    Finished,
    Failed { title: String, chapter: String, error: String },
}

struct Shared {
    jobs: Mutex<Vec<Job>>,
    wake: Condvar,
    paused: AtomicBool,
}

pub struct Downloads {
    shared: Arc<Shared>,
    events: flume::Receiver<Event>,
    root: PathBuf,
    /// File names in each series folder, by the folder's modification time.
    index: HashMap<PathBuf, (Option<SystemTime>, HashSet<String>)>,
    queue_file: PathBuf,
}

impl Downloads {
    pub fn new(ctx: &egui::Context, sources: Arc<Sources>, db: Arc<Db>, root: PathBuf) -> Self {
        let shared = Arc::new(Shared { jobs: Mutex::new(Vec::new()), wake: Condvar::new(), paused: AtomicBool::new(false) });
        let (tx, events) = flume::unbounded();
        let worker = shared.clone();
        let ctx2 = ctx.clone();
        std::thread::Builder::new()
            .name("m3-comics-downloads".into())
            .spawn(move || run(worker, sources, tx, ctx2))
            .expect("spawn the download thread");
        let mut d = Downloads { shared, events, root, index: HashMap::new(), queue_file: crate::config::data_dir().join("download-queue.json") };
        d.restore(&db);
        d
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.root = root;
        self.index.clear();
    }

    fn restore(&mut self, db: &Db) {
        let ids: Vec<i64> = std::fs::read_to_string(&self.queue_file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut items = Vec::new();
        for id in ids {
            if let Ok(Some(ch)) = db.chapter(id) {
                if let Ok(Some(m)) = db.manga(ch.manga_id) {
                    items.push((m, ch));
                }
            }
        }
        self.enqueue(items);
    }

    fn save_queue(&self, jobs: &[Job]) {
        let ids: Vec<i64> = jobs.iter().filter(|j| !matches!(j.state, JobState::Failed(_))).map(|j| j.chapter.id).collect();
        if let Some(dir) = self.queue_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.queue_file, serde_json::to_string(&ids).unwrap_or_default());
    }

    /// Queue chapters (skipping downloaded and queued ones).
    pub fn enqueue(&mut self, items: Vec<(Manga, Chapter)>) -> usize {
        let mut added = 0;
        let items: Vec<(Manga, Chapter)> = items.into_iter().filter(|(m, c)| m.source != crate::sources::local::ID && !self.is_downloaded(m, c)).collect();
        let fresh: Vec<Job> = items
            .into_iter()
            .map(|(manga, chapter)| Job { dest: chapter_path(&self.root, &manga, &chapter), manga, chapter, state: JobState::Queued })
            .collect();
        if let Ok(mut jobs) = self.shared.jobs.lock() {
            for job in fresh {
                if let Some(existing) = jobs.iter_mut().find(|j| j.chapter.id == job.chapter.id) {
                    if matches!(existing.state, JobState::Failed(_)) {
                        existing.state = JobState::Queued;
                        added += 1;
                    }
                } else {
                    jobs.push(job);
                    added += 1;
                }
            }
            self.save_queue(&jobs);
        }
        self.shared.wake.notify_all();
        added
    }

    pub fn cancel(&mut self, chapter_id: i64) {
        if let Ok(mut jobs) = self.shared.jobs.lock() {
            jobs.retain(|j| j.chapter.id != chapter_id || matches!(j.state, JobState::Running { .. }));
            // A running job is dropped when it finishes; flag it as failed so
            // the worker discards the result.
            if let Some(j) = jobs.iter_mut().find(|j| j.chapter.id == chapter_id) {
                j.state = JobState::Failed("Cancelled".into());
            }
            self.save_queue(&jobs);
        }
    }

    pub fn clear(&mut self) {
        if let Ok(mut jobs) = self.shared.jobs.lock() {
            jobs.retain(|j| matches!(j.state, JobState::Running { .. }));
            for j in jobs.iter_mut() {
                j.state = JobState::Failed("Cancelled".into());
            }
            self.save_queue(&jobs);
        }
    }

    pub fn retry_failed(&mut self) {
        if let Ok(mut jobs) = self.shared.jobs.lock() {
            for j in jobs.iter_mut() {
                if matches!(j.state, JobState::Failed(_)) {
                    j.state = JobState::Queued;
                }
            }
            self.save_queue(&jobs);
        }
        self.shared.wake.notify_all();
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }

    pub fn paused(&self) -> bool {
        self.shared.paused.load(Ordering::Relaxed)
    }

    pub fn jobs(&self) -> Vec<Job> {
        self.shared.jobs.lock().map(|j| j.clone()).unwrap_or_default()
    }

    /// Queued and running jobs.
    pub fn active(&self) -> usize {
        self.shared.jobs.lock().map(|j| j.iter().filter(|j| !matches!(j.state, JobState::Failed(_))).count()).unwrap_or(0)
    }

    pub fn state_of(&self, chapter_id: i64) -> Option<JobState> {
        self.shared.jobs.lock().ok()?.iter().find(|j| j.chapter.id == chapter_id).map(|j| j.state.clone())
    }

    /// Finished and failed downloads since the last call.
    pub fn poll(&mut self) -> Vec<Event> {
        let events: Vec<Event> = self.events.try_iter().collect();
        if !events.is_empty() {
            self.index.clear();
            if let Ok(jobs) = self.shared.jobs.lock() {
                self.save_queue(&jobs);
            }
        }
        events
    }

    fn names_in(&mut self, dir: &Path) -> &HashSet<String> {
        let mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        let stale = self.index.get(dir).is_none_or(|(t, _)| *t != mtime);
        if stale {
            let names = std::fs::read_dir(dir)
                .map(|r| r.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).filter(|n| n.ends_with(".cbz")).collect())
                .unwrap_or_default();
            self.index.insert(dir.to_path_buf(), (mtime, names));
        }
        &self.index[dir].1
    }

    pub fn is_downloaded(&mut self, manga: &Manga, chapter: &Chapter) -> bool {
        let dir = manga_dir(&self.root, &manga.source, manga.title());
        let name = chapter_file_name(chapter);
        self.names_in(&dir).contains(&name)
    }

    /// How many of a series' chapters are downloaded.
    pub fn count(&mut self, manga: &Manga) -> usize {
        let dir = manga_dir(&self.root, &manga.source, manga.title());
        self.names_in(&dir).len()
    }

    pub fn path_if_downloaded(&mut self, manga: &Manga, chapter: &Chapter) -> Option<PathBuf> {
        self.is_downloaded(manga, chapter).then(|| chapter_path(&self.root, manga, chapter))
    }

    pub fn delete(&mut self, manga: &Manga, chapters: &[Chapter]) -> usize {
        let mut n = 0;
        for c in chapters {
            if std::fs::remove_file(chapter_path(&self.root, manga, c)).is_ok() {
                n += 1;
            }
        }
        let dir = manga_dir(&self.root, &manga.source, manga.title());
        // Leave no empty folders behind.
        let _ = std::fs::remove_dir(&dir);
        self.index.remove(&dir);
        n
    }
}

fn next_job(shared: &Shared) -> Job {
    let mut jobs = shared.jobs.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if !shared.paused.load(Ordering::Relaxed) {
            if let Some(j) = jobs.iter_mut().find(|j| j.state == JobState::Queued) {
                j.state = JobState::Running { done: 0, total: 0 };
                return j.clone();
            }
        }
        jobs = shared.wake.wait(jobs).unwrap_or_else(|e| e.into_inner());
    }
}

fn set_state(shared: &Shared, id: i64, state: JobState) -> bool {
    let mut jobs = shared.jobs.lock().unwrap_or_else(|e| e.into_inner());
    match jobs.iter_mut().find(|j| j.chapter.id == id) {
        Some(j) if matches!(j.state, JobState::Running { .. }) => {
            j.state = state;
            true
        }
        _ => false,
    }
}

fn run(shared: Arc<Shared>, sources: Arc<Sources>, events: flume::Sender<Event>, ctx: egui::Context) {
    let agent = sources::agent();
    loop {
        let job = next_job(&shared);
        let result = download(&shared, &sources, &agent, &job, &ctx);
        let mut jobs = shared.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let cancelled = jobs.iter().find(|j| j.chapter.id == job.chapter.id).is_none_or(|j| matches!(j.state, JobState::Failed(_)));
        match result {
            Ok(()) if !cancelled => {
                jobs.retain(|j| j.chapter.id != job.chapter.id);
                let _ = events.send(Event::Finished);
            }
            Ok(()) => {
                let _ = std::fs::remove_file(&job.dest);
                jobs.retain(|j| j.chapter.id != job.chapter.id);
            }
            Err(e) => {
                if cancelled {
                    jobs.retain(|j| j.chapter.id != job.chapter.id);
                } else if let Some(j) = jobs.iter_mut().find(|j| j.chapter.id == job.chapter.id) {
                    j.state = JobState::Failed(e.to_string());
                    let _ = events.send(Event::Failed { title: job.manga.title().to_string(), chapter: job.chapter.name.clone(), error: e.to_string() });
                }
            }
        }
        drop(jobs);
        ctx.request_repaint();
    }
}

fn download(shared: &Shared, sources: &Sources, agent: &ureq::Agent, job: &Job, ctx: &egui::Context) -> anyhow::Result<()> {
    let source = sources.get(&job.manga.source).ok_or_else(|| anyhow::anyhow!("{} isn't available", sources.name_of(&job.manga.source)))?;
    let pages = source.pages(job.manga.key(), &job.chapter.key)?;
    let total = pages.len();
    set_state(shared, job.chapter.id, JobState::Running { done: 0, total });
    ctx.request_repaint();
    let results: Mutex<Vec<Option<Vec<u8>>>> = Mutex::new(vec![None; total]);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    let done = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..PAGE_THREADS.min(total.max(1)) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= total || failed.lock().map(|f| f.is_some()).unwrap_or(true) {
                    return;
                }
                match sources::fetch_page(agent, &pages[i]) {
                    Ok(bytes) => {
                        if let Ok(mut r) = results.lock() {
                            r[i] = Some(bytes);
                        }
                        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                        // Stop early when the job was cancelled.
                        if !set_state(shared, job.chapter.id, JobState::Running { done: n, total }) {
                            if let Ok(mut f) = failed.lock() {
                                f.get_or_insert(anyhow::anyhow!("Cancelled"));
                            }
                        }
                        ctx.request_repaint();
                    }
                    Err(e) => {
                        if let Ok(mut f) = failed.lock() {
                            f.get_or_insert(anyhow::anyhow!("Page {}: {e}", i + 1));
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = failed.into_inner().ok().flatten() {
        return Err(e);
    }
    let width = total.to_string().len().max(3);
    let pages: Vec<(String, Vec<u8>)> = results
        .into_inner()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, b)| {
            let b = b.unwrap_or_default();
            (format!("{:0width$}.{}", i + 1, archive::image_ext(&b)), b)
        })
        .collect();
    let info = ComicInfo {
        series: job.manga.title().to_string(),
        title: job.chapter.name.clone(),
        number: format_number(job.chapter.number),
        summary: job.manga.info.description.clone(),
        writer: job.manga.info.author.clone(),
        penciller: job.manga.info.artist.clone(),
        genre: job.manga.info.genres.clone(),
        web: job.chapter.web_url.clone().unwrap_or_default(),
        translator: job.chapter.scanlator.clone(),
        ..Default::default()
    };
    archive::write_cbz(&job.dest, &pages, Some(&info.to_xml()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MangaInfo;

    #[test]
    fn paths_are_stable_and_safe() {
        let m = Manga { source: "suwayomi:123".into(), info: MangaInfo { title: "A/B: C".into(), ..Default::default() }, ..Default::default() };
        let c = Chapter { name: "Chapter 1?".into(), scanlator: "Group".into(), ..Default::default() };
        assert_eq!(chapter_path(Path::new("/d"), &m, &c), PathBuf::from("/d/suwayomi-123/A_B_ C/Chapter 1_ [Group].cbz"));
        let c = Chapter { name: "Oneshot".into(), ..Default::default() };
        assert_eq!(chapter_file_name(&c), "Oneshot.cbz");
    }
}
