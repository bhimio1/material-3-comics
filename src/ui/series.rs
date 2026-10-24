//! A series: cover and details, library and tracking buttons, and the
//! chapter list with read state, bookmarks, downloads and selection.

use super::updates::download_button;
use super::{header, headers_for, plural};
use crate::app::{continue_chapter, Action, App};
use crate::covers;
use crate::db::{Chapter, Db, Manga, Track};
use crate::model::{now, relative_date};
use crate::sources::Sources;
use crate::tracking::{self, Media};
use egui::{pos2, vec2, Rect, RichText, Sense, Ui};
use material_3_ui::common;
use material_3_ui::icons::icons::*;
use material_3_ui::sys::task::Task;
use material_3_ui::toasts::ToastKind;
use material_3_ui::widgets::{self, ButtonKind};
use std::collections::HashSet;
use std::sync::Arc;

pub struct TrackSearch {
    pub query: String,
    pub results: Task<Result<Vec<Media>, String>>,
    pub link: Task<Result<Track, String>>,
}

pub struct State {
    pub id: i64,
    pub reload: bool,
    pub manga: Option<Manga>,
    pub chapters: Vec<Chapter>,
    pub refresh: Task<Result<usize, String>>,
    refreshing: Option<i64>,
    /// Series refreshed from their source this session.
    refreshed: HashSet<i64>,
    pub oldest_first: bool,
    pub unread_only: bool,
    pub downloaded_only: bool,
    pub bookmarked_only: bool,
    pub expanded: bool,
    pub track: Option<Track>,
    pub track_search: Option<TrackSearch>,
    pub track_save: Task<Result<Track, String>>,
    pub selected: HashSet<i64>,
    last_clicked: Option<i64>,
}

impl Default for State {
    fn default() -> Self {
        State {
            id: 0,
            reload: true,
            manga: None,
            chapters: Vec::new(),
            refresh: Task::default(),
            refreshing: None,
            refreshed: HashSet::new(),
            oldest_first: false,
            unread_only: false,
            downloaded_only: false,
            bookmarked_only: false,
            expanded: false,
            track: None,
            track_search: None,
            track_save: Task::default(),
            selected: HashSet::new(),
            last_clicked: None,
        }
    }
}

impl State {
    pub fn open(&mut self, id: i64) {
        if self.id != id {
            self.id = id;
            self.expanded = false;
            self.selected.clear();
            self.track_search = None;
            self.last_clicked = None;
        }
        self.reload = true;
    }

    /// Ask the source for fresh details and chapters.
    pub fn refresh(&mut self, ctx: &egui::Context, db: &Arc<Db>, sources: &Arc<Sources>, id: i64) {
        if self.refresh.running() {
            return;
        }
        let Ok(Some(manga)) = db.manga(id) else { return };
        let (db, sources) = (db.clone(), sources.clone());
        self.refreshing = Some(id);
        self.refreshed.insert(id);
        self.refresh.spawn(ctx, move || crate::updates::refresh(&db, &sources, &manga).map(|n| n.len()).map_err(|e| e.to_string()));
    }

    fn load(&mut self, db: &Db) {
        self.manga = db.manga(self.id).ok().flatten();
        self.chapters = db.chapters(self.id).unwrap_or_default();
        self.track = db.track(self.id, tracking::SERVICE).ok().flatten();
        let ids: HashSet<i64> = self.chapters.iter().map(|c| c.id).collect();
        self.selected.retain(|id| ids.contains(id));
        self.reload = false;
    }
}

pub fn show(ui: &mut Ui, app: &mut App, id: i64) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let ctx = app.ctx.clone();
    {
        let st = &mut app.pages.series;
        if st.id != id {
            st.open(id);
        }
        if let Some(r) = st.refresh.take() {
            match r {
                Ok(n) if n > 0 => actions.push(Action::Toast(format!("{} new", plural(n, "chapter")), ToastKind::Success)),
                Ok(_) => {}
                Err(e) => actions.push(Action::Toast(e, ToastKind::Error)),
            }
            st.refreshing = None;
            st.reload = true;
            app.dirty();
        }
    }
    if app.pages.series.reload {
        app.pages.series.load(&app.db);
        // Fetch chapters the first time a series is opened in a session.
        let st = &app.pages.series;
        if st.manga.is_some() && !st.refreshed.contains(&id) && (st.chapters.is_empty() || st.manga.as_ref().is_some_and(|m| m.source != crate::sources::local::ID || m.last_update == 0)) {
            let (db, sources) = (app.db.clone(), app.sources.clone());
            app.pages.series.refresh(&ctx, &db, &sources, id);
        }
    }
    poll_tracking(app, &mut actions);
    let Some(manga) = app.pages.series.manga.clone() else {
        header(ui, ICON_AUTO_STORIES, "Not found", "", |_| {});
        common::empty_hint(ui, "This series isn't in the database any more");
        return;
    };
    let busy = app.pages.series.refresh.running() && app.pages.series.refreshing == Some(id);
    let chapters = app.pages.series.chapters.clone();
    let headers = headers_for(&manga.source, &app.image_headers(crate::sources::suwayomi::PREFIX));
    let source_name = app.sources.name_of(&manga.source);

    egui::ScrollArea::vertical().id_salt(("series", id)).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 24, right: 24, top: 16, bottom: 24 }).show(ui, |ui| {
            // Top bar.
            ui.horizontal(|ui| {
                if widgets::icon_button(ui, ICON_ARROW_BACK, "Back (Alt+←)", true).clicked() {
                    actions.push(Action::Back);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    more_menu(ui, app, &manga, &chapters, &mut actions);
                    if common::refresh_button(ui, busy) {
                        actions.push(Action::RefreshSeries(id));
                    }
                    if let Some(url) = &manga.info.web_url {
                        if url.starts_with("http") && widgets::icon_button(ui, ICON_OPEN_IN_BROWSER, "Open in browser", true).clicked() {
                            actions.push(Action::OpenUrl(url.clone()));
                        }
                    }
                });
            });
            ui.add_space(8.0);
            // Cover and details.
            ui.horizontal_top(|ui| {
                let cover_w = 190.0;
                let (rect, _) = ui.allocate_exact_size(vec2(cover_w, cover_w * 1.5), Sense::hover());
                covers::paint(ui, &mut app.covers, manga.info.cover.as_deref(), &headers, rect, 18, ICON_AUTO_STORIES);
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(RichText::new(manga.title()).size(26.0).color(p.on_surface)).wrap());
                    ui.add_space(4.0);
                    let people: Vec<&str> = [manga.info.author.as_str(), manga.info.artist.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
                    let mut people = people;
                    people.dedup();
                    if !people.is_empty() {
                        ui.label(RichText::new(people.join(" · ")).size(14.0).color(p.on_surface_variant));
                    }
                    ui.horizontal(|ui| {
                        widgets::icon(ui, status_icon(manga.info.status), 16.0, p.on_surface_variant);
                        ui.label(RichText::new(format!("{} · {}", manga.info.status.label(), source_name)).size(13.0).color(p.on_surface_variant));
                    });
                    ui.add_space(14.0);
                    ui.horizontal_wrapped(|ui| {
                        let next = continue_chapter(&chapters);
                        let started = chapters.iter().any(|c| c.read || c.read_at > 0);
                        let label = match next {
                            Some(c) if started => format!("Continue: {}", short(&c.name)),
                            Some(_) => "Start reading".to_string(),
                            None if busy => "Loading chapters…".to_string(),
                            None => "No chapters".to_string(),
                        };
                        if widgets::button_enabled(ui, ButtonKind::Filled, Some(ICON_PLAY_ARROW), &label, next.is_some()).clicked() {
                            actions.push(Action::Continue(id));
                        }
                        if manga.in_library {
                            if widgets::button(ui, ButtonKind::Tonal, Some(ICON_BOOKMARK_ADDED), "In library").on_hover_text("Remove from the library").clicked() {
                                actions.push(Action::SetInLibrary(id, false));
                            }
                        } else if widgets::button(ui, ButtonKind::Outlined, Some(ICON_BOOKMARK_ADD), "Add to library").clicked() {
                            actions.push(Action::SetInLibrary(id, true));
                        }
                        let tracked = app.pages.series.track.clone();
                        let track_label = match &tracked {
                            Some(t) => format!("{} · {}", tracking::status_label(&t.status), progress_text(t)),
                            None => "Track".to_string(),
                        };
                        if widgets::button(ui, ButtonKind::Outlined, Some(ICON_SYNC), &track_label).on_hover_text("Track progress on AniList").clicked() {
                            if !app.anilist.signed_in() {
                                actions.push(Action::Toast("Sign in to AniList in Preferences first".into(), ToastKind::Info));
                                actions.push(Action::Goto(crate::app::Page::Settings));
                            } else {
                                let query = manga.title().to_string();
                                let al = app.anilist.clone();
                                let mut results = Task::default();
                                let q = query.clone();
                                results.spawn(&ctx, move || al.search(&q).map_err(|e| e.to_string()));
                                app.pages.series.track_search = Some(TrackSearch { query, results, link: Task::default() });
                            }
                        }
                        if manga.in_library && widgets::icon_button(ui, ICON_LABEL, "Categories", true).clicked() {
                            actions.push(Action::EditCategories(id));
                        }
                    });
                    if let Some(t) = app.pages.series.track.clone() {
                        ui.add_space(8.0);
                        track_controls(ui, app, &t, &mut actions);
                    }
                });
            });
            // Description and genres.
            if !manga.info.description.is_empty() {
                ui.add_space(16.0);
                let st = &mut app.pages.series;
                let text = if st.expanded || manga.info.description.len() < 320 {
                    manga.info.description.clone()
                } else {
                    let cut: String = manga.info.description.chars().take(300).collect();
                    format!("{}…", cut.trim_end())
                };
                ui.add(egui::Label::new(RichText::new(text).size(14.0).color(p.on_surface)).wrap());
                if manga.info.description.len() >= 320 && widgets::button(ui, ButtonKind::Text, None, if st.expanded { "Less" } else { "More" }).clicked() {
                    st.expanded = !st.expanded;
                }
            }
            if !manga.info.genres.is_empty() {
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    for g in &manga.info.genres {
                        widgets::filter_chip(ui, g, false, None);
                    }
                });
            }
            ui.add_space(18.0);
            chapter_list(ui, app, &manga, &chapters, busy, &mut actions);
        });
    });
    track_dialog(&ctx, app, &manga, &mut actions);
    app.actions.extend(actions);
}

fn short(name: &str) -> String {
    let s: String = name.chars().take(28).collect();
    if s.len() < name.len() {
        format!("{}…", s.trim_end())
    } else {
        s
    }
}

fn status_icon(s: crate::model::Status) -> material_3_ui::icons::MaterialIcon {
    use crate::model::Status::*;
    match s {
        Ongoing => ICON_SCHEDULE,
        Completed => ICON_DONE_ALL,
        Hiatus => ICON_PAUSE,
        Cancelled => ICON_CLOSE,
        Licensed => ICON_STAR,
        Unknown => ICON_INFO,
    }
}

fn progress_text(t: &Track) -> String {
    if t.total > 0 {
        format!("{}/{}", t.progress, t.total)
    } else {
        format!("{}", t.progress)
    }
}

fn more_menu(ui: &mut Ui, app: &mut App, manga: &Manga, chapters: &[Chapter], actions: &mut Vec<Action>) {
    let id = manga.id;
    ui.menu_button(widgets::icon_text(ICON_MORE_VERT, 22.0), |ui| {
        let unread: Vec<i64> = chapters.iter().filter(|c| !c.read).map(|c| c.id).collect();
        let all: Vec<i64> = chapters.iter().map(|c| c.id).collect();
        // Oldest unread first, like Mihon's "next 5".
        let next_unread: Vec<i64> = chapters.iter().rev().filter(|c| !c.read).map(|c| c.id).collect();
        if manga.source != crate::sources::local::ID {
            if widgets::menu_item(ui, Some(ICON_DOWNLOAD), "Download next 5 unread", "", false) {
                actions.push(Action::Download(id, next_unread.iter().take(5).copied().collect()));
            }
            if widgets::menu_item(ui, Some(ICON_DOWNLOAD), "Download unread", "", false) {
                actions.push(Action::Download(id, unread.clone()));
            }
            if widgets::menu_item(ui, Some(ICON_DOWNLOAD_FOR_OFFLINE), "Download all", "", false) {
                actions.push(Action::Download(id, all.clone()));
            }
            if widgets::menu_item(ui, Some(ICON_DELETE), "Delete downloads", "", false) {
                actions.push(Action::DeleteDownloads(id, all.clone()));
            }
            ui.separator();
        }
        if widgets::menu_item(ui, Some(ICON_DONE_ALL), "Mark all as read", "", false) {
            actions.push(Action::SetRead(all.clone(), true));
        }
        if widgets::menu_item(ui, Some(ICON_REMOVE_DONE), "Mark all as unread", "", false) {
            actions.push(Action::SetRead(all.clone(), false));
        }
        ui.separator();
        ui.label(RichText::new("Reading mode for this series").size(12.5).color(widgets::palette(ui).on_surface_variant));
        if widgets::menu_item(ui, None, "Default", "", manga.reader_mode.is_none()) {
            let _ = app.db.set_reader_mode(id, None);
            app.pages.series.reload = true;
        }
        for m in crate::config::ReadingMode::ALL {
            if widgets::menu_item(ui, None, m.label(), "", manga.reader_mode == Some(m)) {
                let _ = app.db.set_reader_mode(id, Some(m));
                app.pages.series.reload = true;
            }
        }
        ui.separator();
        let dir = crate::downloads::manga_dir(app.downloads.root(), &manga.source, manga.title());
        let local_dir = (manga.source == crate::sources::local::ID).then(|| std::path::PathBuf::from(manga.key()));
        if let Some(d) = local_dir.or(dir.exists().then_some(dir)) {
            if widgets::menu_item(ui, Some(ICON_FOLDER_OPEN), "Show in Files", "", false) {
                actions.push(Action::ShowInFiles(d));
            }
        }
    });
}

fn track_controls(ui: &mut Ui, app: &mut App, t: &Track, actions: &mut Vec<Action>) {
    let p = widgets::palette(ui);
    egui::Frame::new().fill(p.surface_container).corner_radius(egui::CornerRadius::same(16)).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            widgets::icon(ui, ICON_SYNC, 18.0, p.primary);
            ui.label(RichText::new(format!("AniList: {}", t.title)).size(13.5).color(p.on_surface));
            let mut next = t.clone();
            let mut changed = false;
            egui::ComboBox::from_id_salt("track-status").selected_text(tracking::status_label(&t.status)).show_ui(ui, |ui| {
                for (k, label) in tracking::STATUSES {
                    if ui.selectable_label(t.status == *k, *label).clicked() {
                        next.status = k.to_string();
                        changed = true;
                    }
                }
            });
            if widgets::icon_button(ui, ICON_REMOVE, "One chapter less", t.progress > 0).clicked() {
                next.progress -= 1;
                changed = true;
            }
            ui.label(RichText::new(progress_text(t)).size(13.5).color(p.on_surface));
            if widgets::icon_button(ui, ICON_ADD, "One chapter more", true).clicked() {
                next.progress += 1;
                changed = true;
            }
            let mut score = t.score as f32;
            ui.label(RichText::new("Score").size(12.5).color(p.on_surface_variant));
            if ui.add(egui::DragValue::new(&mut score).range(0.0..=10.0).speed(0.1).fixed_decimals(1)).drag_stopped() {
                next.score = score as f64;
                changed = true;
            }
            if !t.url.is_empty() && widgets::icon_button(ui, ICON_OPEN_IN_NEW, "Open on AniList", true).clicked() {
                actions.push(Action::OpenUrl(t.url.clone()));
            }
            if widgets::icon_button(ui, ICON_LINK_OFF, "Stop tracking", true).clicked() {
                let _ = app.db.remove_track(t.manga_id, tracking::SERVICE);
                app.pages.series.reload = true;
            }
            if app.pages.series.track_save.running() {
                ui.add(egui::Spinner::new().color(p.primary).size(16.0));
            }
            if changed {
                let al = app.anilist.clone();
                app.pages.series.track_save.spawn(&app.ctx, move || al.save(&next).map_err(|e| e.to_string()));
            }
        });
    });
}

fn poll_tracking(app: &mut App, actions: &mut Vec<Action>) {
    if let Some(r) = app.pages.series.track_save.take() {
        match r {
            Ok(t) => {
                let _ = app.db.set_track(&t);
                app.pages.series.reload = true;
            }
            Err(e) => actions.push(Action::Toast(e, ToastKind::Error)),
        }
    }
    let Some(ts) = &mut app.pages.series.track_search else { return };
    ts.results.poll();
    if let Some(r) = ts.link.take() {
        match r {
            Ok(t) => {
                let _ = app.db.set_track(&t);
                actions.push(Action::Toast(format!("Tracking on AniList as {}", t.title), ToastKind::Success));
                app.pages.series.track_search = None;
                app.pages.series.reload = true;
            }
            Err(e) => actions.push(Action::Toast(e, ToastKind::Error)),
        }
    }
}

fn track_dialog(ctx: &egui::Context, app: &mut App, manga: &Manga, actions: &mut Vec<Action>) {
    let p = app.palette.clone();
    let al = app.anilist.clone();
    let Some(ts) = &mut app.pages.series.track_search else { return };
    let mut close = false;
    let mut chosen: Option<Media> = None;
    let mut search = false;
    common::modal(ctx, &p, "track", ICON_SYNC, "Track on AniList", |ui| {
        ui.set_width(520.0);
        ui.horizontal(|ui| {
            let r = common::search_field(ui, &mut ts.query, "Search AniList", 440.0);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                search = true;
            }
            if widgets::icon_button(ui, ICON_SEARCH, "Search", true).clicked() {
                search = true;
            }
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| match &ts.results.value {
            None => common::loading(ui, "Searching…"),
            Some(Err(e)) => common::banner(ui, ICON_ERROR, e, true, |_| {}),
            Some(Ok(list)) if list.is_empty() => common::empty_hint(ui, "No matches"),
            Some(Ok(list)) => {
                for m in list {
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
                    if r.hovered() {
                        ui.painter().rect_filled(rect, egui::CornerRadius::same(12), p.on_surface.gamma_multiply(0.06));
                    }
                    let cover = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(36.0, 52.0));
                    covers::paint(ui, &mut app.covers, m.cover.as_deref(), &[], cover, 6, ICON_AUTO_STORIES);
                    let x = cover.right() + 12.0;
                    let w = rect.right() - x - 8.0;
                    let tg = widgets::truncated_galley(ui, &m.title, widgets::title_font(14.0), p.on_surface, w);
                    ui.painter().galley(pos2(x, rect.center().y - tg.size().y - 1.0), tg, p.on_surface);
                    let mut bits = vec![m.format.replace('_', " ").to_lowercase()];
                    if let Some(y) = m.year {
                        bits.push(y.to_string());
                    }
                    if let Some(c) = m.chapters {
                        bits.push(plural(c as usize, "chapter"));
                    }
                    bits.push(m.status.replace('_', " ").to_lowercase());
                    let sg = widgets::truncated_galley(ui, &bits.join(" · "), egui::FontId::proportional(12.5), p.on_surface_variant, w);
                    ui.painter().galley(pos2(x, rect.center().y + 2.0), sg, p.on_surface_variant);
                    if r.clicked() {
                        chosen = Some(m.clone());
                    }
                }
            }
        });
        common::dialog_buttons(ui, |ui| {
            if ts.link.running() {
                ui.add(egui::Spinner::new().color(p.primary));
            }
            if widgets::button(ui, ButtonKind::Text, None, "Cancel").clicked() {
                close = true;
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
    });
    if search && !ts.query.trim().is_empty() {
        let q = ts.query.trim().to_string();
        let al = al.clone();
        ts.results.value = None;
        ts.results.spawn(ctx, move || al.search(&q).map_err(|e| e.to_string()));
    }
    if let Some(m) = chosen {
        let id = manga.id;
        ts.link.spawn(ctx, move || al.link(id, &m).map_err(|e| e.to_string()));
    }
    if close {
        app.pages.series.track_search = None;
    }
    let _ = actions;
}

fn chapter_list(ui: &mut Ui, app: &mut App, manga: &Manga, chapters: &[Chapter], busy: bool, actions: &mut Vec<Action>) {
    let p = app.palette.clone();
    let id = manga.id;
    let unread = chapters.iter().filter(|c| !c.read).count();
    ui.horizontal(|ui| {
        let mut title = plural(chapters.len(), "chapter");
        if unread > 0 && unread < chapters.len() {
            title.push_str(&format!(" · {unread} unread"));
        }
        ui.label(widgets::title_text(title, 17.0).color(p.on_surface));
        if busy {
            ui.add(egui::Spinner::new().color(p.primary).size(16.0));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let st = &mut app.pages.series;
            let sort_tip = if st.oldest_first { "Oldest first" } else { "Newest first" };
            if widgets::icon_button(ui, if st.oldest_first { ICON_ARROW_UPWARD } else { ICON_ARROW_DOWNWARD }, sort_tip, true).clicked() {
                st.oldest_first = !st.oldest_first;
            }
            if widgets::filter_chip(ui, "Bookmarked", st.bookmarked_only, None).clicked() {
                st.bookmarked_only = !st.bookmarked_only;
            }
            if widgets::filter_chip(ui, "Downloaded", st.downloaded_only, None).clicked() {
                st.downloaded_only = !st.downloaded_only;
            }
            if widgets::filter_chip(ui, "Unread", st.unread_only, None).clicked() {
                st.unread_only = !st.unread_only;
            }
        });
    });
    ui.add_space(6.0);

    // Selection bar.
    let selected: Vec<i64> = app.pages.series.selected.iter().copied().collect();
    if !selected.is_empty() {
        egui::Frame::new().fill(p.secondary_container).corner_radius(egui::CornerRadius::same(16)).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if widgets::icon_button(ui, ICON_CLOSE, "Clear the selection (Esc)", true).clicked() {
                    app.pages.series.selected.clear();
                }
                ui.label(RichText::new(format!("{} selected", selected.len())).color(p.on_secondary_container));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut done = false;
                    let remote = manga.source != crate::sources::local::ID;
                    if remote && widgets::icon_button(ui, ICON_DELETE, "Delete downloads", true).clicked() {
                        actions.push(Action::DeleteDownloads(id, selected.clone()));
                        done = true;
                    }
                    if remote && widgets::icon_button(ui, ICON_DOWNLOAD, "Download", true).clicked() {
                        actions.push(Action::Download(id, selected.clone()));
                        done = true;
                    }
                    if widgets::icon_button(ui, ICON_BOOKMARK, "Bookmark", true).clicked() {
                        let all_marked = chapters.iter().filter(|c| selected.contains(&c.id)).all(|c| c.bookmark);
                        actions.push(Action::SetBookmark(selected.clone(), !all_marked));
                        done = true;
                    }
                    if widgets::icon_button(ui, ICON_REMOVE_DONE, "Mark as unread", true).clicked() {
                        actions.push(Action::SetRead(selected.clone(), false));
                        done = true;
                    }
                    if widgets::icon_button(ui, ICON_DONE_ALL, "Mark as read", true).clicked() {
                        actions.push(Action::SetRead(selected.clone(), true));
                        done = true;
                    }
                    if widgets::button(ui, ButtonKind::Text, None, "Select all").clicked() {
                        app.pages.series.selected = chapters.iter().map(|c| c.id).collect();
                    }
                    if done {
                        app.pages.series.selected.clear();
                    }
                });
            });
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.pages.series.selected.clear();
        }
        ui.add_space(6.0);
    }

    let st = &app.pages.series;
    let (unread_only, downloaded_only, bookmarked_only, oldest_first) = (st.unread_only, st.downloaded_only, st.bookmarked_only, st.oldest_first);
    let mut list: Vec<&Chapter> = chapters
        .iter()
        .filter(|c| !unread_only || !c.read)
        .filter(|c| !bookmarked_only || c.bookmark)
        .collect();
    if downloaded_only {
        list.retain(|c| app.downloads.is_downloaded(manga, c));
    }
    if oldest_first {
        list.reverse();
    }
    if chapters.is_empty() && !busy {
        common::empty_hint(ui, "No chapters");
    }
    let today = now();
    for c in list {
        let selected = app.pages.series.selected.contains(&c.id);
        let width = ui.available_width();
        let (rect, r) = ui.allocate_exact_size(vec2(width, 54.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let bg = if selected { p.secondary_container } else if r.hovered() { p.on_surface.gamma_multiply(0.06) } else { egui::Color32::TRANSPARENT };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(12), bg);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 4.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        download_button(&mut child, app, manga, c, actions);
        let trailing_w = child.min_rect().width();
        let mut x = rect.left() + 12.0;
        if c.bookmark {
            widgets::paint_icon(ui, ICON_BOOKMARK, pos2(x + 9.0, rect.center().y), 18.0, p.primary);
            x += 26.0;
        }
        let max_w = (rect.right() - trailing_w - 20.0 - x).max(40.0);
        let fg = if c.read { p.on_surface.gamma_multiply(0.45) } else { p.on_surface };
        let tg = widgets::truncated_galley(ui, &c.name, widgets::title_font(14.0), fg, max_w);
        ui.painter().galley(pos2(x, rect.center().y - tg.size().y - 1.0), tg, fg);
        let mut bits = Vec::new();
        let date = relative_date(c.uploaded, today);
        if !date.is_empty() {
            bits.push(date);
        }
        if !c.scanlator.is_empty() {
            bits.push(c.scanlator.clone());
        }
        if c.in_progress() && c.pages > 0 {
            bits.push(format!("Page {} of {}", c.last_page + 1, c.pages));
        }
        let sub_fg = if c.read { p.on_surface_variant.gamma_multiply(0.6) } else { p.on_surface_variant };
        let sg = widgets::truncated_galley(ui, &bits.join(" · "), egui::FontId::proportional(12.5), sub_fg, max_w);
        ui.painter().galley(pos2(x, rect.center().y + 2.0), sg, sub_fg);

        let mods = ui.input(|i| i.modifiers);
        if r.clicked() {
            let st = &mut app.pages.series;
            if mods.shift && st.last_clicked.is_some() {
                // Select the range between the last click and this one.
                let a = chapters.iter().position(|x| Some(x.id) == st.last_clicked);
                let b = chapters.iter().position(|x| x.id == c.id);
                if let (Some(a), Some(b)) = (a, b) {
                    for ch in &chapters[a.min(b)..=a.max(b)] {
                        st.selected.insert(ch.id);
                    }
                }
            } else if mods.command || !st.selected.is_empty() {
                if !st.selected.remove(&c.id) {
                    st.selected.insert(c.id);
                }
            } else {
                actions.push(Action::Read { manga: id, chapter: c.id });
            }
            st.last_clicked = Some(c.id);
        }
        r.context_menu(|ui| {
            if widgets::menu_item(ui, Some(if c.read { ICON_REMOVE_DONE } else { ICON_DONE_ALL }), if c.read { "Mark as unread" } else { "Mark as read" }, "", false) {
                actions.push(Action::SetRead(vec![c.id], !c.read));
            }
            if widgets::menu_item(ui, Some(ICON_DONE_ALL), "Mark previous as read", "", false) {
                let older: Vec<i64> = chapters.iter().filter(|x| x.source_order < c.source_order).map(|x| x.id).collect();
                actions.push(Action::SetRead(older, true));
            }
            if widgets::menu_item(ui, Some(ICON_BOOKMARK), if c.bookmark { "Remove bookmark" } else { "Bookmark" }, "", false) {
                actions.push(Action::SetBookmark(vec![c.id], !c.bookmark));
            }
            if widgets::menu_item(ui, Some(ICON_CHECK), "Select", "Ctrl+click", false) {
                app.pages.series.selected.insert(c.id);
                app.pages.series.last_clicked = Some(c.id);
            }
            if let Some(url) = c.web_url.as_ref().filter(|u| u.starts_with("http")) {
                if widgets::menu_item(ui, Some(ICON_OPEN_IN_BROWSER), "Open in browser", "", false) {
                    actions.push(Action::OpenUrl(url.clone()));
                }
            }
        });
    }
}
