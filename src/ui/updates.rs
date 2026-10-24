//! Updates: new chapters the library checks found, newest first, grouped
//! by day.

use super::{empty_state, header, headers_for};
use crate::app::{Action, App, Page};
use crate::covers::{self, Covers};
use crate::db::{Chapter, Manga};
use crate::downloads::JobState;
use crate::model::{now, relative_date};
use egui::{pos2, vec2, Rect, Response, Sense, Ui};
use material_3_ui::common;
use material_3_ui::icons::icons::*;
use material_3_ui::widgets;
use std::sync::atomic::Ordering;

/// A row with room for a small cover, two lines of text, and trailing
/// controls. Returns the row and where the cover goes (see [`paint_cover`]).
pub fn cover_row(ui: &mut Ui, title: &str, subtitle: &str, dim: bool, trailing: impl FnOnce(&mut Ui)) -> (Response, Rect) {
    let p = widgets::palette(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 68.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(14), p.on_surface.gamma_multiply(0.06));
    }
    let cover_rect = Rect::from_min_size(rect.min + vec2(8.0, 6.0), vec2(38.0, 56.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 4.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    trailing(&mut child);
    let trailing_w = child.min_rect().width();
    let left = rect.left() + 60.0;
    let max_w = (rect.right() - trailing_w - 20.0 - left).max(40.0);
    let fg = if dim { p.on_surface.gamma_multiply(0.5) } else { p.on_surface };
    let tg = widgets::truncated_galley(ui, title, widgets::title_font(14.5), fg, max_w);
    ui.painter().galley(pos2(left, rect.center().y - tg.size().y - 1.0), tg, fg);
    let sub_fg = if dim { p.on_surface_variant.gamma_multiply(0.6) } else { p.on_surface_variant };
    let sg = widgets::truncated_galley(ui, subtitle, egui::FontId::proportional(12.5), sub_fg, max_w);
    ui.painter().galley(pos2(left, rect.center().y + 2.0), sg, sub_fg);
    (response, cover_rect)
}

pub fn paint_cover(ui: &Ui, covers: &mut Covers, cover: Option<&str>, headers: &[(String, String)], rect: Rect) {
    if ui.is_rect_visible(rect) {
        covers::paint(ui, covers, cover, headers, rect, 8, ICON_AUTO_STORIES);
    }
}

/// The download button of a chapter: download, progress, or done.
pub fn download_button(ui: &mut Ui, app: &mut App, m: &Manga, c: &Chapter, actions: &mut Vec<Action>) {
    // Local files are already on this computer.
    if m.source == crate::sources::local::ID {
        return;
    }
    let p = widgets::palette(ui);
    match app.downloads.state_of(c.id) {
        Some(JobState::Running { done, total }) => {
            let r = ui.add(egui::Spinner::new().color(p.primary).size(20.0));
            if r.on_hover_text(format!("Downloading: {done} of {total} pages")).clicked() {
                actions.push(Action::CancelDownload(c.id));
            }
        }
        Some(JobState::Queued) => {
            if widgets::icon_button(ui, ICON_SCHEDULE, "Queued (click to cancel)", true).clicked() {
                actions.push(Action::CancelDownload(c.id));
            }
        }
        Some(JobState::Failed(e)) => {
            if widgets::icon_button(ui, ICON_ERROR, &format!("Failed: {e}. Click to try again."), true).clicked() {
                actions.push(Action::Download(m.id, vec![c.id]));
            }
        }
        None if app.downloads.is_downloaded(m, c) => {
            let r = widgets::icon_button_sized(ui, ICON_DOWNLOAD_DONE, "Downloaded", true, 36.0, false);
            r.context_menu(|ui| {
                if widgets::menu_item(ui, Some(ICON_DELETE), "Delete download", "", false) {
                    actions.push(Action::DeleteDownloads(m.id, vec![c.id]));
                }
            });
        }
        None => {
            if widgets::icon_button(ui, ICON_DOWNLOAD, "Download", true).clicked() {
                actions.push(Action::Download(m.id, vec![c.id]));
            }
        }
    }
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let list = app.updates_list();
    let busy = app.update_job.is_some();
    let progress = app.update_job.as_ref().map(|(_, p)| (p.done.load(Ordering::Relaxed), p.total.load(Ordering::Relaxed)));
    header(ui, ICON_NEW_RELEASES, "Updates", "New chapters in your library", |ui| {
        if common::refresh_button(ui, busy) {
            actions.push(Action::UpdateLibrary);
        }
        if let Some((done, total)) = progress {
            ui.label(egui::RichText::new(format!("{done} of {total}")).color(p.on_surface_variant));
        }
    });
    if let Some((done, total)) = progress {
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            widgets::linear_progress(ui, (total > 0).then(|| done as f32 / total as f32), ui.available_width() - 24.0);
        });
        ui.add_space(8.0);
    }
    if list.is_empty() {
        empty_state(
            ui,
            ICON_NEW_RELEASES,
            "No updates yet",
            "When a library check finds new chapters, they show up here. The library is checked when Comics starts (see Preferences), or now:",
            (!busy).then_some(("Check now", Action::UpdateLibrary)),
            &mut actions,
        );
        app.actions.extend(actions);
        return;
    }
    let server = app.image_headers(crate::sources::suwayomi::PREFIX);
    let today = now();
    egui::ScrollArea::vertical().id_salt("updates").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 20, top: 0, bottom: 16 }).show(ui, |ui| {
            let mut last_day = String::new();
            for item in list.iter() {
                let day = relative_date(item.chapter.fetched_at, today);
                if day != last_day {
                    widgets::section_header(ui, &day);
                    last_day = day;
                }
                let headers = headers_for(&item.manga.source, &server);
                let (r, cover) = cover_row(ui, item.manga.title(), &item.chapter.name, item.chapter.read, |ui| {
                    download_button(ui, app, &item.manga, &item.chapter, &mut actions);
                });
                paint_cover(ui, &mut app.covers, item.manga.info.cover.as_deref(), &headers, cover);
                if r.clicked() {
                    actions.push(Action::Read { manga: item.manga.id, chapter: item.chapter.id });
                }
                r.context_menu(|ui| {
                    let read = item.chapter.read;
                    if widgets::menu_item(ui, Some(if read { ICON_REMOVE_DONE } else { ICON_DONE_ALL }), if read { "Mark as unread" } else { "Mark as read" }, "", false) {
                        actions.push(Action::SetRead(vec![item.chapter.id], !read));
                    }
                    if widgets::menu_item(ui, Some(ICON_AUTO_STORIES), "Open series", "", false) {
                        actions.push(Action::Goto(Page::Series(item.manga.id)));
                    }
                });
            }
        });
    });
    app.actions.extend(actions);
}
