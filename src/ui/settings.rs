//! Preferences: appearance, the reader, the library (folders, updates,
//! categories), downloads, sources, tracking, and what Comics needs.

use crate::app::{App, BrowseTab, Page};
use crate::config::{Background, Fit, ReadingMode};
use crate::tracking;
use egui::{RichText, Ui};
use material_3_ui::common::{self, card, row, Requirement};
use material_3_ui::icons::icons::*;
use material_3_ui::prefs;
use material_3_ui::sys::paths;
use material_3_ui::sys::task::Task;
use material_3_ui::toasts::ToastKind;
use material_3_ui::widgets::{self, segmented, switch, ButtonKind};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Default)]
pub struct State {
    new_folder: String,
    new_category: String,
    renaming: Option<(i64, String)>,
    token: String,
    viewer: Task<Result<String, String>>,
    pick: Task<Option<(bool, PathBuf)>>,
    installed: Option<HashSet<String>>,
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut changed = false;
    let mut theme_changed = false;
    let ctx = app.ctx.clone();
    // A folder chosen in the file dialog: (is the downloads folder, path).
    if let Some(Some((downloads, dir))) = app.pages.settings.pick.take() {
        let text = paths::tilde(&dir);
        if downloads {
            app.cfg.downloads_folder = text;
        } else if !app.cfg.library_folders.contains(&text) {
            app.cfg.library_folders.push(text);
        }
        changed = true;
    }
    if app.anilist.signed_in() && app.pages.settings.viewer.value.is_none() && !app.pages.settings.viewer.running() {
        let al = app.anilist.clone();
        app.pages.settings.viewer.spawn(&ctx, move || al.viewer().map_err(|e| e.to_string()));
    }
    app.pages.settings.viewer.poll();

    common::page(ui, ICON_SETTINGS, "Preferences", "", |ui| {
        theme_changed |= prefs::appearance_card(ui, &mut app.cfg.appearance, &app.theme);

        let r = &mut app.cfg.reader;
        card(ui, "Reader", |ui| {
            row(ui, "Reading mode", "Each series can have its own, from the menu on its page.", |ui| {
                changed |= segmented(ui, &mut r.mode, &[
                    (ReadingMode::RightToLeft, ICON_SWIPE_LEFT, ""),
                    (ReadingMode::LeftToRight, ICON_SWIPE_RIGHT, ""),
                    (ReadingMode::Vertical, ICON_SWIPE_DOWN, ""),
                    (ReadingMode::Webtoon, ICON_VIEW_DAY, ""),
                ]);
                ui.label(RichText::new(r.mode.label()).color(p.on_surface_variant));
            });
            ui.separator();
            row(ui, "Page size", "In the paged modes.", |ui| {
                egui::ComboBox::from_id_salt("fit").selected_text(r.fit.label()).show_ui(ui, |ui| {
                    for f in Fit::ALL {
                        changed |= ui.selectable_value(&mut r.fit, f, f.label()).changed();
                    }
                });
            });
            ui.separator();
            row(ui, "Strip width", "How wide pages are in the vertical and webtoon modes.", |ui| {
                let mut pct = r.strip_width * 100.0;
                if ui.add(egui::Slider::new(&mut pct, 30.0..=100.0).suffix("%").integer()).changed() {
                    r.strip_width = pct / 100.0;
                    changed = true;
                }
            });
            ui.separator();
            row(ui, "Background", "", |ui| {
                changed |= segmented(ui, &mut r.background, &[
                    (Background::Black, ICON_DARK_MODE, "Black"),
                    (Background::Gray, ICON_CONTRAST, "Gray"),
                    (Background::White, ICON_LIGHT_MODE, "White"),
                    (Background::Theme, ICON_PALETTE, "Theme"),
                ]);
            });
            ui.separator();
            row(ui, "Pages to load ahead", "", |ui| {
                changed |= ui.add(egui::Slider::new(&mut r.preload, 1..=12)).changed();
            });
            ui.separator();
            row(ui, "Click to turn pages", "Clicking the left or right third of the page turns it; the middle shows the controls.", |ui| {
                changed |= switch(ui, &mut r.tap_zones).changed();
            });
            ui.separator();
            row(ui, "Page number", "", |ui| {
                changed |= switch(ui, &mut r.show_page_number).changed();
            });
            ui.separator();
            row(ui, "Go on to the next chapter", "At the end of a chapter, keep going.", |ui| {
                changed |= switch(ui, &mut r.continue_to_next).changed();
            });
        });

        library_card(ui, app, &mut changed);
        categories_card(ui, app);

        card(ui, "Downloads", |ui| {
            let dir = app.cfg.downloads_dir();
            row(ui, "Folder", &paths::tilde(&dir), |ui| {
                if widgets::button(ui, ButtonKind::Tonal, Some(ICON_FOLDER_OPEN), "Change…").clicked() {
                    pick_folder(app, true, dir.clone());
                }
                if !app.cfg.downloads_folder.is_empty() && widgets::button(ui, ButtonKind::Text, None, "Default").clicked() {
                    app.cfg.downloads_folder.clear();
                    changed = true;
                }
            });
            ui.separator();
            row(ui, "Delete after reading", "Remove a chapter's download when you finish it.", |ui| {
                changed |= switch(ui, &mut app.cfg.delete_after_reading).changed();
            });
        });

        card(ui, "Sources", |ui| {
            row(ui, "MangaDex languages", "Comma-separated codes: en, es-la, pt-br, fr, ja…", |ui| {
                changed |= widgets::field(ui, &mut app.cfg.mangadex_languages, "en", 200.0, egui::Id::unique("md-langs")).lost_focus();
            });
            ui.separator();
            row(ui, "MangaDex data saver", "Smaller, compressed pages.", |ui| {
                changed |= switch(ui, &mut app.cfg.mangadex_data_saver).changed();
            });
            ui.separator();
            row(ui, "Extension languages", "Only show Suwayomi sources and extensions in these languages (empty: all).", |ui| {
                let mut text = app.cfg.source_languages.join(", ");
                if widgets::field(ui, &mut text, "en, ja", 200.0, egui::Id::unique("src-langs")).changed() {
                    app.cfg.source_languages = text.split([',', ' ']).map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
                    changed = true;
                }
            });
            ui.separator();
            row(ui, "Show 18+ sources and series", "", |ui| {
                changed |= switch(ui, &mut app.cfg.show_nsfw).changed();
            });
            ui.separator();
            let server = if app.cfg.suwayomi_url.is_empty() { "Not set up".to_string() } else { app.cfg.suwayomi_url.clone() };
            row(ui, "Suwayomi server", &server, |ui| {
                if widgets::button(ui, ButtonKind::Tonal, Some(ICON_EXTENSION), "Extensions").clicked() {
                    app.actions.push(crate::app::Action::Goto(Page::Browse(BrowseTab::Extensions)));
                }
                if !app.cfg.suwayomi_url.is_empty() && widgets::button(ui, ButtonKind::Text, None, "Disconnect").clicked() {
                    app.cfg.suwayomi_url.clear();
                    app.pages.browse.ext = Default::default();
                    changed = true;
                }
            });
        });

        tracking_card(ui, app, &mut changed);
        needs_card(ui, app);

        card(ui, "About", |ui| {
            common::facts(ui, "about", &[
                ("Version", env!("CARGO_PKG_VERSION").to_string()),
                ("Library", paths::tilde(&crate::config::data_dir().join("library.db"))),
                ("Preferences", paths::tilde(&crate::config::Config::path())),
                ("Cache", paths::tilde(&crate::config::cache_dir())),
            ]);
        });
    });

    if theme_changed {
        app.apply_theme();
        changed = true;
    }
    if changed {
        app.config_changed();
    }
}

fn pick_folder(app: &mut App, downloads: bool, start: PathBuf) {
    app.pages.settings.pick.spawn(&app.ctx, move || {
        let _ = std::fs::create_dir_all(&start);
        rfd::FileDialog::new().set_directory(&start).pick_folder().map(|d| (downloads, d))
    });
}

fn library_card(ui: &mut Ui, app: &mut App, changed: &mut bool) {
    let p = app.palette.clone();
    card(ui, "Library", |ui| {
        ui.add_space(8.0);
        ui.label(RichText::new("Local files folders").size(14.5).color(p.on_surface));
        ui.label(RichText::new("Each folder in these is a series, and each comic file or image folder in it a chapter.").size(12.5).color(p.on_surface_variant));
        let mut remove = None;
        for (i, f) in app.cfg.library_folders.iter().enumerate() {
            ui.horizontal(|ui| {
                widgets::icon(ui, ICON_FOLDER, 18.0, p.on_surface_variant);
                let exists = paths::expand(f).is_dir();
                ui.label(RichText::new(f).color(if exists { p.on_surface } else { p.on_surface_variant }));
                if !exists {
                    ui.label(RichText::new("(doesn't exist yet)").size(12.0).color(p.on_surface_variant));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, ICON_CLOSE, "Remove", true).clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            app.cfg.library_folders.remove(i);
            *changed = true;
        }
        ui.horizontal(|ui| {
            let r = widgets::field(ui, &mut app.pages.settings.new_folder, "~/Manga", 260.0, egui::Id::unique("new-folder"));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let text = app.pages.settings.new_folder.trim().to_string();
            if (widgets::icon_button(ui, ICON_ADD, "Add", !text.is_empty()).clicked() || enter) && !text.is_empty() {
                app.cfg.library_folders.push(text);
                app.pages.settings.new_folder.clear();
                *changed = true;
            }
            if widgets::button(ui, ButtonKind::Text, None, "Choose…").clicked() {
                pick_folder(app, false, paths::home());
            }
        });
        ui.add_space(8.0);
        ui.separator();
        row(ui, "Check for new chapters", "When Comics starts, if the last check is older than this.", |ui| {
            let options = [(0, "Only by hand"), (6, "Every 6 hours"), (12, "Every 12 hours"), (24, "Daily"), (48, "Every 2 days"), (168, "Weekly")];
            let current = options.iter().find(|(h, _)| *h == app.cfg.update_interval_hours).map(|(_, l)| *l).unwrap_or("Custom");
            egui::ComboBox::from_id_salt("interval").selected_text(current).show_ui(ui, |ui| {
                for (h, label) in options {
                    *changed |= ui.selectable_value(&mut app.cfg.update_interval_hours, h, label).changed();
                }
            });
        });
        ui.separator();
        row(ui, "Notify about new chapters", "A desktop notification when a check finds some and Comics isn't focused.", |ui| {
            *changed |= switch(ui, &mut app.cfg.notify_new_chapters).changed();
        });
        ui.separator();
        row(ui, "Download new chapters", "Queue new chapters of series in the library for download.", |ui| {
            *changed |= switch(ui, &mut app.cfg.download_new_chapters).changed();
        });
        ui.separator();
        row(ui, "From a timer", "`m3-comics update` checks without opening a window; the package ships a systemd user timer for it.", |_| {});
    });
}

fn categories_card(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let cats = app.db.categories().unwrap_or_default();
    let mut dirty = false;
    card(ui, "Categories", |ui| {
        if cats.is_empty() {
            common::note(ui, "Categories group your library into tabs (Reading, Plan to read, …).");
        }
        let st = &mut app.pages.settings;
        for (i, c) in cats.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.set_height(36.0);
                widgets::icon(ui, ICON_LABEL, 18.0, p.on_surface_variant);
                match &mut st.renaming {
                    Some((id, text)) if *id == c.id => {
                        let r = widgets::field(ui, text, "", 220.0, egui::Id::unique(("rename", c.id)));
                        if r.lost_focus() {
                            if !text.trim().is_empty() {
                                let _ = app.db.rename_category(c.id, text);
                                dirty = true;
                            }
                            st.renaming = None;
                        }
                    }
                    _ => {
                        ui.label(RichText::new(&c.name).size(14.5).color(p.on_surface));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, ICON_DELETE, "Delete", true).clicked() {
                        let _ = app.db.delete_category(c.id);
                        dirty = true;
                    }
                    if widgets::icon_button(ui, ICON_EDIT, "Rename", true).clicked() {
                        st.renaming = Some((c.id, c.name.clone()));
                    }
                    if widgets::icon_button(ui, ICON_ARROW_DOWNWARD, "Move down", i + 1 < cats.len()).clicked() {
                        let _ = app.db.move_category(c.id, 1);
                        dirty = true;
                    }
                    if widgets::icon_button(ui, ICON_ARROW_UPWARD, "Move up", i > 0).clicked() {
                        let _ = app.db.move_category(c.id, -1);
                        dirty = true;
                    }
                });
            });
        }
        ui.horizontal(|ui| {
            let r = widgets::field(ui, &mut st.new_category, "New category", 260.0, egui::Id::unique("settings-new-category"));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let name = st.new_category.trim().to_string();
            if (widgets::icon_button(ui, ICON_ADD, "Add", !name.is_empty()).clicked() || enter) && !name.is_empty() && app.db.add_category(&name).is_ok() {
                st.new_category.clear();
                dirty = true;
            }
        });
        ui.add_space(8.0);
    });
    if dirty {
        app.dirty();
    }
}

fn tracking_card(ui: &mut Ui, app: &mut App, changed: &mut bool) {
    let p = app.palette.clone();
    let signed_in = app.anilist.signed_in();
    let mut sign_out = false;
    let mut save_token = None;
    card(ui, "Tracking", |ui| {
        if signed_in {
            let who = match &app.pages.settings.viewer.value {
                Some(Ok(name)) => format!("Signed in as {name}"),
                Some(Err(e)) => e.clone(),
                None => "Checking…".to_string(),
            };
            row(ui, "AniList", &who, |ui| {
                if widgets::button(ui, ButtonKind::Text, None, "Sign out").clicked() {
                    sign_out = true;
                }
            });
            ui.separator();
            row(ui, "Update progress automatically", "After you finish a chapter of a tracked series.", |ui| {
                *changed |= switch(ui, &mut app.cfg.auto_track).changed();
            });
            return;
        }
        common::note(
            ui,
            &format!(
                "Track what you read on AniList. Create an API client once at anilist.co → Settings → Developer, with the redirect URL {}, and put its ID here. Then sign in: AniList shows a token to paste below.",
                tracking::PIN_REDIRECT
            ),
        );
        row(ui, "Client ID", "", |ui| {
            *changed |= widgets::field(ui, &mut app.cfg.anilist_client_id, "12345", 160.0, egui::Id::unique("anilist-id")).lost_focus();
            if widgets::button(ui, ButtonKind::Text, None, "Developer settings").clicked() {
                app.actions.push(crate::app::Action::OpenUrl("https://anilist.co/settings/developer".into()));
            }
        });
        ui.separator();
        row(ui, "Sign in", "Opens AniList in your browser.", |ui| {
            let ok = !app.cfg.anilist_client_id.trim().is_empty();
            if widgets::button_enabled(ui, ButtonKind::Tonal, Some(ICON_OPEN_IN_BROWSER), "Sign in", ok).clicked() {
                app.actions.push(crate::app::Action::OpenUrl(tracking::auth_url(&app.cfg.anilist_client_id)));
            }
        });
        ui.separator();
        row(ui, "Token", "Paste the token (or the whole address) AniList shows.", |ui| {
            let st = &mut app.pages.settings;
            if widgets::button_enabled(ui, ButtonKind::Filled, None, "Save", !st.token.trim().is_empty()).clicked() {
                save_token = Some(st.token.clone());
            }
            widgets::field(ui, &mut st.token, "", 220.0, egui::Id::unique("anilist-token"));
        });
        if !material_3_ui::secrets::Keyring::available() {
            ui.label(RichText::new("The token is kept in your keyring, which needs secret-tool (libsecret).").size(12.5).color(p.error));
        }
    });
    if sign_out {
        let _ = app.anilist.sign_out();
        app.pages.settings.viewer = Task::default();
    }
    if let Some(pasted) = save_token {
        match tracking::parse_token(&pasted) {
            Some(t) => match app.anilist.save_token(&t) {
                Ok(()) => {
                    app.pages.settings.token.clear();
                    app.pages.settings.viewer = Task::default();
                    app.toast("Signed in to AniList", ToastKind::Success);
                }
                Err(e) => app.toast(e.to_string(), ToastKind::Error),
            },
            None => app.toast("That doesn't look like an AniList token", ToastKind::Error),
        }
    }
}

fn needs_card(ui: &mut Ui, app: &mut App) {
    let installed = app.pages.settings.installed.get_or_insert_with(material_3_ui::sys::arch::installed_packages).clone();
    let mut install = Vec::new();
    card(ui, "Optional tools", |ui| {
        let present: Vec<bool> = [("bsdtar", "libarchive", "Opens CBR, CB7 and CBT files."), ("secret-tool", "libsecret", "Keeps the AniList sign-in in your keyring.")]
            .into_iter()
            .map(|(prog, pkg, why)| {
                let r = common::require(ui, &installed, prog, pkg, why);
                if r == Requirement::InstallRequested {
                    install.push(pkg);
                }
                r.present()
            })
            .collect();
        if present.iter().all(|p| *p) {
            common::note(ui, "Everything Comics can use is installed.");
        }
    });
    if !install.is_empty() {
        app.install(&install);
        app.pages.settings.installed = None;
    }
}
