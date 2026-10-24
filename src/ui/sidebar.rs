//! The navigation drawer: the app's destinations, and the library update
//! status at the bottom.

use crate::app::{Action, App, BrowseTab, Page};
use egui::{vec2, Sense, Ui};
use material_3_ui::icons::icons::*;
use material_3_ui::widgets::{self, nav_item, section_header};
use std::sync::atomic::Ordering;

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let current = app.page.clone();

    ui.horizontal(|ui| {
        ui.add_space(12.0);
        widgets::icon(ui, ICON_AUTO_STORIES, 26.0, p.primary);
        ui.label(widgets::title_text("Comics", 20.0).color(p.on_surface));
    });
    ui.add_space(14.0);

    let library_n = app.library().len();
    let new_n = app.updates_list().iter().filter(|u| !u.chapter.read).count();
    let active = app.downloads.active();
    let in_browse = matches!(current, Page::Browse(_) | Page::Catalogue(_) | Page::GlobalSearch);
    let in_library = matches!(current, Page::Library) || matches!(current, Page::Series(_)) && app.back.is_empty();
    let count = |n: usize| (n > 0).then(|| n.to_string());

    let bottom_h = 64.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - bottom_h).show(ui, |ui| {
        let mut item = |ui: &mut Ui, icon, label: &str, page: Page, selected: bool, trailing: Option<String>| {
            if nav_item(ui, icon, label, selected, trailing.as_deref()).clicked() {
                actions.push(Action::Goto(page));
            }
        };
        item(ui, ICON_COLLECTIONS_BOOKMARK, "Library", Page::Library, in_library, count(library_n));
        item(ui, ICON_NEW_RELEASES, "Updates", Page::Updates, current == Page::Updates, count(new_n));
        item(ui, ICON_HISTORY, "History", Page::History, current == Page::History, None);
        section_header(ui, "Find");
        item(ui, ICON_EXPLORE, "Browse", Page::Browse(BrowseTab::Sources), in_browse && current != Page::Browse(BrowseTab::Extensions), None);
        item(ui, ICON_EXTENSION, "Extensions", Page::Browse(BrowseTab::Extensions), current == Page::Browse(BrowseTab::Extensions), None);
        section_header(ui, "On this computer");
        item(ui, ICON_DOWNLOAD, "Downloads", Page::Downloads, current == Page::Downloads, count(active));
        item(ui, ICON_SETTINGS, "Preferences", Page::Settings, current == Page::Settings, None);
    });

    // Library update status / button.
    ui.add_space(4.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = widgets::state_layer(p.surface_container, p.on_surface, &resp);
    ui.painter().rect_filled(rect, egui::CornerRadius::same(26), fill);
    let icon_c = rect.left_center() + vec2(26.0, 0.0);
    let (title, sub) = match &app.update_job {
        Some((_, progress)) => {
            let done = progress.done.load(Ordering::Relaxed);
            let total = progress.total.load(Ordering::Relaxed);
            let current = progress.current.lock().map(|c| c.clone()).unwrap_or_default();
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            (format!("Updating {done} of {total}"), current)
        }
        None => {
            let last: i64 = app.db.meta(crate::updates::LAST_UPDATE_KEY).and_then(|s| s.parse().ok()).unwrap_or(0);
            let when = crate::model::relative_date(last, crate::model::now());
            ("Update library".to_string(), if when.is_empty() { "Never checked".into() } else { format!("Checked {}", when.to_lowercase()) })
        }
    };
    ui.painter().circle_filled(icon_c, 18.0, p.primary_container);
    widgets::paint_icon(ui, if app.update_job.is_some() { ICON_SYNC } else { ICON_REFRESH }, icon_c, 20.0, p.on_primary_container);
    let tx = icon_c.x + 28.0;
    let max = rect.right() - tx - 12.0;
    let tg = widgets::truncated_galley(ui, &title, widgets::title_font(13.5), p.on_surface, max);
    ui.painter().galley(egui::pos2(tx, rect.center().y - 16.0), tg, p.on_surface);
    let sg = widgets::truncated_galley(ui, &sub, egui::FontId::proportional(11.5), p.on_surface_variant, max);
    ui.painter().galley(egui::pos2(tx, rect.center().y + 1.0), sg, p.on_surface_variant);
    if resp.on_hover_text("Check the library for new chapters (Ctrl+R)").clicked() {
        actions.push(Action::UpdateLibrary);
    }
    app.actions.extend(actions);
}
