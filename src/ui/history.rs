//! History: the last chapter read of each series, most recent first, to
//! pick up where you left off.

use super::updates::{cover_row, paint_cover};
use super::{empty_state, header, headers_for};
use crate::app::{Action, App, BrowseTab, Page};
use crate::model::{now, relative_date};
use egui::Ui;
use material_3_ui::common;
use material_3_ui::icons::icons::*;
use material_3_ui::widgets;

pub fn show(ui: &mut Ui, app: &mut App) {
    let mut actions = Vec::new();
    let list = app.history_list();
    let mut clear_all = false;
    header(ui, ICON_HISTORY, "History", "Pick up where you left off", |ui| {
        if !list.is_empty() && widgets::icon_button(ui, ICON_CLEAR_ALL, "Clear history", true).clicked() {
            clear_all = true;
        }
        let r = common::search_field(ui, &mut app.pages.history_query, "Search history", 240.0);
        if app.pages.focus_search {
            app.pages.focus_search = false;
            r.request_focus();
        }
    });
    if list.is_empty() {
        empty_state(ui, ICON_HISTORY, "Nothing read yet", "Chapters you read show up here.", Some(("Browse", Action::Goto(Page::Browse(BrowseTab::Sources)))), &mut actions);
        app.actions.extend(actions);
        return;
    }
    let q = app.pages.history_query.to_lowercase();
    let server = app.image_headers(crate::sources::suwayomi::PREFIX);
    let today = now();
    let mut forget = None;
    egui::ScrollArea::vertical().id_salt("history").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 20, top: 0, bottom: 16 }).show(ui, |ui| {
            let mut last_day = String::new();
            for item in list.iter().filter(|h| q.is_empty() || h.manga.title().to_lowercase().contains(&q)) {
                let day = relative_date(item.chapter.read_at, today);
                if day != last_day {
                    widgets::section_header(ui, &day);
                    last_day = day;
                }
                let c = &item.chapter;
                let time = chrono::DateTime::from_timestamp(c.read_at, 0).map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default();
                let progress = if c.in_progress() && c.pages > 0 { format!(" · Page {} of {}", c.last_page + 1, c.pages) } else { String::new() };
                let sub = format!("{}{progress} · {time}", c.name);
                let headers = headers_for(&item.manga.source, &server);
                let (r, cover) = cover_row(ui, item.manga.title(), &sub, false, |ui| {
                    if widgets::icon_button(ui, ICON_DELETE, "Remove from history", true).clicked() {
                        forget = Some(item.manga.id);
                    }
                    if widgets::icon_button(ui, ICON_PLAY_ARROW, "Resume", true).clicked() {
                        actions.push(Action::Continue(item.manga.id));
                    }
                });
                paint_cover(ui, &mut app.covers, item.manga.info.cover.as_deref(), &headers, cover);
                if r.clicked() {
                    actions.push(Action::Goto(Page::Series(item.manga.id)));
                }
            }
        });
    });
    if let Some(id) = forget {
        let _ = app.db.clear_history(Some(id));
        app.dirty();
    }
    if clear_all {
        let _ = app.db.clear_history(None);
        app.dirty();
        app.toast("History cleared", material_3_ui::toasts::ToastKind::Info);
    }
    app.actions.extend(actions);
}
