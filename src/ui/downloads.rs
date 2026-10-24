//! Downloads: the queue, with progress, pause, retry and cancel.

use super::{empty_state, header, headers_for, plural};
use super::updates::{cover_row, paint_cover};
use crate::app::{Action, App, BrowseTab, Page};
use crate::downloads::JobState;
use egui::Ui;
use material_3_ui::icons::icons::*;
use material_3_ui::sys::paths;
use material_3_ui::widgets::{self, ButtonKind};

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let jobs = app.downloads.jobs();
    let paused = app.downloads.paused();
    let root = app.downloads.root().to_path_buf();
    let failed = jobs.iter().filter(|j| matches!(j.state, JobState::Failed(_))).count();
    let mut clear = false;
    let mut retry = false;
    let mut toggle_pause = false;
    header(ui, ICON_DOWNLOAD, "Downloads", &format!("Saved as CBZ files in {}", paths::tilde(&root)), |ui| {
        if widgets::icon_button(ui, ICON_FOLDER_OPEN, "Show in Files", true).clicked() {
            let _ = std::fs::create_dir_all(&root);
            actions.push(Action::ShowInFiles(root.clone()));
        }
        if !jobs.is_empty() {
            if widgets::button(ui, ButtonKind::Text, None, "Clear").clicked() {
                clear = true;
            }
            if failed > 0 && widgets::button(ui, ButtonKind::Tonal, Some(ICON_REFRESH), "Retry failed").clicked() {
                retry = true;
            }
            let (icon, label) = if paused { (ICON_PLAY_ARROW, "Resume") } else { (ICON_PAUSE, "Pause") };
            if widgets::button(ui, ButtonKind::Tonal, Some(icon), label).clicked() {
                toggle_pause = true;
            }
        }
    });
    if clear {
        app.downloads.clear();
    }
    if retry {
        app.downloads.retry_failed();
    }
    if toggle_pause {
        app.downloads.set_paused(!paused);
    }
    if jobs.is_empty() {
        empty_state(
            ui,
            ICON_DOWNLOAD_DONE,
            "No downloads",
            "Download chapters from a series' page to read them offline. Downloaded chapters also show a badge in the library.",
            Some(("Browse", Action::Goto(Page::Browse(BrowseTab::Sources)))),
            &mut actions,
        );
        app.actions.extend(actions);
        return;
    }
    let server = app.image_headers(crate::sources::suwayomi::PREFIX);
    egui::ScrollArea::vertical().id_salt("downloads").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 20, top: 0, bottom: 16 }).show(ui, |ui| {
            let queued = jobs.iter().filter(|j| !matches!(j.state, JobState::Failed(_))).count();
            widgets::section_header(ui, &format!("{}{}", plural(queued, "chapter"), if paused { " · paused" } else { "" }));
            for job in &jobs {
                let (sub, fraction) = match &job.state {
                    JobState::Queued => (format!("{} · waiting", job.chapter.name), None),
                    JobState::Running { done, total } => (format!("{} · {done} of {total} pages", job.chapter.name), Some(if *total > 0 { *done as f32 / *total as f32 } else { 0.0 })),
                    JobState::Failed(e) => (format!("{} · {e}", job.chapter.name), None),
                };
                let headers = headers_for(&job.manga.source, &server);
                let failed = matches!(job.state, JobState::Failed(_));
                let (r, cover) = cover_row(ui, job.manga.title(), &sub, failed, |ui| {
                    if widgets::icon_button(ui, ICON_CLOSE, "Cancel", true).clicked() {
                        actions.push(Action::CancelDownload(job.chapter.id));
                    }
                    if failed && widgets::icon_button(ui, ICON_REFRESH, "Try again", true).clicked() {
                        actions.push(Action::Download(job.manga.id, vec![job.chapter.id]));
                    }
                });
                paint_cover(ui, &mut app.covers, job.manga.info.cover.as_deref(), &headers, cover);
                if let Some(f) = fraction {
                    let bar = egui::Rect::from_min_size(r.rect.left_bottom() + egui::vec2(60.0, -8.0), egui::vec2(r.rect.width() - 120.0, 4.0));
                    ui.painter().rect_filled(bar, egui::CornerRadius::same(2), p.secondary_container);
                    let mut fill = bar;
                    fill.set_width(bar.width() * f);
                    ui.painter().rect_filled(fill, egui::CornerRadius::same(2), p.primary);
                }
                if r.clicked() {
                    actions.push(Action::Goto(Page::Series(job.manga.id)));
                }
            }
        });
    });
    app.actions.extend(actions);
}
