//! Drawing the window: the navigation drawer, the pages, dialogs, and the
//! pieces pages share (the cover grid).

mod browse;
mod downloads;
mod history;
mod library;
mod series;
mod settings;
mod sidebar;
mod updates;

use crate::app::{Action, App, Page};
use crate::covers::{self, Covers};
use egui::{pos2, vec2, Color32, CornerRadius, Frame, Margin, Rect, Response, Sense, Ui};
use material_3_ui::common;
use material_3_ui::icons::icons::*;
use material_3_ui::widgets::{self, ButtonKind};

#[derive(Default)]
pub struct States {
    /// Ctrl+F was pressed: the page's search field takes focus.
    pub focus_search: bool,
    pub library: library::State,
    pub browse: browse::State,
    pub series: series::State,
    pub settings: settings::State,
    pub history_query: String,
}

pub fn layout(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let surface = Frame::new().fill(p.surface);
    egui::Panel::left("sidebar")
        .resizable(false)
        .exact_size(232.0)
        .show_separator_line(false)
        .frame(surface.inner_margin(Margin { left: 12, right: 4, top: 12, bottom: 8 }))
        .show(ui, |ui| sidebar::show(ui, app));
    egui::CentralPanel::default()
        .frame(surface.inner_margin(Margin { left: 8, right: 12, top: 12, bottom: 12 }))
        .show(ui, |ui| {
            Frame::new().fill(p.surface_container_low).corner_radius(CornerRadius::same(24)).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                match app.page.clone() {
                    Page::Library => library::show(ui, app),
                    Page::Updates => updates::show(ui, app),
                    Page::History => history::show(ui, app),
                    Page::Browse(tab) => browse::show(ui, app, tab),
                    Page::Catalogue(id) => browse::catalogue(ui, app, &id),
                    Page::GlobalSearch => browse::global_search(ui, app),
                    Page::Series(id) => series::show(ui, app, id),
                    Page::Downloads => downloads::show(ui, app),
                    Page::Settings => settings::show(ui, app),
                }
            });
        });
}

pub fn dialogs(ctx: &egui::Context, app: &mut App) {
    category_dialog(ctx, app);
}

fn category_dialog(ctx: &egui::Context, app: &mut App) {
    let Some(dialog) = &mut app.category_dialog else { return };
    let p = app.palette.clone();
    let cats = app.db.categories().unwrap_or_default();
    let mut close = false;
    let mut save = false;
    let mut add = None;
    common::modal(ctx, &p, "categories", ICON_LABEL, "Categories", |ui| {
        ui.set_width(340.0);
        if cats.is_empty() {
            common::note(ui, "Categories group your library into tabs. Add one:");
        }
        for c in &cats {
            let mut on = dialog.chosen.contains(&c.id);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut on, egui::RichText::new(&c.name).size(14.5)).changed() {
                    if on {
                        dialog.chosen.push(c.id);
                    } else {
                        dialog.chosen.retain(|x| *x != c.id);
                    }
                }
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let r = widgets::field(ui, &mut dialog.new_name, "New category", 240.0, egui::Id::unique("new-category"));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (widgets::icon_button(ui, ICON_ADD, "Add", !dialog.new_name.trim().is_empty()).clicked() || enter) && !dialog.new_name.trim().is_empty() {
                add = Some(dialog.new_name.trim().to_string());
            }
        });
        common::dialog_buttons(ui, |ui| {
            if widgets::button(ui, ButtonKind::Filled, None, "Save").clicked() {
                save = true;
            }
            if widgets::button(ui, ButtonKind::Text, None, "Cancel").clicked() {
                close = true;
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
    });
    if let Some(name) = add {
        match app.db.add_category(&name) {
            Ok(id) => {
                dialog.chosen.push(id);
                dialog.new_name.clear();
            }
            Err(_) => app.toasts.error(format!("There's already a category called {name}")),
        }
    }
    if save {
        let _ = app.db.set_manga_categories(dialog.manga, &dialog.chosen);
        app.category_dialog = None;
        app.dirty();
    } else if close {
        app.category_dialog = None;
    }
}

/// Badges on a cover.
#[derive(Default, Clone, Copy)]
pub struct Badges {
    pub unread: i64,
    pub downloaded: usize,
    pub in_library: bool,
}

/// Height of a cover card of `width`: a 2:3 cover and two lines of title.
pub fn card_height(width: f32) -> f32 {
    (width * 1.5).round() + 46.0
}

/// A series' cover with its title under it (Mihon's comfortable grid).
pub fn cover_card(ui: &mut Ui, covers: &mut Covers, cover: Option<&str>, headers: &[(String, String)], title: &str, width: f32, badges: Badges) -> Response {
    let p = widgets::palette(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(width, card_height(width)), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let cover_rect = Rect::from_min_size(rect.min, vec2(width, (width * 1.5).round()));
    covers::paint(ui, covers, cover, headers, cover_rect, 14, ICON_AUTO_STORIES);
    if badges.in_library {
        ui.painter().rect_filled(cover_rect, CornerRadius::same(14), Color32::from_black_alpha(110));
    }
    if response.hovered() {
        ui.painter().rect_filled(cover_rect, CornerRadius::same(14), p.on_surface.gamma_multiply(0.10));
    }
    // Badges, top left: unread (primary), downloaded (tertiary).
    let mut x = cover_rect.left() + 6.0;
    let y = cover_rect.top() + 6.0;
    let mut pill = |text: String, bg: Color32, fg: Color32| {
        let g = ui.painter().layout_no_wrap(text, widgets::title_font(12.0), fg);
        let r = Rect::from_min_size(pos2(x, y), vec2(g.size().x + 12.0, 20.0));
        ui.painter().rect_filled(r, CornerRadius::same(6), bg);
        ui.painter().galley(pos2(r.left() + 6.0, r.center().y - g.size().y / 2.0), g, fg);
        x = r.right() + 4.0;
    };
    if badges.unread > 0 {
        pill(badges.unread.to_string(), p.primary, p.on_primary);
    }
    if badges.downloaded > 0 {
        pill(badges.downloaded.to_string(), p.tertiary, p.on_tertiary);
    }
    if badges.in_library {
        let c = cover_rect.right_top() + vec2(-16.0, 16.0);
        ui.painter().circle_filled(c, 12.0, p.primary);
        widgets::paint_icon(ui, ICON_BOOKMARK_ADDED, c, 15.0, p.on_primary);
    }
    let mut job = egui::text::LayoutJob::single_section(title.to_string(), egui::TextFormat::simple(egui::FontId::proportional(13.0), p.on_surface));
    job.wrap = egui::text::TextWrapping { max_width: width - 4.0, max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(rect.left() + 2.0, cover_rect.bottom() + 6.0), g, p.on_surface);
    response.on_hover_text(title)
}

/// A virtualized grid of `count` cards. `card(ui, index, width)` draws one.
/// Returns whether the last row is on screen (to load more).
pub fn cover_grid(ui: &mut Ui, id: &str, count: usize, preferred: f32, mut card: impl FnMut(&mut Ui, usize, f32)) -> bool {
    let gap = 14.0;
    let avail = ui.available_width() - 40.0;
    let cols = (((avail + gap) / (preferred + gap)).floor() as usize).max(2);
    let width = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let row_h = card_height(width) + gap;
    let rows = count.div_ceil(cols);
    let mut at_end = rows == 0;
    egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show_rows(ui, row_h, rows, |ui, range| {
        if range.end >= rows {
            at_end = true;
        }
        for row in range {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.spacing_mut().item_spacing.x = gap;
                for col in 0..cols {
                    let i = row * cols + col;
                    if i >= count {
                        break;
                    }
                    card(ui, i, width);
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
    });
    at_end
}

/// A page title row with trailing controls.
pub fn header(ui: &mut Ui, icon: material_3_ui::icons::MaterialIcon, title: &str, subtitle: &str, trailing: impl FnOnce(&mut Ui)) {
    let p = widgets::palette(ui);
    ui.add_space(18.0);
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        widgets::icon(ui, icon, 28.0, p.primary);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).size(24.0).color(p.on_surface));
            if !subtitle.is_empty() {
                ui.label(egui::RichText::new(subtitle).size(12.5).color(p.on_surface_variant));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(20.0);
            trailing(ui);
        });
    });
    ui.add_space(10.0);
}

/// A centered message for empty pages, with an optional button.
pub fn empty_state(ui: &mut Ui, icon: material_3_ui::icons::MaterialIcon, title: &str, text: &str, button: Option<(&str, Action)>, actions: &mut Vec<Action>) {
    let p = widgets::palette(ui);
    ui.add_space((ui.available_height() * 0.25).max(24.0));
    ui.vertical_centered(|ui| {
        widgets::icon(ui, icon, 56.0, p.on_surface_variant.gamma_multiply(0.7));
        ui.add_space(10.0);
        ui.label(widgets::title_text(title, 18.0).color(p.on_surface));
        ui.add_space(4.0);
        ui.set_max_width(440.0);
        ui.add(egui::Label::new(egui::RichText::new(text).color(p.on_surface_variant)).wrap());
        if let Some((label, action)) = button {
            ui.add_space(14.0);
            if widgets::button(ui, ButtonKind::Tonal, None, label).clicked() {
                actions.push(action);
            }
        }
    });
}

/// Cover headers for a source, given the Suwayomi server's.
pub fn headers_for(source: &str, server: &[(String, String)]) -> Vec<(String, String)> {
    if source.starts_with(crate::sources::suwayomi::PREFIX) {
        server.to_vec()
    } else {
        Vec::new()
    }
}

/// "12 chapters", "1 chapter".
pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}
