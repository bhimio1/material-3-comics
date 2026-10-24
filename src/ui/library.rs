//! The library: the series the user added, as a grid of covers with
//! unread and downloaded counts, in category tabs, searchable, sortable
//! and filterable.

use super::{cover_card, cover_grid, empty_state, header, headers_for, Badges};
use crate::app::{Action, App, BrowseTab, Page};
use crate::config::LibrarySort;
use crate::db::LibraryEntry;
use crate::model::{natural_cmp, Status};
use egui::Ui;
use material_3_ui::common;
use material_3_ui::icons::icons::*;
use material_3_ui::widgets;

#[derive(Default)]
pub struct State {
    pub query: String,
    /// `None`: all series.
    pub category: Option<i64>,
    pub unread_only: bool,
    pub downloaded_only: bool,
    pub ongoing_only: bool,
}

pub fn sort(entries: &mut [LibraryEntry], by: LibrarySort, descending: bool) {
    entries.sort_by(|a, b| {
        let ord = match by {
            LibrarySort::Title => natural_cmp(a.manga.title(), b.manga.title()),
            LibrarySort::LastRead => b.last_read.cmp(&a.last_read),
            LibrarySort::Added => b.manga.added_at.cmp(&a.manga.added_at),
            LibrarySort::Unread => b.unread.cmp(&a.unread),
            LibrarySort::LatestChapter => b.latest_upload.cmp(&a.latest_upload),
        };
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let all = app.library();
    let cats = app.db.categories().unwrap_or_default();
    let st = &mut app.pages.library;
    if st.category.is_some_and(|c| !cats.iter().any(|x| x.id == c)) {
        st.category = None;
    }
    let mut changed_cfg = false;

    header(ui, ICON_COLLECTIONS_BOOKMARK, "Library", &format!("{} series", all.len()), |ui| {
        // Sort.
        ui.menu_button(widgets::icon_text(ICON_SORT, 22.0), |ui| {
            for s in LibrarySort::ALL {
                if widgets::menu_item(ui, None, s.label(), "", app.cfg.library_sort == s) {
                    if app.cfg.library_sort == s {
                        app.cfg.library_sort_descending = !app.cfg.library_sort_descending;
                    } else {
                        app.cfg.library_sort = s;
                        app.cfg.library_sort_descending = false;
                    }
                    changed_cfg = true;
                }
            }
            ui.separator();
            if widgets::menu_item(ui, Some(ICON_SWAP_VERT), "Reverse order", "", app.cfg.library_sort_descending) {
                app.cfg.library_sort_descending = !app.cfg.library_sort_descending;
                changed_cfg = true;
            }
        })
        .response
        .on_hover_text("Sort");
        // Filters.
        let filtering = st.unread_only || st.downloaded_only || st.ongoing_only;
        let filter_icon = widgets::icon_text(ICON_FILTER_LIST, 22.0).color(if filtering { p.primary } else { p.on_surface_variant });
        ui.menu_button(filter_icon, |ui| {
            if widgets::menu_item(ui, None, "Unread", "", st.unread_only) {
                st.unread_only = !st.unread_only;
            }
            if widgets::menu_item(ui, None, "Downloaded", "", st.downloaded_only) {
                st.downloaded_only = !st.downloaded_only;
            }
            if widgets::menu_item(ui, None, "Ongoing", "", st.ongoing_only) {
                st.ongoing_only = !st.ongoing_only;
            }
            ui.separator();
            ui.label(egui::RichText::new("Cover size").size(12.5).color(p.on_surface_variant));
            if ui.add(egui::Slider::new(&mut app.cfg.cover_width, 100.0..=240.0).show_value(false)).changed() {
                changed_cfg = true;
            }
            if widgets::menu_item(ui, None, "Unread badges", "", app.cfg.unread_badges) {
                app.cfg.unread_badges = !app.cfg.unread_badges;
                changed_cfg = true;
            }
            if widgets::menu_item(ui, None, "Downloaded badges", "", app.cfg.download_badges) {
                app.cfg.download_badges = !app.cfg.download_badges;
                changed_cfg = true;
            }
        })
        .response
        .on_hover_text("Filter and display");
        ui.add_space(4.0);
        let r = common::search_field(ui, &mut st.query, "Search the library", 240.0);
        if app.pages.focus_search {
            app.pages.focus_search = false;
            r.request_focus();
        }
    });

    if !cats.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(24.0);
            let count = |c: Option<i64>| all.iter().filter(|e| c.is_none_or(|c| e.categories.contains(&c))).count();
            if widgets::filter_chip(ui, &format!("All  {}", count(None)), st.category.is_none(), None).clicked() {
                st.category = None;
            }
            for c in &cats {
                if widgets::filter_chip(ui, &format!("{}  {}", c.name, count(Some(c.id))), st.category == Some(c.id), None).clicked() {
                    st.category = Some(c.id);
                }
            }
        });
        ui.add_space(10.0);
    }

    if all.is_empty() {
        empty_state(
            ui,
            ICON_COLLECTIONS_BOOKMARK,
            "Your library is empty",
            "Find series in Browse and add them, or put comic files in ~/Comics (each series in its own folder) and add them from Local files.",
            Some(("Browse", Action::Goto(Page::Browse(BrowseTab::Sources)))),
            &mut actions,
        );
        app.actions.extend(actions);
        return;
    }

    let q = st.query.to_lowercase();
    let (unread_only, downloaded_only, ongoing_only, category) = (st.unread_only, st.downloaded_only, st.ongoing_only, st.category);
    let mut entries: Vec<LibraryEntry> = all
        .iter()
        .filter(|e| category.is_none_or(|c| e.categories.contains(&c)))
        .filter(|e| q.is_empty() || e.manga.title().to_lowercase().contains(&q) || e.manga.info.author.to_lowercase().contains(&q))
        .filter(|e| !unread_only || e.unread > 0)
        .filter(|e| !ongoing_only || e.manga.info.status != Status::Completed && e.manga.info.status != Status::Cancelled)
        .cloned()
        .collect();
    if downloaded_only {
        entries.retain(|e| app.downloads.count(&e.manga) > 0);
    }
    sort(&mut entries, app.cfg.library_sort, app.cfg.library_sort_descending);

    if entries.is_empty() {
        common::empty_hint(ui, "Nothing matches");
    }
    let server = app.image_headers(crate::sources::suwayomi::PREFIX);
    let width = app.cfg.cover_width;
    let (unread_badges, download_badges) = (app.cfg.unread_badges, app.cfg.download_badges);
    cover_grid(ui, "library-grid", entries.len(), width, |ui, i, w| {
        let e = &entries[i];
        let headers = headers_for(&e.manga.source, &server);
        let badges = Badges {
            unread: if unread_badges { e.unread } else { 0 },
            downloaded: if download_badges { app.downloads.count(&e.manga) } else { 0 },
            in_library: false,
        };
        let r = cover_card(ui, &mut app.covers, e.manga.info.cover.as_deref(), &headers, e.manga.title(), w, badges);
        if r.clicked() {
            actions.push(Action::Goto(Page::Series(e.manga.id)));
        }
        r.context_menu(|ui| {
            if widgets::menu_item(ui, Some(ICON_PLAY_ARROW), if e.last_read > 0 { "Continue reading" } else { "Start reading" }, "", false) {
                actions.push(Action::Continue(e.manga.id));
            }
            if widgets::menu_item(ui, Some(ICON_DONE_ALL), "Mark all as read", "", false) {
                let ids = app.db.chapters(e.manga.id).unwrap_or_default().iter().map(|c| c.id).collect();
                actions.push(Action::SetRead(ids, true));
            }
            if widgets::menu_item(ui, Some(ICON_LABEL), "Categories…", "", false) {
                actions.push(Action::EditCategories(e.manga.id));
            }
            ui.separator();
            if widgets::menu_item(ui, Some(ICON_DELETE), "Remove from library", "", false) {
                actions.push(Action::SetInLibrary(e.manga.id, false));
            }
        });
    });
    if changed_cfg {
        app.save_config();
    }
    app.actions.extend(actions);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Manga;
    use crate::model::MangaInfo;

    fn e(title: &str, unread: i64, last_read: i64) -> LibraryEntry {
        LibraryEntry { manga: Manga { info: MangaInfo { title: title.into(), ..Default::default() }, ..Default::default() }, unread, last_read, ..Default::default() }
    }

    #[test]
    fn sorting() {
        let mut v = vec![e("b10", 1, 5), e("B2", 7, 1), e("a", 0, 9)];
        sort(&mut v, LibrarySort::Title, false);
        assert_eq!(v.iter().map(|e| e.manga.title()).collect::<Vec<_>>(), vec!["a", "B2", "b10"]);
        sort(&mut v, LibrarySort::Unread, false);
        assert_eq!(v[0].manga.title(), "B2");
        sort(&mut v, LibrarySort::LastRead, true);
        assert_eq!(v[0].manga.title(), "B2");
    }
}
