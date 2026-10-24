//! The window: state, navigation, background jobs and the actions pages
//! ask for. Drawing lives in `ui/`, the reader in `reader.rs`.

use crate::config::Config;
use crate::covers::Covers;
use crate::db::{Chapter, ChapterWithManga, Db, LibraryEntry, Manga};
use crate::downloads::{self, Downloads};
use crate::model::MangaInfo;
use crate::reader::Reader;
use crate::sources::{self, Sources};
use crate::tracking::{self, AniList};
use crate::ui;
use crate::updates::{self, Progress, Report};
use material_3_ui::sys::task::Task;
use material_3_ui::sys::{cmd, paths};
use material_3_ui::theme::{M3Palette, ThemeManager};
use material_3_ui::toasts::{Toast, ToastKind, Toasts};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowseTab {
    Sources,
    Extensions,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Library,
    Updates,
    History,
    Browse(BrowseTab),
    /// One source's popular, latest and search results.
    Catalogue(String),
    GlobalSearch,
    Series(i64),
    Downloads,
    Settings,
}

#[derive(Clone, Debug)]
pub enum Action {
    Goto(Page),
    Back,
    /// Open a series a source listed (storing it first).
    OpenSeries { source: String, info: MangaInfo },
    Read { manga: i64, chapter: i64 },
    /// Resume where the series was left, or start it.
    Continue(i64),
    SetInLibrary(i64, bool),
    Download(i64, Vec<i64>),
    CancelDownload(i64),
    DeleteDownloads(i64, Vec<i64>),
    SetRead(Vec<i64>, bool),
    SetBookmark(Vec<i64>, bool),
    RefreshSeries(i64),
    UpdateLibrary,
    ShowInFiles(PathBuf),
    OpenUrl(String),
    EditCategories(i64),
    Toast(String, ToastKind),
    RefreshSources,
}

/// The categories dialog for one series.
pub struct CategoryDialog {
    pub manga: i64,
    pub chosen: Vec<i64>,
    pub new_name: String,
}

pub struct App {
    pub ctx: egui::Context,
    pub cfg: Config,
    pub theme: ThemeManager,
    pub palette: M3Palette,
    pub toasts: Toasts<Action>,
    pub db: Arc<Db>,
    pub sources: Arc<Sources>,
    pub covers: Covers,
    pub downloads: Downloads,
    pub anilist: Arc<AniList>,
    pub page: Page,
    pub back: Vec<Page>,
    pub actions: Vec<Action>,
    pub reader: Option<Reader>,
    pub max_texture: u32,
    pub pages: ui::States,
    pub category_dialog: Option<CategoryDialog>,

    library: Option<Arc<Vec<LibraryEntry>>>,
    updates_list: Option<Arc<Vec<ChapterWithManga>>>,
    history_list: Option<Arc<Vec<ChapterWithManga>>>,

    pub update_job: Option<(Task<Report>, Arc<Progress>)>,
    pub sources_task: Task<Result<usize, String>>,
    auto_update_at: Option<Instant>,
    jobs: Vec<Task<Result<String, String>>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, open: Option<PathBuf>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let theme = ThemeManager::new(&ctx, &cfg.appearance);
        let max_texture = cc
            .gl
            .as_ref()
            .map(|gl| {
                use eframe::glow::HasContext;
                // SAFETY: a plain query on the current context.
                unsafe { gl.get_parameter_i32(eframe::glow::MAX_TEXTURE_SIZE) as u32 }
            })
            .unwrap_or(8192)
            .clamp(2048, 16384);
        let mut toasts = Toasts::default();
        let db = match Db::open(&crate::config::data_dir().join("library.db")) {
            Ok(db) => Arc::new(db),
            Err(e) => {
                toasts.error(format!("Couldn't open the library ({e}); nothing will be saved"));
                Arc::new(Db::open_in_memory().expect("an in-memory database"))
            }
        };
        let _ = db.prune();
        let sources = Sources::new(&cfg);
        sources.mangadex.set_data_saver(cfg.mangadex_data_saver);
        let downloads = Downloads::new(&ctx, sources.clone(), db.clone(), cfg.downloads_dir());
        let secrets: Arc<dyn material_3_ui::secrets::Secrets> = Arc::new(material_3_ui::secrets::Keyring::new("m3-comics"));
        let mut app = App {
            palette: theme.palette().clone(),
            theme,
            toasts,
            covers: Covers::new(ctx.clone()),
            downloads,
            anilist: Arc::new(AniList::new(secrets)),
            page: Page::Library,
            back: Vec::new(),
            actions: Vec::new(),
            reader: None,
            max_texture,
            pages: ui::States::default(),
            category_dialog: None,
            library: None,
            updates_list: None,
            history_list: None,
            update_job: None,
            sources_task: Task::default(),
            auto_update_at: None,
            jobs: Vec::new(),
            db,
            sources,
            cfg,
            ctx,
        };
        if app.sources.server().is_some() {
            app.refresh_sources();
        }
        if updates::due(&app.db, app.cfg.update_interval_hours) {
            app.auto_update_at = Some(Instant::now() + Duration::from_secs(5));
        }
        if let Some(path) = open {
            app.open_file(&path);
        }
        // Development and screenshots: start on a page.
        if let Ok(p) = std::env::var("M3_COMICS_PAGE") {
            app.page = match p.as_str() {
                "updates" => Page::Updates,
                "history" => Page::History,
                "browse" => Page::Browse(BrowseTab::Sources),
                "extensions" => Page::Browse(BrowseTab::Extensions),
                "downloads" => Page::Downloads,
                "settings" => Page::Settings,
                s => match s.strip_prefix("series:").and_then(|id| id.parse().ok()) {
                    Some(id) => Page::Series(id),
                    None => Page::Library,
                },
            };
        }
        app
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast::new(text, kind));
    }

    /// Something in the database changed: reload lists when next shown.
    pub fn dirty(&mut self) {
        self.library = None;
        self.updates_list = None;
        self.history_list = None;
        self.pages.series.reload = true;
    }

    pub fn library(&mut self) -> Arc<Vec<LibraryEntry>> {
        if self.library.is_none() {
            self.library = Some(Arc::new(self.db.library().unwrap_or_default()));
        }
        self.library.clone().unwrap_or_default()
    }

    pub fn updates_list(&mut self) -> Arc<Vec<ChapterWithManga>> {
        if self.updates_list.is_none() {
            self.updates_list = Some(Arc::new(self.db.updates(500).unwrap_or_default()));
        }
        self.updates_list.clone().unwrap_or_default()
    }

    pub fn history_list(&mut self) -> Arc<Vec<ChapterWithManga>> {
        if self.history_list.is_none() {
            self.history_list = Some(Arc::new(self.db.history(300).unwrap_or_default()));
        }
        self.history_list.clone().unwrap_or_default()
    }

    /// Headers to load a series' covers with (Suwayomi with a password).
    pub fn image_headers(&self, source: &str) -> Vec<(String, String)> {
        if source.starts_with(sources::suwayomi::PREFIX) {
            self.sources.server().map(|c| c.headers()).unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    pub fn goto(&mut self, page: Page) {
        if page != self.page {
            let from = std::mem::replace(&mut self.page, page);
            // Top-level destinations don't stack up.
            if matches!(self.page, Page::Library | Page::Updates | Page::History | Page::Browse(_) | Page::Downloads | Page::Settings) {
                self.back.clear();
            } else {
                self.back.push(from);
            }
            self.on_page_shown();
        }
    }

    fn on_page_shown(&mut self) {
        if let Page::Series(id) = self.page {
            self.pages.series.open(id);
        }
    }

    pub fn go_back(&mut self) {
        if let Some(p) = self.back.pop() {
            self.page = p;
            self.on_page_shown();
        } else if !matches!(self.page, Page::Library) {
            self.page = Page::Library;
        }
    }

    pub fn save_config(&mut self) {
        if let Err(e) = self.cfg.save() {
            self.toast(format!("Couldn't save preferences: {e}"), ToastKind::Error);
        }
    }

    /// Settings changed: hand them to the sources and downloads.
    pub fn config_changed(&mut self) {
        let had_server = self.sources.server().map(|c| c.base.clone());
        self.sources.configure(&self.cfg);
        self.sources.mangadex.set_data_saver(self.cfg.mangadex_data_saver);
        self.downloads.set_root(self.cfg.downloads_dir());
        if self.sources.server().map(|c| c.base.clone()) != had_server && self.sources.server().is_some() {
            self.refresh_sources();
        }
        self.save_config();
    }

    pub fn apply_theme(&mut self) {
        self.theme.apply(&self.cfg.appearance);
        self.theme_applied();
    }

    fn theme_applied(&mut self) {
        if let Some(e) = self.theme.take_error() {
            self.toast(e, ToastKind::Error);
        }
        self.palette = self.theme.palette().clone();
    }

    pub fn refresh_sources(&mut self) {
        let sources = self.sources.clone();
        self.sources_task.spawn(&self.ctx, move || sources.refresh_remote().map_err(|e| e.to_string()));
    }

    /// Install packages in a terminal (pacman or the AUR helper).
    pub fn install(&mut self, packages: &[&str]) {
        let script = material_3_ui::sys::arch::install_command(packages, self.cfg.aur_helper);
        match material_3_ui::sys::term::run(&self.cfg.terminal, &script) {
            Ok(_) => self.toast(format!("Installing {} in a terminal", packages.join(", ")), ToastKind::Info),
            Err(e) => self.toast(e, ToastKind::Error),
        }
    }

    pub fn start_update(&mut self) {
        if self.update_job.is_some() {
            return;
        }
        let progress = Arc::new(Progress::default());
        let (db, sources, p) = (self.db.clone(), self.sources.clone(), progress.clone());
        let mut task = Task::default();
        task.spawn(&self.ctx, move || updates::update_library(&db, &sources, &p));
        self.update_job = Some((task, progress));
    }

    /// A comic file or folder from the command line or a drop.
    pub fn open_file(&mut self, path: &Path) {
        if !path.exists() {
            self.toast(format!("{} doesn't exist", path.display()), ToastKind::Error);
            return;
        }
        let series = sources::local::Local::series_for(path);
        let opened = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let result = (|| -> anyhow::Result<(i64, Option<i64>)> {
            use sources::Source;
            let local = self.sources.local.clone();
            let key = series.display().to_string();
            let info = local.details(&key)?;
            let id = self.db.upsert_manga(sources::local::ID, &info)?;
            let list = local.chapters(&key)?;
            self.db.sync_chapters(id, &list)?;
            let chapters = self.db.chapters(id)?;
            let chosen = chapters.iter().find(|c| Path::new(&c.key) == opened).or(chapters.last()).map(|c| c.id);
            Ok((id, chosen))
        })();
        match result {
            Ok((manga, Some(chapter))) => {
                self.page = Page::Series(manga);
                self.on_page_shown();
                self.open_reader(manga, chapter);
            }
            Ok((manga, None)) => {
                self.page = Page::Series(manga);
                self.on_page_shown();
            }
            Err(e) => self.toast(format!("Can't open {}: {e}", path.display()), ToastKind::Error),
        }
    }

    pub fn open_reader(&mut self, manga_id: i64, chapter_id: i64) {
        let (Ok(Some(manga)), Ok(chapters)) = (self.db.manga(manga_id), self.db.chapters(manga_id)) else {
            self.toast("That series isn't in the library any more", ToastKind::Error);
            return;
        };
        if !chapters.iter().any(|c| c.id == chapter_id) {
            self.toast("That chapter is gone", ToastKind::Error);
            return;
        }
        let mode = manga.reader_mode.unwrap_or(self.cfg.reader.mode);
        self.reader = Some(Reader::new(&self.ctx, manga, chapters, chapter_id, mode, self.max_texture));
        self.start_reader_chapter();
    }

    /// Find the pages of the reader's chapter (downloaded copy first).
    pub fn start_reader_chapter(&mut self) {
        let Some(r) = &mut self.reader else { return };
        let manga = r.manga.clone();
        let chapter = r.chapter().clone();
        let downloaded = self.downloads.path_if_downloaded(&manga, &chapter);
        let sources = self.sources.clone();
        r.load_pages(&self.ctx, move || resolve_pages(&sources, &manga, &chapter, downloaded));
    }

    /// The reader moved: save progress, and act on finished chapters.
    pub fn reader_progress(&mut self, chapter: Chapter, page: usize, pages: usize) {
        let finished = self.db.set_progress(chapter.id, page, pages).unwrap_or(false);
        self.dirty();
        if !finished || chapter.read {
            return;
        }
        let Some(manga) = self.reader.as_ref().map(|r| r.manga.clone()) else { return };
        if self.cfg.delete_after_reading {
            self.downloads.delete(&manga, std::slice::from_ref(&chapter));
        }
        if self.cfg.auto_track && self.anilist.signed_in() {
            if let Ok(Some(t)) = self.db.track(manga.id, tracking::SERVICE) {
                if let Some(next) = tracking::advance(&t, chapter.number) {
                    let (al, db) = (self.anilist.clone(), self.db.clone());
                    let mut task = Task::default();
                    task.spawn(&self.ctx, move || {
                        let saved = al.save(&next).map_err(|e| format!("AniList: {e}"))?;
                        db.set_track(&saved).map_err(|e| e.to_string())?;
                        Ok(String::new())
                    });
                    self.jobs.push(task);
                }
            }
        }
    }

    fn poll_background(&mut self) {
        self.covers.poll();
        for e in self.downloads.poll() {
            match e {
                downloads::Event::Finished => self.dirty(),
                downloads::Event::Failed { title, chapter, error } => {
                    let mut t = Toast::new(format!("Couldn't download {title} {chapter}: {error}"), ToastKind::Error);
                    t.action = Some(("Downloads".into(), Action::Goto(Page::Downloads)));
                    self.toasts.push(t);
                }
            }
        }
        if self.sources_task.poll() {
            if let Some(Err(e)) = &self.sources_task.value {
                let e = e.clone();
                self.toast(e, ToastKind::Error);
            }
        }
        let mut finished = Vec::new();
        self.jobs.retain_mut(|t| match t.take() {
            Some(r) => {
                finished.push(r);
                false
            }
            None => true,
        });
        for r in finished {
            match r {
                Ok(msg) if !msg.is_empty() => self.toast(msg, ToastKind::Success),
                Ok(_) => self.dirty(),
                Err(e) => self.toast(e, ToastKind::Error),
            }
        }
        if let Some((task, _)) = &mut self.update_job {
            if let Some(report) = task.take() {
                self.update_job = None;
                self.update_finished(report);
            }
        }
        if self.auto_update_at.is_some_and(|t| Instant::now() >= t) {
            self.auto_update_at = None;
            self.start_update();
        } else if let Some(t) = self.auto_update_at {
            self.ctx.request_repaint_after(t.saturating_duration_since(Instant::now()));
        }
    }

    fn update_finished(&mut self, report: Report) {
        self.dirty();
        let n = report.new_chapters();
        if n > 0 {
            if self.cfg.download_new_chapters {
                let items: Vec<(Manga, Chapter)> = report.new.iter().flat_map(|(m, cs)| cs.iter().map(move |c| (m.clone(), c.clone()))).collect();
                self.downloads.enqueue(items);
            }
            if self.cfg.notify_new_chapters && !self.ctx.input(|i| i.focused) {
                updates::notify(&report);
            }
            let mut t = Toast::new(report.summary(), ToastKind::Success);
            t.action = Some(("View".into(), Action::Goto(Page::Updates)));
            self.toasts.push(t);
        } else if self.page == Page::Updates {
            self.toast("No new chapters", ToastKind::Info);
        }
        if !report.errors.is_empty() {
            let (title, err) = &report.errors[0];
            let more = if report.errors.len() > 1 { format!(" (and {} more)", report.errors.len() - 1) } else { String::new() };
            self.toast(format!("Couldn't check {title}: {err}{more}"), ToastKind::Error);
        }
    }

    pub fn process_actions(&mut self) {
        let actions = std::mem::take(&mut self.actions);
        for a in actions {
            self.act(a);
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Goto(p) => self.goto(p),
            Action::Back => self.go_back(),
            Action::OpenSeries { source, info } => match self.db.upsert_manga(&source, &info) {
                Ok(id) => self.goto(Page::Series(id)),
                Err(e) => self.toast(format!("Couldn't open {}: {e}", info.title), ToastKind::Error),
            },
            Action::Read { manga, chapter } => self.open_reader(manga, chapter),
            Action::Continue(manga) => {
                let chapters = self.db.chapters(manga).unwrap_or_default();
                match continue_chapter(&chapters) {
                    Some(c) => self.open_reader(manga, c.id),
                    None => self.toast("No chapters to read yet", ToastKind::Info),
                }
            }
            Action::SetInLibrary(id, on) => {
                if let Err(e) = self.db.set_in_library(id, on) {
                    self.toast(e.to_string(), ToastKind::Error);
                }
                if on && !self.db.categories().unwrap_or_default().is_empty() && self.db.manga_categories(id).unwrap_or_default().is_empty() {
                    self.act(Action::EditCategories(id));
                }
                self.dirty();
            }
            Action::Download(manga, ids) => {
                let Ok(Some(m)) = self.db.manga(manga) else { return };
                let chapters = self.db.chapters(manga).unwrap_or_default();
                let items: Vec<(Manga, Chapter)> = chapters.into_iter().filter(|c| ids.contains(&c.id)).map(|c| (m.clone(), c)).collect();
                let n = self.downloads.enqueue(items);
                if n > 1 {
                    self.toast(format!("Downloading {n} chapters"), ToastKind::Info);
                }
            }
            Action::CancelDownload(id) => self.downloads.cancel(id),
            Action::DeleteDownloads(manga, ids) => {
                let Ok(Some(m)) = self.db.manga(manga) else { return };
                let chapters: Vec<Chapter> = self.db.chapters(manga).unwrap_or_default().into_iter().filter(|c| ids.contains(&c.id)).collect();
                let n = self.downloads.delete(&m, &chapters);
                if n > 0 {
                    self.toast(format!("Deleted {n} download{}", if n == 1 { "" } else { "s" }), ToastKind::Info);
                }
            }
            Action::SetRead(ids, read) => {
                let _ = self.db.set_read(&ids, read);
                self.dirty();
            }
            Action::SetBookmark(ids, on) => {
                let _ = self.db.set_bookmark(&ids, on);
                self.dirty();
            }
            Action::RefreshSeries(id) => self.pages.series.refresh(&self.ctx, &self.db, &self.sources, id),
            Action::UpdateLibrary => self.start_update(),
            Action::ShowInFiles(path) => {
                if let Err(e) = show_in_files(&path) {
                    self.toast(e, ToastKind::Error);
                }
            }
            Action::OpenUrl(url) => {
                if let Err(e) = cmd::spawn_detached("xdg-open", &[&url]) {
                    self.toast(format!("Couldn't open the browser: {e}"), ToastKind::Error);
                }
            }
            Action::EditCategories(id) => {
                let chosen = self.db.manga_categories(id).unwrap_or_default();
                self.category_dialog = Some(CategoryDialog { manga: id, chosen, new_name: String::new() });
            }
            Action::Toast(text, kind) => self.toast(text, kind),
            Action::RefreshSources => self.refresh_sources(),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        use material_3_ui::keys::take_key;
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        if take_key(ctx, Modifiers::ALT, Key::ArrowLeft) || take_key(ctx, Modifiers::NONE, Key::Backspace) {
            self.go_back();
        }
        if take_key(ctx, Modifiers::CTRL, Key::F) || take_key(ctx, Modifiers::NONE, Key::Slash) {
            self.pages.focus_search = true;
        }
        if take_key(ctx, Modifiers::CTRL, Key::R) {
            match self.page {
                Page::Series(id) => self.act(Action::RefreshSeries(id)),
                _ => self.start_update(),
            }
        }
        for (i, page) in [Page::Library, Page::Updates, Page::History, Page::Browse(BrowseTab::Sources), Page::Downloads].into_iter().enumerate() {
            let key = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5][i];
            if take_key(ctx, Modifiers::CTRL, key) {
                self.goto(page);
            }
        }
    }

    fn dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = dropped.first() {
            self.open_file(p);
        }
    }
}

/// The chapter to continue with: the one in progress, else the first
/// unread one after the last read one, else the first.
pub fn continue_chapter(chapters: &[Chapter]) -> Option<&Chapter> {
    // `chapters` is newest first; reading goes the other way.
    let reading: Vec<&Chapter> = chapters.iter().rev().collect();
    if let Some(c) = reading.iter().filter(|c| c.in_progress()).max_by_key(|c| c.read_at) {
        return Some(c);
    }
    match reading.iter().rposition(|c| c.read) {
        Some(i) => reading.get(i + 1).or(reading.get(i)).copied(),
        None => reading.first().copied(),
    }
}

/// A chapter's pages: from its download when there is one.
pub fn resolve_pages(sources: &Sources, manga: &Manga, chapter: &Chapter, downloaded: Option<PathBuf>) -> Result<Vec<crate::model::PageRef>, String> {
    use crate::model::PageRef;
    if let Some(path) = downloaded {
        if let Ok(entries) = crate::archive::list(&path) {
            if !entries.is_empty() {
                return Ok(entries.into_iter().map(|entry| PageRef::Archive { path: path.clone(), entry }).collect());
            }
        }
    }
    let source = sources.get(&manga.source).ok_or_else(|| format!("{} isn't available. Is the Suwayomi server running?", sources.name_of(&manga.source)))?;
    let pages = source.pages(manga.key(), &chapter.key).map_err(|e| e.to_string())?;
    if pages.is_empty() {
        return Err("This chapter has no pages".into());
    }
    Ok(pages)
}

/// Open a folder in the Material 3 file manager, or the default one.
pub fn show_in_files(path: &Path) -> Result<(), String> {
    let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
    let dir = dir.to_string_lossy().to_string();
    let local = paths::home().join(".local/bin/m3fm");
    if cmd::exists("m3fm") {
        cmd::spawn_detached("m3fm", &[&dir])
    } else if local.is_file() {
        cmd::spawn_detached(&local.to_string_lossy(), &[&dir])
    } else {
        cmd::spawn_detached("xdg-open", &[&dir])
    }
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background();
        if self.theme.poll(&self.cfg.appearance) {
            self.theme_applied();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = self.ctx.clone();
        self.dropped_files(&ctx);
        if self.reader.is_some() {
            crate::reader::show(ui, self);
        } else {
            self.shortcuts(&ctx);
            ui::layout(ui, self);
            ui::dialogs(&ctx, self);
        }
        let bottom = if self.reader.is_some() { 96.0 } else { 20.0 };
        let fired = self.toasts.show(&ctx, &self.palette, bottom);
        self.actions.extend(fired);
        self.process_actions();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.palette.surface.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.cfg.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(id: i64, read: bool, last_page: i64, read_at: i64) -> Chapter {
        Chapter { id, read, last_page, read_at, ..Default::default() }
    }

    #[test]
    fn continues_where_reading_stopped() {
        // Newest first, like the database lists them.
        let list = vec![ch(3, false, 0, 0), ch(2, false, 0, 0), ch(1, true, 0, 5)];
        assert_eq!(continue_chapter(&list).unwrap().id, 2);
        let list = vec![ch(3, false, 4, 9), ch(2, false, 0, 0), ch(1, true, 0, 5)];
        assert_eq!(continue_chapter(&list).unwrap().id, 3);
        let list = vec![ch(2, false, 0, 0), ch(1, false, 0, 0)];
        assert_eq!(continue_chapter(&list).unwrap().id, 1);
        let list = vec![ch(2, true, 0, 0), ch(1, true, 0, 0)];
        assert_eq!(continue_chapter(&list).unwrap().id, 2, "all read: the newest");
        assert!(continue_chapter(&[]).is_none());
    }
}
