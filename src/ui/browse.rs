//! Browse: the sources (built-in ones and the Suwayomi server's), each
//! source's popular / latest / search results, a search across all
//! sources, and the Extensions tab that installs Mihon extensions on the
//! server.

use super::{cover_card, cover_grid, header, headers_for, Badges};
use crate::app::{Action, App, BrowseTab, Page};
use crate::covers;
use crate::model::{MangaInfo, MangaList};
use crate::sources::suwayomi::{Extension, ExtensionAction};
use crate::sources::{language_name, Source};
use egui::{pos2, vec2, Rect, Response, Sense, Ui};
use material_3_ui::common::{self, card, row};
use material_3_ui::icons::icons::*;
use material_3_ui::sys::task::Task;
use material_3_ui::widgets::{self, ButtonKind};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum Listing {
    Popular,
    Latest,
    Search(String),
}

pub struct Catalogue {
    pub listing: Listing,
    pub items: Vec<MangaInfo>,
    pub page: u32,
    pub has_next: bool,
    pub task: Task<Result<MangaList, String>>,
    pub error: Option<String>,
    pub query: String,
}

impl Catalogue {
    fn new(listing: Listing) -> Self {
        let query = match &listing {
            Listing::Search(q) => q.clone(),
            _ => String::new(),
        };
        Catalogue { listing, items: Vec::new(), page: 0, has_next: true, task: Task::default(), error: None, query }
    }

    fn load_next(&mut self, ctx: &egui::Context, source: Arc<dyn Source>) {
        if self.task.running() || !self.has_next {
            return;
        }
        let page = self.page + 1;
        let listing = self.listing.clone();
        self.task.spawn(ctx, move || {
            match &listing {
                Listing::Popular => source.popular(page),
                Listing::Latest => source.latest(page),
                Listing::Search(q) => source.search(q, page),
            }
            .map_err(|e| e.to_string())
        });
    }

    fn poll(&mut self) {
        if let Some(result) = self.task.take() {
            match result {
                Ok(list) => {
                    self.page += 1;
                    self.has_next = list.has_next && !list.items.is_empty();
                    let known: HashSet<String> = self.items.iter().map(|m| m.key.clone()).collect();
                    self.items.extend(list.items.into_iter().filter(|m| !known.contains(&m.key)));
                    self.error = None;
                }
                Err(e) => {
                    self.error = Some(e);
                    self.has_next = false;
                }
            }
        }
    }
}

/// (source id, source name, its results)
type SourceResults = (String, String, Task<Result<MangaList, String>>);

pub struct GlobalSearch {
    pub query: String,
    pub results: Vec<SourceResults>,
}

#[derive(Default)]
pub struct Extensions {
    pub list: Task<Result<Vec<Extension>, String>>,
    pub change: Task<Result<String, String>>,
    pub busy_pkg: Option<String>,
    pub about: Task<Result<String, String>>,
    pub filter: String,
    pub url: String,
    pub user: String,
    pub password: String,
    pub installed: Option<HashSet<String>>,
}

#[derive(Default)]
pub struct State {
    pub global_query: String,
    pub catalogues: HashMap<String, Catalogue>,
    pub global: Option<GlobalSearch>,
    pub ext: Extensions,
}

/// A row with a square icon image, two lines of text and trailing controls.
fn icon_row(ui: &mut Ui, title: &str, subtitle: &str, trailing: impl FnOnce(&mut Ui)) -> (Response, Rect) {
    let p = widgets::palette(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 56.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(14), p.on_surface.gamma_multiply(0.06));
    }
    let icon = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(40.0, 40.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 4.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    trailing(&mut child);
    let left = rect.left() + 60.0;
    let max_w = (rect.right() - child.min_rect().width() - 20.0 - left).max(40.0);
    let tg = widgets::truncated_galley(ui, title, widgets::title_font(14.5), p.on_surface, max_w);
    let ty = if subtitle.is_empty() { rect.center().y - tg.size().y / 2.0 } else { rect.center().y - tg.size().y - 1.0 };
    ui.painter().galley(pos2(left, ty), tg, p.on_surface);
    if !subtitle.is_empty() {
        let sg = widgets::truncated_galley(ui, subtitle, egui::FontId::proportional(12.5), p.on_surface_variant, max_w);
        ui.painter().galley(pos2(left, rect.center().y + 2.0), sg, p.on_surface_variant);
    }
    (response, icon)
}

fn paint_icon_image(ui: &Ui, app: &mut App, url: Option<&str>, rect: Rect, fallback: material_3_ui::icons::MaterialIcon) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let headers = app.image_headers(crate::sources::suwayomi::PREFIX);
    let p = widgets::palette(ui);
    match url {
        Some(u) => covers::paint(ui, &mut app.covers, Some(u), &headers, rect, 10, fallback),
        None => {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(10), p.primary_container);
            widgets::paint_icon(ui, fallback, rect.center(), 22.0, p.on_primary_container);
        }
    }
}

pub fn show(ui: &mut Ui, app: &mut App, tab: BrowseTab) {
    match tab {
        BrowseTab::Sources => sources_tab(ui, app),
        BrowseTab::Extensions => extensions_tab(ui, app),
    }
}

fn source_icon(id: &str) -> material_3_ui::icons::MaterialIcon {
    match id {
        crate::sources::local::ID => ICON_FOLDER,
        crate::sources::mangadex::ID => ICON_AUTO_STORIES,
        _ => ICON_EXTENSION,
    }
}

fn sources_tab(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let all = app.sources.all();
    let loading = app.sources_task.running();
    let server = app.sources.server();
    let mut search = false;
    header(ui, ICON_EXPLORE, "Browse", "Find series in your sources", |ui| {
        if server.is_some() && common::refresh_button(ui, loading) {
            actions.push(Action::RefreshSources);
        }
        let st = &mut app.pages.browse;
        let r = common::search_field(ui, &mut st.global_query, "Search all sources", 300.0);
        if app.pages.focus_search {
            app.pages.focus_search = false;
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !st.global_query.trim().is_empty() {
            search = true;
        }
    });
    if search {
        start_global_search(app);
        actions.push(Action::Goto(Page::GlobalSearch));
    }

    let pinned: Vec<String> = app.cfg.pinned_sources.clone();
    let langs = app.cfg.source_languages.clone();
    let show_nsfw = app.cfg.show_nsfw;
    let mut groups: BTreeMap<String, Vec<Arc<dyn Source>>> = BTreeMap::new();
    let mut pinned_list = Vec::new();
    let mut builtin = Vec::new();
    for s in all {
        if pinned.contains(&s.id()) {
            pinned_list.push(s);
        } else if s.lang() == "local" || s.id() == crate::sources::mangadex::ID {
            builtin.push(s);
        } else if (langs.is_empty() || langs.contains(&s.lang())) && (show_nsfw || !s.nsfw()) {
            groups.entry(s.lang()).or_default().push(s);
        }
    }
    let mut toggle_pin = None;
    egui::ScrollArea::vertical().id_salt("sources").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 20, top: 0, bottom: 16 }).show(ui, |ui| {
            match (&server, &app.sources_task.value) {
                (None, _) => {
                    common::banner(ui, ICON_EXTENSION, "Get every Mihon source: Comics runs Mihon's extensions through a Suwayomi server.", false, |ui| {
                        if widgets::button(ui, ButtonKind::Filled, None, "Set up").clicked() {
                            actions.push(Action::Goto(Page::Browse(BrowseTab::Extensions)));
                        }
                    });
                }
                (Some(_), Some(Err(e))) if !loading => {
                    let e = e.clone();
                    common::banner(ui, ICON_CLOUD_OFF, &format!("Couldn't list the server's sources: {e}"), true, |ui| {
                        if widgets::button(ui, ButtonKind::Tonal, None, "Retry").clicked() {
                            actions.push(Action::RefreshSources);
                        }
                    });
                }
                (Some(_), Some(Ok(0))) if !loading => {
                    common::banner(ui, ICON_EXTENSION, "The Suwayomi server has no sources yet. Install extensions to add some.", false, |ui| {
                        if widgets::button(ui, ButtonKind::Tonal, None, "Extensions").clicked() {
                            actions.push(Action::Goto(Page::Browse(BrowseTab::Extensions)));
                        }
                    });
                }
                _ => {}
            }
            let mut section = |ui: &mut Ui, app: &mut App, title: &str, list: &[Arc<dyn Source>], actions: &mut Vec<Action>| {
                if list.is_empty() {
                    return;
                }
                widgets::section_header(ui, title);
                for s in list {
                    let id = s.id();
                    let is_pinned = pinned.contains(&id);
                    let sub = match s.lang().as_str() {
                        "local" => app.sources.local.folders().iter().map(|f| material_3_ui::sys::paths::tilde(f)).collect::<Vec<_>>().join(", "),
                        "all" => "All languages".into(),
                        l => language_name(l),
                    };
                    let (r, icon) = icon_row(ui, &s.name(), &sub, |ui| {
                        let pin_tip = if is_pinned { "Unpin" } else { "Pin to the top" };
                        if widgets::icon_button_sized(ui, ICON_PUSH_PIN, pin_tip, true, 36.0, is_pinned).clicked() {
                            toggle_pin = Some(id.clone());
                        }
                        if s.supports_latest() && widgets::button(ui, ButtonKind::Text, None, "Latest").clicked() {
                            app.pages.browse.catalogues.insert(id.clone(), Catalogue::new(Listing::Latest));
                            actions.push(Action::Goto(Page::Catalogue(id.clone())));
                        }
                    });
                    paint_icon_image(ui, app, s.icon_url().filter(|_| s.lang() != "all").as_deref(), icon, source_icon(&id));
                    if r.clicked() {
                        app.pages.browse.catalogues.entry(id.clone()).or_insert_with(|| Catalogue::new(Listing::Popular));
                        actions.push(Action::Goto(Page::Catalogue(id.clone())));
                    }
                }
            };
            section(ui, app, "Pinned", &pinned_list, &mut actions);
            section(ui, app, "Built in", &builtin, &mut actions);
            for (lang, list) in &groups {
                section(ui, app, &language_name(lang), list, &mut actions);
            }
            if loading {
                common::loading(ui, "Asking the Suwayomi server for its sources…");
            }
            let _ = &p;
        });
    });
    if let Some(id) = toggle_pin {
        if let Some(i) = app.cfg.pinned_sources.iter().position(|s| *s == id) {
            app.cfg.pinned_sources.remove(i);
        } else {
            app.cfg.pinned_sources.push(id);
        }
        app.save_config();
    }
    app.actions.extend(actions);
}

fn start_global_search(app: &mut App) {
    let query = app.pages.browse.global_query.trim().to_string();
    let langs = app.cfg.source_languages.clone();
    let pinned = app.cfg.pinned_sources.clone();
    let show_nsfw = app.cfg.show_nsfw;
    let mut results = Vec::new();
    for s in app.sources.all() {
        let wanted = pinned.contains(&s.id()) || s.lang() == "local" || s.lang() == "all" || (langs.is_empty() || langs.contains(&s.lang())) && (show_nsfw || !s.nsfw());
        if !wanted {
            continue;
        }
        let mut task = Task::default();
        let q = query.clone();
        let src = s.clone();
        task.spawn(&app.ctx, move || src.search(&q, 1).map_err(|e| e.to_string()));
        results.push((s.id(), s.name(), task));
    }
    // Pinned sources first.
    results.sort_by_key(|(id, _, _)| !pinned.contains(id));
    app.pages.browse.global = Some(GlobalSearch { query, results });
}

pub fn global_search(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let mut again = false;
    header(ui, ICON_TRAVEL_EXPLORE, "Search all sources", "", |ui| {
        let st = &mut app.pages.browse;
        let r = common::search_field(ui, &mut st.global_query, "Search all sources", 300.0);
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !st.global_query.trim().is_empty() {
            again = true;
        }
        if widgets::icon_button(ui, ICON_ARROW_BACK, "Back", true).clicked() {
            actions.push(Action::Back);
        }
    });
    if again {
        start_global_search(app);
    }
    let server = app.image_headers(crate::sources::suwayomi::PREFIX);
    let Some(mut gs) = app.pages.browse.global.take() else {
        common::empty_hint(ui, "Type something and press Enter");
        app.actions.extend(actions);
        return;
    };
    for (_, _, t) in &mut gs.results {
        t.poll();
    }
    // Sources with results first, then the ones still searching.
    gs.results.sort_by_key(|(_, _, t)| match &t.value {
        Some(Ok(l)) if !l.items.is_empty() => 0,
        None => 1,
        _ => 2,
    });
    let card_w = 124.0;
    egui::ScrollArea::vertical().id_salt("global").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 0, bottom: 16 }).show(ui, |ui| {
            for (id, name, task) in &gs.results {
                ui.horizontal(|ui| {
                    ui.label(widgets::title_text(name, 15.0).color(p.on_surface));
                    match &task.value {
                        None => {
                            ui.add(egui::Spinner::new().color(p.primary).size(16.0));
                        }
                        Some(Ok(l)) => {
                            ui.label(egui::RichText::new(format!("{}", l.items.len())).color(p.on_surface_variant));
                        }
                        Some(Err(_)) => {
                            ui.label(egui::RichText::new("Error").color(p.error));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, ICON_CHEVRON_RIGHT, "More from this source", true).clicked() {
                            app.pages.browse.catalogues.insert(id.clone(), Catalogue::new(Listing::Search(gs.query.clone())));
                            actions.push(Action::Goto(Page::Catalogue(id.clone())));
                        }
                    });
                });
                match &task.value {
                    Some(Ok(l)) if !l.items.is_empty() => {
                        let in_lib = app.db.library_keys(id).unwrap_or_default();
                        let headers = headers_for(id, &server);
                        egui::ScrollArea::horizontal().id_salt(("global-row", id)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                for m in &l.items {
                                    let badges = Badges { in_library: in_lib.contains(&m.key), ..Default::default() };
                                    if cover_card(ui, &mut app.covers, m.cover.as_deref(), &headers, &m.title, card_w, badges).clicked() {
                                        actions.push(Action::OpenSeries { source: id.clone(), info: m.clone() });
                                    }
                                }
                            });
                        });
                    }
                    Some(Ok(_)) => common::empty_hint(ui, "No results"),
                    Some(Err(e)) => {
                        ui.label(egui::RichText::new(e).size(12.5).color(p.on_surface_variant));
                    }
                    None => {}
                }
                ui.add_space(14.0);
            }
        });
    });
    app.pages.browse.global = Some(gs);
    app.actions.extend(actions);
}

pub fn catalogue(ui: &mut Ui, app: &mut App, id: &str) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let Some(source) = app.sources.get(id) else {
        header(ui, ICON_EXTENSION, &app.sources.name_of(id), "", |ui| {
            if widgets::icon_button(ui, ICON_ARROW_BACK, "Back", true).clicked() {
                actions.push(Action::Back);
            }
        });
        common::banner(ui, ICON_CLOUD_OFF, "This source isn't available. Is the Suwayomi server set up and running?", true, |_| {});
        app.actions.extend(actions);
        return;
    };
    let ctx = app.ctx.clone();
    let mut cat = app.pages.browse.catalogues.remove(id).unwrap_or_else(|| Catalogue::new(Listing::Popular));
    cat.poll();
    let lang = source.lang();
    let subtitle = match lang.as_str() {
        "local" => "Comic files on this computer".to_string(),
        l => language_name(l),
    };
    let mut new_listing = None;
    header(ui, source_icon(id), &source.name(), &subtitle, |ui| {
        let r = common::search_field(ui, &mut cat.query, "Search", 260.0);
        if app.pages.focus_search {
            app.pages.focus_search = false;
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !cat.query.trim().is_empty() {
            new_listing = Some(Listing::Search(cat.query.trim().to_string()));
        }
        if source.supports_latest() && widgets::filter_chip(ui, "Latest", cat.listing == Listing::Latest, Some(ICON_NEW_RELEASES)).clicked() {
            new_listing = Some(Listing::Latest);
        }
        if widgets::filter_chip(ui, "Popular", cat.listing == Listing::Popular, Some(ICON_STAR)).clicked() {
            new_listing = Some(Listing::Popular);
        }
        if lang == "local" {
            if let Some(dir) = app.sources.local.folders().first() {
                if widgets::icon_button(ui, ICON_FOLDER_OPEN, "Open the folder", true).clicked() {
                    let _ = std::fs::create_dir_all(dir);
                    actions.push(Action::ShowInFiles(dir.clone()));
                }
            }
        }
        if !app.back.is_empty() && widgets::icon_button(ui, ICON_ARROW_BACK, "Back", true).clicked() {
            actions.push(Action::Back);
        }
    });
    if let Some(l) = new_listing {
        if l != cat.listing || matches!(l, Listing::Search(_)) {
            if !matches!(l, Listing::Search(_)) {
                cat.query.clear();
            }
            cat = Catalogue { query: cat.query.clone(), ..Catalogue::new(l) };
        }
    }
    if cat.page == 0 && cat.error.is_none() {
        cat.load_next(&ctx, source.clone());
    }
    if let Some(e) = cat.error.clone() {
        common::banner(ui, ICON_ERROR, &e, true, |ui| {
            if widgets::button(ui, ButtonKind::Tonal, None, "Retry").clicked() {
                cat.has_next = true;
                cat.error = None;
                cat.load_next(&ctx, source.clone());
            }
        });
    }
    if cat.items.is_empty() {
        if cat.task.running() {
            common::loading(ui, "Loading…");
        } else if cat.error.is_none() {
            let hint = if lang == "local" {
                format!(
                    "No comics found. Put each series in its own folder in {} (CBZ, CBR, CB7 or folders of images), or change the folders in Preferences.",
                    app.sources.local.folders().iter().map(|f| material_3_ui::sys::paths::tilde(f)).collect::<Vec<_>>().join(", ")
                )
            } else {
                "Nothing here".to_string()
            };
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.add(egui::Label::new(egui::RichText::new(hint).color(p.on_surface_variant)).wrap());
            });
        }
    } else {
        let in_lib = app.db.library_keys(id).unwrap_or_default();
        let headers = headers_for(id, &app.image_headers(crate::sources::suwayomi::PREFIX));
        let width = app.cfg.cover_width;
        let items = &cat.items;
        let at_end = cover_grid(ui, &format!("catalogue-{id}"), items.len() + usize::from(cat.task.running()), width, |ui, i, w| {
            let Some(m) = items.get(i) else {
                ui.allocate_ui(vec2(w, super::card_height(w)), |ui| {
                    ui.centered_and_justified(|ui| ui.add(egui::Spinner::new().color(p.primary)));
                });
                return;
            };
            let badges = Badges { in_library: in_lib.contains(&m.key), ..Default::default() };
            let r = cover_card(ui, &mut app.covers, m.cover.as_deref(), &headers, &m.title, w, badges);
            if r.clicked() {
                actions.push(Action::OpenSeries { source: id.to_string(), info: m.clone() });
            }
        });
        if at_end && cat.has_next && cat.error.is_none() {
            cat.load_next(&ctx, source.clone());
        }
    }
    app.pages.browse.catalogues.insert(id.to_string(), cat);
    app.actions.extend(actions);
}

fn extensions_tab(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let ctx = app.ctx.clone();
    let server = app.sources.server();
    let ext = &mut app.pages.browse.ext;
    ext.list.poll();
    ext.about.poll();
    if let Some(r) = ext.change.take() {
        ext.busy_pkg = None;
        match r {
            Ok(msg) => {
                actions.push(Action::Toast(msg, material_3_ui::toasts::ToastKind::Success));
                actions.push(Action::RefreshSources);
            }
            Err(e) => actions.push(Action::Toast(e, material_3_ui::toasts::ToastKind::Error)),
        }
        if let Some(c) = server.clone() {
            ext.list.spawn(&ctx, move || c.extensions().map_err(|e| e.to_string()));
        }
    }
    let mut fetch = false;
    header(ui, ICON_EXTENSION, "Extensions", "Mihon's extensions, run by Suwayomi", |ui| {
        if let Some(c) = &server {
            if widgets::icon_button(ui, ICON_OPEN_IN_BROWSER, "Open Suwayomi's web interface", true).clicked() {
                actions.push(Action::OpenUrl(c.web_ui()));
            }
            if common::refresh_button(ui, ext.list.running()) {
                fetch = true;
            }
            common::search_field(ui, &mut ext.filter, "Search extensions", 240.0);
        }
    });

    let Some(client) = server else {
        setup_card(ui, app, &mut actions);
        app.actions.extend(actions);
        return;
    };
    if fetch || (ext.list.value.is_none() && !ext.list.running()) {
        let c = client.clone();
        // A refresh asks the repositories; the first load just lists.
        ext.list.spawn(&ctx, move || if fetch { c.fetch_extensions() } else { c.extensions() }.map_err(|e| e.to_string()));
    }
    if ext.about.value.is_none() && !ext.about.running() {
        let c = client.clone();
        ext.about.spawn(&ctx, move || c.about().map_err(|e| e.to_string()));
    }
    let list = match &ext.list.value {
        Some(Ok(l)) => l.clone(),
        Some(Err(e)) => {
            let e = e.clone();
            common::banner(ui, ICON_CLOUD_OFF, &e.to_string(), true, |ui| {
                if widgets::button(ui, ButtonKind::Tonal, None, "Retry").clicked() {
                    let c = client.clone();
                    ext.list.spawn(&ctx, move || c.extensions().map_err(|e| e.to_string()));
                }
            });
            ui.add_space(8.0);
            setup_card(ui, app, &mut actions);
            app.actions.extend(actions);
            return;
        }
        None => {
            common::loading(ui, "Asking Suwayomi for its extensions…");
            app.actions.extend(actions);
            return;
        }
    };
    let q = ext.filter.to_lowercase();
    let langs = app.cfg.source_languages.clone();
    let show_nsfw = app.cfg.show_nsfw;
    let visible = |e: &&Extension| {
        (q.is_empty() || e.name.to_lowercase().contains(&q) || e.pkg.contains(&q)) && (show_nsfw || !e.nsfw) && (e.installed || langs.is_empty() || langs.contains(&e.lang) || e.lang == "all")
    };
    let updates: Vec<&Extension> = list.iter().filter(|e| e.installed && e.has_update).filter(visible).collect();
    let installed: Vec<&Extension> = list.iter().filter(|e| e.installed && !e.has_update).filter(visible).collect();
    let available: Vec<&Extension> = list.iter().filter(|e| !e.installed).filter(visible).collect();
    let mut change: Option<(String, ExtensionAction)> = None;
    let busy = ext.busy_pkg.clone();
    let about = match &ext.about.value {
        Some(Ok(v)) => format!("Connected to {v} at {}", client.base),
        Some(Err(e)) => format!("{}: {e}", client.base),
        None => format!("Connecting to {}…", client.base),
    };
    egui::ScrollArea::vertical().id_salt("extensions").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 20, top: 0, bottom: 16 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(about).size(12.5).color(p.on_surface_variant));
            });
            if list.is_empty() {
                common::banner(ui, ICON_INFO, "No extensions yet. Add an extension repository in Suwayomi's web interface (Settings → Browse), then refresh.", false, |ui| {
                    if widgets::button(ui, ButtonKind::Tonal, None, "Open Suwayomi").clicked() {
                        actions.push(Action::OpenUrl(client.web_ui()));
                    }
                });
            }
            let mut section = |ui: &mut Ui, app: &mut App, title: &str, items: &[&Extension]| {
                if items.is_empty() {
                    return;
                }
                widgets::section_header(ui, &format!("{title} · {}", items.len()));
                for e in items {
                    let lang = if e.lang == "all" { "All languages".to_string() } else { language_name(&e.lang) };
                    let mut sub = format!("{lang} · {}", e.version);
                    if e.nsfw {
                        sub.push_str(" · 18+");
                    }
                    if e.obsolete {
                        sub.push_str(" · No longer in the repository");
                    }
                    let (_, icon) = icon_row(ui, &e.name, &sub, |ui| {
                        if busy.as_deref() == Some(e.pkg.as_str()) {
                            ui.add(egui::Spinner::new().color(p.primary));
                            return;
                        }
                        let enabled = busy.is_none();
                        if e.installed {
                            if widgets::button_enabled(ui, ButtonKind::Text, None, "Uninstall", enabled).clicked() {
                                change = Some((e.pkg.clone(), ExtensionAction::Uninstall));
                            }
                            if e.has_update && widgets::button_enabled(ui, ButtonKind::Filled, Some(ICON_UPDATE), "Update", enabled).clicked() {
                                change = Some((e.pkg.clone(), ExtensionAction::Update));
                            }
                        } else if widgets::button_enabled(ui, ButtonKind::Tonal, Some(ICON_DOWNLOAD), "Install", enabled).clicked() {
                            change = Some((e.pkg.clone(), ExtensionAction::Install));
                        }
                    });
                    paint_icon_image(ui, app, e.icon_url.as_deref(), icon, ICON_EXTENSION);
                }
            };
            section(ui, app, "Updates", &updates);
            section(ui, app, "Installed", &installed);
            section(ui, app, "Available", &available);
        });
    });
    if let Some((pkg, action)) = change {
        let ext = &mut app.pages.browse.ext;
        ext.busy_pkg = Some(pkg.clone());
        let c = client.clone();
        let name = list.iter().find(|e| e.pkg == pkg).map(|e| e.name.clone()).unwrap_or_else(|| pkg.clone());
        ext.change.spawn(&ctx, move || {
            c.change_extension(&pkg, action).map_err(|e| e.to_string())?;
            Ok(match action {
                ExtensionAction::Install => format!("Installed {name}"),
                ExtensionAction::Update => format!("Updated {name}"),
                ExtensionAction::Uninstall => format!("Uninstalled {name}"),
            })
        });
    }
    app.actions.extend(actions);
}

/// Connecting a Suwayomi server, or installing one.
fn setup_card(ui: &mut Ui, app: &mut App, actions: &mut Vec<Action>) {
    let p = app.palette.clone();
    let mut save = false;
    let mut install = false;
    egui::ScrollArea::vertical().id_salt("ext-setup").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 24, right: 24, top: 0, bottom: 16 }).show(ui, |ui| {
            ui.set_max_width(760.0);
            let ext = &mut app.pages.browse.ext;
            if ext.url.is_empty() && ext.user.is_empty() {
                ext.url = if app.cfg.suwayomi_url.is_empty() { crate::sources::suwayomi::DEFAULT_URL.to_string() } else { app.cfg.suwayomi_url.clone() };
                ext.user = app.cfg.suwayomi_user.clone();
                ext.password = app.cfg.suwayomi_password.clone();
            }
            card(ui, "How it works", |ui| {
                common::note(
                    ui,
                    "Mihon's sources are Android extensions. Suwayomi-Server runs those same extensions on Linux (on this computer or a server at home) and Comics talks to it. \
                     Every extension you install there becomes a source in Browse, and the series you add stay in this app's library.",
                );
            });
            card(ui, "Install Suwayomi", |ui| {
                let installed = ext.installed.get_or_insert_with(material_3_ui::sys::arch::installed_packages);
                let have = material_3_ui::sys::cmd::exists("suwayomi-server") || installed.contains("suwayomi-server-bin") || installed.contains("suwayomi-server");
                if have {
                    row(ui, "Suwayomi-Server is installed", "Start it, then connect below. It keeps running in the background.", |ui| {
                        if widgets::button(ui, ButtonKind::Tonal, Some(ICON_PLAY_ARROW), "Start").clicked() {
                            match material_3_ui::sys::cmd::spawn_detached("suwayomi-server", &[]) {
                                Ok(()) => actions.push(Action::Toast("Starting Suwayomi; it takes a few seconds".into(), material_3_ui::toasts::ToastKind::Info)),
                                Err(e) => actions.push(Action::Toast(e, material_3_ui::toasts::ToastKind::Error)),
                            }
                        }
                    });
                } else {
                    row(ui, "suwayomi-server-bin", "From the AUR. Needs Java, which it pulls in.", |ui| {
                        if widgets::button(ui, ButtonKind::Filled, Some(ICON_DOWNLOAD), "Install").clicked() {
                            install = true;
                        }
                    });
                }
                common::note(ui, "Docker works too: ghcr.io/suwayomi/tachidesk, port 4567.");
            });
            card(ui, "Connect", |ui| {
                row(ui, "Server address", "", |ui| {
                    widgets::field(ui, &mut ext.url, crate::sources::suwayomi::DEFAULT_URL, 260.0, egui::Id::unique("suwa-url"));
                });
                row(ui, "User name", "Only when the server has basic authentication turned on", |ui| {
                    widgets::field(ui, &mut ext.user, "", 260.0, egui::Id::unique("suwa-user"));
                });
                row(ui, "Password", "", |ui| {
                    egui::Frame::new().fill(p.surface_container_highest).corner_radius(egui::CornerRadius::same(12)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut ext.password).password(true).frame(egui::Frame::NONE).desired_width(240.0));
                    });
                });
                common::dialog_buttons(ui, |ui| {
                    if widgets::button(ui, ButtonKind::Filled, Some(ICON_LINK), "Connect").clicked() {
                        save = true;
                    }
                });
                ui.add_space(8.0);
            });
        });
    });
    if install {
        app.install(&["suwayomi-server-bin"]);
        app.pages.browse.ext.installed = None;
    }
    if save {
        let ext = &mut app.pages.browse.ext;
        app.cfg.suwayomi_url = ext.url.trim().to_string();
        app.cfg.suwayomi_user = ext.user.trim().to_string();
        app.cfg.suwayomi_password = ext.password.clone();
        ext.list = Task::default();
        ext.about = Task::default();
        app.config_changed();
        app.refresh_sources();
    }
}
