//! The reader: one chapter at a time, as pages turned right to left or
//! left to right, or as a scrolling strip (vertical, webtoon). Clicking
//! the middle of the page (or M) shows the menu: chapter, page slider,
//! reading mode and fullscreen.

use crate::app::App;
use crate::config::{Background, Fit, PageLayout, ReadingMode};
use crate::db::{Chapter, Manga};
use crate::model::PageRef;
use crate::pages::{Page, PageKey, PageLoader, PageState};
use egui::{pos2, vec2, Color32, CornerRadius, Rect, Sense, Ui, UiBuilder};
use material_3_ui::icons::icons::*;
use material_3_ui::keys::take_key;
use material_3_ui::sys::task::Task;
use material_3_ui::theme::M3Palette;
use material_3_ui::widgets::{self, ButtonKind};
use std::collections::HashMap;
use std::sync::Arc;

/// Gap between pages in the vertical mode.
const GAP: f32 = 12.0;
/// Height of the cards between chapters in the scrolling modes.
const TRANSITION: f32 = 280.0;

pub struct Reader {
    pub manga: Manga,
    /// Newest first, as the database lists them.
    pub chapters: Vec<Chapter>,
    index: usize,
    pub mode: ReadingMode,
    loader: PageLoader,
    pages_task: Task<Result<Vec<PageRef>, String>>,
    pages: Option<Vec<PageRef>>,
    error: Option<String>,
    /// The current page. In the paged modes `pages.len()` is the card at
    /// the end of the chapter.
    page: usize,
    /// Open the chapter on its last page (after going back from the next).
    start_at_end: bool,
    reported: Option<usize>,
    pub menu: bool,
    pub fullscreen: bool,
    /// Page sizes seen so far, to lay out the strip before pages load.
    sizes: HashMap<usize, egui::Vec2>,
    /// The scrolling modes: (offset, page tops) from the last frame.
    strip: Option<(f32, Vec<f32>)>,
    jump: Option<usize>,
    /// A key scrolled the strip: apply the offset in `strip`.
    strip_forced: bool,
    /// The paged modes: the page fits, so the wheel turns pages.
    fit_screen_hint: bool,
    /// The scrolling modes: the end of the strip is in view.
    at_bottom: bool,
    /// Two pages side by side (set each frame from the layout setting).
    double: bool,
    cover_alone: bool,
    /// Pair pages the other way (for chapters whose spreads are off by
    /// one page).
    pub shift: bool,
    /// The last page on screen, for progress.
    last_shown: usize,
}

/// What the paged modes show at once: one page, or two in reading order
/// (first, second).
pub type Spread = (usize, Option<usize>);

/// A page wider than it is tall is a two-page spread already.
fn is_wide(size: egui::Vec2) -> bool {
    size.x > size.y * 1.1
}

/// Pair up `n` pages for the two-page view. Wide pages (spreads) stand
/// alone, as does the cover when `cover_alone`; pairing starts again after
/// each lone page, so pages around a spread line up the way the book
/// does. Pages whose size isn't known yet count as single pages.
pub fn spreads(n: usize, sizes: &HashMap<usize, egui::Vec2>, cover_alone: bool) -> Vec<Spread> {
    let wide = |i: usize| sizes.get(&i).is_some_and(|s| is_wide(*s));
    let mut out = Vec::with_capacity(n / 2 + 1);
    let mut i = 0;
    if cover_alone && n > 0 {
        out.push((0, None));
        i = 1;
    }
    while i < n {
        if wide(i) || i + 1 >= n || wide(i + 1) {
            out.push((i, None));
            i += 1;
        } else {
            out.push((i, Some(i + 1)));
            i += 2;
        }
    }
    out
}

/// What the reader asks the app for after drawing.
enum Event {
    Progress(Chapter, usize, usize),
    Chapter(usize, bool),
    Close,
    Mode(ReadingMode),
    Layout(PageLayout),
    Bookmark(bool),
}

impl Reader {
    pub fn new(ctx: &egui::Context, manga: Manga, chapters: Vec<Chapter>, chapter_id: i64, mode: ReadingMode, max_texture: u32) -> Self {
        let index = chapters.iter().position(|c| c.id == chapter_id).unwrap_or(0);
        Reader {
            manga,
            chapters,
            index,
            mode,
            loader: PageLoader::new(ctx, max_texture),
            pages_task: Task::default(),
            pages: None,
            error: None,
            page: 0,
            start_at_end: false,
            reported: None,
            menu: false,
            fullscreen: false,
            sizes: HashMap::new(),
            strip: None,
            jump: None,
            strip_forced: false,
            fit_screen_hint: true,
            at_bottom: false,
            double: false,
            cover_alone: true,
            shift: false,
            last_shown: 0,
        }
    }

    pub fn chapter(&self) -> &Chapter {
        &self.chapters[self.index]
    }

    /// Find the current chapter's pages on a worker thread.
    pub fn load_pages(&mut self, ctx: &egui::Context, f: impl FnOnce() -> Result<Vec<PageRef>, String> + Send + 'static) {
        self.pages = None;
        self.error = None;
        self.reported = None;
        self.sizes.clear();
        self.strip = None;
        self.loader.clear();
        self.pages_task.spawn(ctx, f);
    }

    /// The chapter after this one (newer), if any.
    fn next_index(&self) -> Option<usize> {
        self.index.checked_sub(1)
    }

    fn prev_index(&self) -> Option<usize> {
        (self.index + 1 < self.chapters.len()).then_some(self.index + 1)
    }

    fn page_count(&self) -> usize {
        self.pages.as_ref().map_or(0, Vec::len)
    }

    fn poll(&mut self) {
        if self.pages_task.poll() {
            match self.pages_task.value.take() {
                Some(Ok(pages)) => {
                    let n = pages.len();
                    let c = self.chapter();
                    self.page = if self.start_at_end {
                        n.saturating_sub(1)
                    } else if !c.read && c.last_page > 0 && (c.last_page as usize) < n {
                        c.last_page as usize
                    } else {
                        0
                    };
                    self.start_at_end = false;
                    self.jump = Some(self.page);
                    self.pages = Some(pages);
                }
                Some(Err(e)) => self.error = Some(e),
                None => {}
            }
        }
        self.loader.poll();
        for i in 0..self.page_count() {
            if !self.sizes.contains_key(&i) {
                if let Some(p) = self.loader.ready(self.key(i)) {
                    self.sizes.insert(i, p.size);
                }
            }
        }
    }

    /// Ask for the pages around the current one, current first.
    fn want(&mut self, preload: usize) {
        let Some(pages) = &self.pages else { return };
        let chapter = self.chapter().id;
        let n = pages.len();
        let cur = self.page.min(n.saturating_sub(1));
        let mut order = vec![cur];
        let ahead = if self.double { preload.max(4) } else { preload.max(1) };
        for i in 1..=ahead {
            if cur + i < n {
                order.push(cur + i);
            }
        }
        if cur > 0 {
            order.push(cur - 1);
        }
        if cur > 1 && (self.mode.continuous() || self.double) {
            order.push(cur - 2);
        }
        let list = order.into_iter().map(|i| (PageKey { chapter, index: i }, pages[i].clone())).collect();
        self.loader.want(list);
    }

    fn key(&self, index: usize) -> PageKey {
        PageKey { chapter: self.chapter().id, index }
    }

    /// The two-page view's spreads, or None in one-page view.
    fn spreads(&self) -> Option<Vec<Spread>> {
        (self.double && !self.mode.continuous()).then(|| spreads(self.page_count(), &self.sizes, self.cover_alone != self.shift))
    }

    /// The spread holding the current page.
    fn spread_at(list: &[Spread], page: usize) -> usize {
        list.iter().rposition(|s| s.0 <= page).unwrap_or(0)
    }
}

fn background(bg: Background, p: &M3Palette) -> Color32 {
    match bg {
        Background::Black => Color32::BLACK,
        Background::Gray => Color32::from_gray(48),
        Background::White => Color32::WHITE,
        Background::Theme => p.surface,
    }
}

fn paint_page(ui: &Ui, page: &Page, rect: Rect) {
    let scale = rect.width() / page.size.x.max(1.0);
    let mut y = rect.top();
    for (tex, h) in &page.strips {
        let r = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), h * scale));
        if ui.is_rect_visible(r) {
            ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        y += h * scale;
    }
}

/// A page still loading, or one that failed (with a retry button).
fn placeholder(ui: &mut Ui, state: &PageState, rect: Rect, fg: Color32) -> bool {
    let mut retry = false;
    match state {
        PageState::Failed(e) => {
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space((rect.height() / 2.0 - 60.0).max(8.0));
                    ui.label(widgets::icon_text(ICON_BROKEN_IMAGE, 40.0).color(fg));
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(e).color(fg).size(13.5));
                    ui.add_space(10.0);
                    retry = widgets::button(ui, ButtonKind::Tonal, Some(ICON_REFRESH), "Try again").clicked();
                });
            });
        }
        _ => {
            let spinner = Rect::from_center_size(rect.center(), vec2(32.0, 32.0));
            ui.put(spinner, egui::Spinner::new().size(32.0).color(fg));
        }
    }
    retry
}

fn page_text(r: &Reader) -> String {
    let n = r.page_count();
    if n == 0 {
        String::new()
    } else if r.last_shown > r.page && r.page < n {
        format!("{}–{} / {n}", r.page + 1, r.last_shown + 1)
    } else {
        format!("{} / {n}", (r.page + 1).min(n))
    }
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let Some(mut r) = app.reader.take() else { return };
    let mut events = Vec::new();
    r.cover_alone = app.cfg.reader.cover_alone;
    let screen = ui.ctx().content_rect();
    r.double = match app.cfg.reader.layout {
        PageLayout::Single => false,
        PageLayout::Double => true,
        PageLayout::Auto => screen.width() > screen.height(),
    };
    r.last_shown = r.page;
    r.poll();
    r.want(app.cfg.reader.preload);
    let p = app.palette.clone();
    let bg = background(app.cfg.reader.background, &p);
    let fg = if matches!(app.cfg.reader.background, Background::White) || (app.cfg.reader.background == Background::Theme && !p.is_dark()) {
        Color32::from_gray(40)
    } else {
        Color32::from_gray(220)
    };
    egui::CentralPanel::default().frame(egui::Frame::new().fill(bg)).show(ui, |ui| {
        let rect = ui.max_rect();
        keys(ui.ctx(), &mut r, app, &mut events, rect);
        if let Some(e) = r.error.clone() {
            chapter_error(ui, &r, &e, rect, fg, &mut events);
        } else if r.pages.is_none() {
            ui.put(Rect::from_center_size(rect.center(), vec2(40.0, 40.0)), egui::Spinner::new().size(40.0).color(fg));
        } else if r.mode.continuous() {
            strip(ui, &mut r, app, rect, fg, &mut events);
        } else {
            paged(ui, &mut r, app, rect, fg, &mut events);
        }
        if r.menu {
            menu(ui, &mut r, app.cfg.reader.layout, rect, &p, &mut events);
        } else if app.cfg.reader.show_page_number && r.page_count() > 0 && r.page < r.page_count() && !(r.mode.continuous() && r.at_bottom) {
            let text = page_text(&r);
            let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(12.5), Color32::WHITE);
            let size = galley.size() + vec2(20.0, 8.0);
            let chip = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 10.0 - size.y / 2.0), size);
            ui.painter().rect_filled(chip, CornerRadius::same(12), Color32::from_black_alpha(150));
            ui.painter().galley(chip.center() - galley.size() / 2.0, galley, Color32::WHITE);
        }
    });
    // Report the page once it changes.
    let n = r.page_count();
    let shown = r.last_shown.max(r.page);
    if n > 0 && r.page < n && r.reported != Some(shown) {
        r.reported = Some(shown);
        events.push(Event::Progress(r.chapter().clone(), shown.min(n - 1), n));
    }
    app.reader = Some(r);
    for e in events {
        handle(app, e);
    }
}

fn handle(app: &mut App, e: Event) {
    match e {
        Event::Progress(chapter, page, pages) => {
            let id = chapter.id;
            app.reader_progress(chapter, page, pages);
            if let Some(r) = &mut app.reader {
                if let Ok(Some(c)) = app.db.chapter(id) {
                    if let Some(slot) = r.chapters.iter_mut().find(|x| x.id == id) {
                        *slot = c;
                    }
                }
            }
        }
        Event::Chapter(index, at_end) => {
            if let Some(r) = &mut app.reader {
                if index < r.chapters.len() {
                    r.index = index;
                    r.start_at_end = at_end;
                    app.start_reader_chapter();
                }
            }
        }
        Event::Close => {
            if let Some(r) = app.reader.take() {
                if r.fullscreen {
                    app.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                }
                app.dirty();
            }
        }
        Event::Mode(mode) => {
            if let Some(r) = &mut app.reader {
                r.mode = mode;
                r.strip = None;
                r.jump = Some(r.page.min(r.page_count().saturating_sub(1)));
                let id = r.manga.id;
                let _ = app.db.set_reader_mode(id, if mode == app.cfg.reader.mode { None } else { Some(mode) });
            }
        }
        Event::Layout(layout) => {
            app.cfg.reader.layout = layout;
            app.save_config();
        }
        Event::Bookmark(on) => {
            if let Some(r) = &mut app.reader {
                let i = r.index;
                r.chapters[i].bookmark = on;
                let _ = app.db.set_bookmark(&[r.chapters[i].id], on);
                app.dirty();
            }
        }
    }
}

fn toggle_fullscreen(ctx: &egui::Context, r: &mut Reader) {
    r.fullscreen = !r.fullscreen;
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(r.fullscreen));
}

/// Forward one page (or on to the next chapter).
fn forward(r: &mut Reader, continue_to_next: bool, events: &mut Vec<Event>) {
    let n = r.page_count();
    if r.page < n {
        r.page = match r.spreads() {
            Some(list) => list.get(Reader::spread_at(&list, r.page) + 1).map_or(n, |s| s.0),
            None => r.page + 1,
        };
    } else if let Some(next) = r.next_index() {
        if continue_to_next {
            events.push(Event::Chapter(next, false));
        }
    }
}

fn backward(r: &mut Reader, events: &mut Vec<Event>) {
    if r.page > 0 {
        r.page = match r.spreads() {
            Some(list) if r.page >= r.page_count() => list.last().map_or(0, |s| s.0),
            Some(list) => list[Reader::spread_at(&list, r.page).saturating_sub(1)].0,
            None => r.page - 1,
        };
    } else if let Some(prev) = r.prev_index() {
        events.push(Event::Chapter(prev, true));
    }
}

fn keys(ctx: &egui::Context, r: &mut Reader, app: &App, events: &mut Vec<Event>, rect: Rect) {
    use egui::{Key, Modifiers};
    let none = Modifiers::NONE;
    if take_key(ctx, none, Key::Escape) {
        if r.menu {
            r.menu = false;
        } else if r.fullscreen {
            toggle_fullscreen(ctx, r);
        } else {
            events.push(Event::Close);
        }
    }
    if take_key(ctx, none, Key::Backspace) || take_key(ctx, Modifiers::ALT, Key::ArrowLeft) {
        events.push(Event::Close);
    }
    if take_key(ctx, none, Key::F11) || take_key(ctx, none, Key::F) {
        toggle_fullscreen(ctx, r);
    }
    if take_key(ctx, none, Key::M) {
        r.menu = !r.menu;
    }
    if !r.mode.continuous() {
        if take_key(ctx, none, Key::D) {
            events.push(Event::Layout(if r.double { PageLayout::Single } else { PageLayout::Double }));
        }
        if take_key(ctx, none, Key::S) && r.double {
            r.shift = !r.shift;
        }
    }
    if take_key(ctx, none, Key::CloseBracket) || take_key(ctx, none, Key::N) {
        if let Some(i) = r.next_index() {
            events.push(Event::Chapter(i, false));
        }
    }
    if take_key(ctx, none, Key::OpenBracket) || take_key(ctx, none, Key::P) {
        if let Some(i) = r.prev_index() {
            events.push(Event::Chapter(i, false));
        }
    }
    if r.pages.is_none() {
        return;
    }
    let next = app.cfg.reader.continue_to_next;
    if r.mode.continuous() {
        let mut delta = 0.0;
        if take_key(ctx, none, Key::ArrowDown) || take_key(ctx, none, Key::J) {
            delta += 120.0;
        }
        if take_key(ctx, none, Key::ArrowUp) || take_key(ctx, none, Key::K) {
            delta -= 120.0;
        }
        if take_key(ctx, none, Key::Space) || take_key(ctx, none, Key::PageDown) || take_key(ctx, none, Key::ArrowRight) {
            delta += rect.height() * 0.85;
        }
        if take_key(ctx, Modifiers::SHIFT, Key::Space) || take_key(ctx, none, Key::PageUp) || take_key(ctx, none, Key::ArrowLeft) {
            delta -= rect.height() * 0.85;
        }
        if take_key(ctx, none, Key::Home) {
            r.jump = Some(0);
        }
        if take_key(ctx, none, Key::End) {
            r.jump = Some(r.page_count().saturating_sub(1));
        }
        if delta > 0.0 && r.at_bottom && next {
            if let Some(i) = r.next_index() {
                events.push(Event::Chapter(i, false));
                return;
            }
        }
        if delta != 0.0 {
            if let Some((offset, _)) = &mut r.strip {
                *offset = (*offset + delta).max(0.0);
                r.jump = None;
                // Picked up as a forced offset this frame.
                r.strip_forced = true;
            }
        }
        return;
    }
    let rtl = r.mode == ReadingMode::RightToLeft;
    let (left, right) = (take_key(ctx, none, Key::ArrowLeft) || take_key(ctx, none, Key::H), take_key(ctx, none, Key::ArrowRight) || take_key(ctx, none, Key::L));
    if (left && rtl) || (right && !rtl) || take_key(ctx, none, Key::Space) || take_key(ctx, none, Key::PageDown) || take_key(ctx, none, Key::ArrowDown) {
        forward(r, next, events);
    }
    if (right && rtl) || (left && !rtl) || take_key(ctx, Modifiers::SHIFT, Key::Space) || take_key(ctx, none, Key::PageUp) || take_key(ctx, none, Key::ArrowUp) {
        backward(r, events);
    }
    if take_key(ctx, none, Key::Home) {
        r.page = 0;
    }
    if take_key(ctx, none, Key::End) {
        r.page = r.page_count().saturating_sub(1);
    }
    let scroll: f32 = ctx.input(|i| {
        i.raw.events.iter().map(|e| match e {
            egui::Event::MouseWheel { delta, .. } => delta.y,
            _ => 0.0,
        }).sum()
    });
    if scroll.abs() > 0.0 && !r.menu && !ctx.input(|i| i.modifiers.ctrl) {
        // A mouse wheel notch turns the page when it fits on screen.
        if r.fit_screen_hint {
            if scroll < 0.0 {
                forward(r, next, events);
            } else {
                backward(r, events);
            }
        }
    }
}

fn chapter_error(ui: &mut Ui, r: &Reader, e: &str, rect: Rect, fg: Color32, events: &mut Vec<Event>) {
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink(32.0)), |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(rect.height() / 2.0 - 100.0);
            ui.label(widgets::icon_text(ICON_ERROR, 44.0).color(fg));
            ui.add_space(8.0);
            ui.label(widgets::title_text(format!("Couldn't open {}", r.chapter().name), 18.0).color(fg));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(e).color(fg).size(13.5));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let w = 260.0;
                ui.add_space((ui.available_width() - w).max(0.0) / 2.0);
                if widgets::button(ui, ButtonKind::Text, Some(ICON_ARROW_BACK), "Back").clicked() {
                    events.push(Event::Close);
                }
                if widgets::button(ui, ButtonKind::Filled, Some(ICON_REFRESH), "Try again").clicked() {
                    events.push(Event::Chapter(r.index, false));
                }
            });
        });
    });
}

/// The card shown after the last page (and before the first in the strip).
fn transition(ui: &mut Ui, r: &Reader, rect: Rect, fg: Color32, at_end: bool, events: &mut Vec<Event>) {
    let (other, verb) = if at_end { (r.next_index(), "Next") } else { (r.prev_index(), "Previous") };
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink(24.0)), |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((rect.height() / 2.0 - 90.0).max(8.0));
            let done = if at_end { "Finished" } else { "Current" };
            ui.label(egui::RichText::new(done).color(fg.gamma_multiply(0.7)).size(13.0));
            ui.label(widgets::title_text(r.chapter().name.clone(), 17.0).color(fg));
            ui.add_space(14.0);
            match other {
                Some(i) => {
                    let c = &r.chapters[i];
                    ui.label(egui::RichText::new(verb).color(fg.gamma_multiply(0.7)).size(13.0));
                    ui.label(widgets::title_text(c.name.clone(), 17.0).color(fg));
                    ui.add_space(14.0);
                    let icon = if at_end { ICON_SKIP_NEXT } else { ICON_SKIP_PREVIOUS };
                    if widgets::button(ui, ButtonKind::Filled, Some(icon), &format!("{verb} chapter")).clicked() {
                        events.push(Event::Chapter(i, !at_end));
                    }
                }
                None => {
                    let msg = if at_end { "There's no next chapter" } else { "There's no previous chapter" };
                    ui.label(egui::RichText::new(msg).color(fg).size(14.0));
                    if at_end {
                        ui.add_space(14.0);
                        if widgets::button(ui, ButtonKind::Tonal, Some(ICON_ARROW_BACK), "Back to the series").clicked() {
                            events.push(Event::Close);
                        }
                    }
                }
            }
        });
    });
}

/// The pages one (or two) at a time.
fn paged(ui: &mut Ui, r: &mut Reader, app: &App, rect: Rect, fg: Color32, events: &mut Vec<Event>) {
    let n = r.page_count();
    r.page = r.page.min(n);
    let next = app.cfg.reader.continue_to_next;
    let ppp = ui.ctx().pixels_per_point();
    r.fit_screen_hint = true;
    let clicked = if r.page == n {
        transition(ui, r, rect, fg, true, events);
        let resp = ui.interact(rect, ui.scope_id().with("end"), Sense::click());
        resp.clicked().then(|| resp.interact_pointer_pos()).flatten()
    } else {
        let spread = match r.spreads() {
            Some(list) => list[Reader::spread_at(&list, r.page)],
            None => (r.page, None),
        };
        r.page = spread.0;
        r.last_shown = spread.1.unwrap_or(spread.0);
        // Reading order on screen: right to left puts the first page on the right.
        let mut on_screen: Vec<usize> = std::iter::once(spread.0).chain(spread.1).collect();
        if r.mode == ReadingMode::RightToLeft {
            on_screen.reverse();
        }
        show_spread(ui, r, &on_screen, app.cfg.reader.fit, ppp, rect, fg)
    };
    if let Some(pos) = clicked {
        let third = rect.width() / 3.0;
        let rtl = r.mode == ReadingMode::RightToLeft;
        if !app.cfg.reader.tap_zones || r.menu || (pos.x > rect.left() + third && pos.x < rect.right() - third) {
            r.menu = !r.menu;
        } else if (pos.x <= rect.left() + third) == rtl {
            forward(r, next, events);
        } else {
            backward(r, events);
        }
    }
}

/// Draw pages side by side, left to right, at one height. Returns where
/// the page was clicked.
fn show_spread(ui: &mut Ui, r: &mut Reader, pages: &[usize], fit: Fit, ppp: f32, rect: Rect, fg: Color32) -> Option<egui::Pos2> {
    let states: Vec<(PageKey, Option<Arc<Page>>)> = pages
        .iter()
        .map(|&i| {
            let key = r.key(i);
            (key, r.loader.ready(key))
        })
        .collect();
    // Width over height of each page; a loading page borrows its
    // neighbour's shape.
    let known: Vec<f32> = states.iter().filter_map(|(_, p)| p.as_ref().map(|p| p.size.x / p.size.y.max(1.0))).collect();
    let fallback = known.first().copied().unwrap_or(0.7);
    let aspects: Vec<f32> = states.iter().map(|(_, p)| p.as_ref().map_or(fallback, |p| p.size.x / p.size.y.max(1.0))).collect();
    let total_aspect: f32 = aspects.iter().sum();
    let tallest = states.iter().filter_map(|(_, p)| p.as_ref().map(|p| p.size.y)).fold(0.0, f32::max);
    let height = match fit {
        Fit::Screen => rect.height().min(rect.width() / total_aspect),
        Fit::Width => rect.width() / total_aspect,
        Fit::Height => rect.height(),
        Fit::Original if tallest > 0.0 => tallest / ppp,
        Fit::Original => rect.height(),
    };
    let size = vec2(height * total_aspect, height);
    let fits = size.x <= rect.width() + 0.5 && size.y <= rect.height() + 0.5;
    r.fit_screen_hint = fits;
    let mut retry = Vec::new();
    let mut draw = |ui: &mut Ui, target: Rect| {
        let mut x = target.left();
        for ((key, page), aspect) in states.iter().zip(&aspects) {
            let w = height * aspect;
            let slot = Rect::from_min_size(pos2(x, target.top()), vec2(w, height));
            match page {
                Some(p) => paint_page(ui, p, slot),
                None => {
                    if placeholder(ui, r.loader.get(*key), slot, fg) {
                        retry.push(*key);
                    }
                }
            }
            x += w;
        }
    };
    let pos = if fits {
        draw(ui, Rect::from_center_size(rect.center(), size));
        let resp = ui.interact(rect, ui.scope_id().with("page"), Sense::click());
        resp.clicked().then(|| resp.interact_pointer_pos()).flatten()
    } else {
        let mut pos = None;
        let first = states[0].0;
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::both().id_salt(("page", first.chapter, first.index)).auto_shrink([false, false]).show(ui, |ui| {
                let avail = ui.available_size();
                let outer = vec2(size.x.max(avail.x), size.y.max(avail.y));
                let (area, resp) = ui.allocate_exact_size(outer, Sense::click());
                draw(ui, Rect::from_center_size(area.center(), size));
                if resp.clicked() {
                    pos = resp.interact_pointer_pos();
                }
            });
        });
        pos
    };
    for key in retry {
        r.loader.retry(key);
    }
    pos
}

/// Where each page starts in the strip, and the strip's total height.
fn strip_layout(r: &Reader, width: f32, gap: f32) -> (Vec<f32>, f32) {
    let n = r.page_count();
    let known: Vec<f32> = r.sizes.values().map(|s| s.y / s.x.max(1.0)).collect();
    let typical = if known.is_empty() { 1.45 } else { known.iter().sum::<f32>() / known.len() as f32 };
    let mut tops = Vec::with_capacity(n + 1);
    let mut y = TRANSITION;
    for i in 0..n {
        tops.push(y);
        let aspect = r.sizes.get(&i).map_or(typical, |s| s.y / s.x.max(1.0));
        y += width * aspect + gap;
    }
    tops.push(y);
    (tops, y + TRANSITION)
}

/// The pages one under another (vertical with gaps, webtoon without).
fn strip(ui: &mut Ui, r: &mut Reader, app: &App, rect: Rect, fg: Color32, events: &mut Vec<Event>) {
    let n = r.page_count();
    for i in 0..n {
        if let Some(p) = r.loader.ready(r.key(i)) {
            r.sizes.insert(i, p.size);
        }
    }
    let share = if r.fullscreen { app.cfg.reader.strip_width.max(0.6) } else { app.cfg.reader.strip_width };
    let width = (rect.width() * share.clamp(0.2, 1.0)).min(rect.width()).max(200.0_f32.min(rect.width()));
    let gap = if r.mode == ReadingMode::Webtoon { 0.0 } else { GAP };
    let (tops, total) = strip_layout(r, width, gap);
    // Keep the same spot in view when pages above change size.
    let mut forced = None;
    if let Some(i) = r.jump.take() {
        forced = Some(tops.get(i).copied().unwrap_or(0.0) - if i == 0 { TRANSITION * 0.5 } else { 0.0 });
    } else if let Some((offset, old)) = &r.strip {
        if r.strip_forced || *old != tops {
            let i = old.iter().rposition(|t| t <= offset).unwrap_or(0);
            let new = if i + 1 < old.len() && i + 1 < tops.len() {
                let frac = (offset - old[i]) / (old[i + 1] - old[i]).max(1.0);
                tops[i] + frac * (tops[i + 1] - tops[i])
            } else {
                *offset - old.first().copied().unwrap_or(0.0) + tops.first().copied().unwrap_or(0.0)
            };
            forced = Some(new);
        }
    }
    r.strip_forced = false;
    let mut retry = Vec::new();
    let mut clicked = None;
    let out = ui
        .scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.spacing_mut().scroll.fade.strength = 0.0;
            let mut area = egui::ScrollArea::vertical().id_salt(("strip", r.chapter().id)).auto_shrink([false, false]);
            if let Some(y) = forced {
                area = area.vertical_scroll_offset(y.clamp(0.0, (total - rect.height()).max(0.0)));
            }
            area.show_viewport(ui, |ui, viewport| {
                let (full, resp) = ui.allocate_exact_size(vec2(ui.available_width(), total), Sense::click());
                let left = full.center().x - width / 2.0;
                let origin = full.top();
                // Before the first page and after the last.
                let top_card = Rect::from_min_size(pos2(left, origin), vec2(width, TRANSITION));
                if viewport.min.y < TRANSITION {
                    transition(ui, r, top_card, fg, false, events);
                }
                let end = tops[n];
                if viewport.max.y > end {
                    let end_card = Rect::from_min_size(pos2(left, origin + end), vec2(width, TRANSITION));
                    transition(ui, r, end_card, fg, true, events);
                }
                for i in 0..n {
                    let page_rect = Rect::from_min_max(pos2(left, origin + tops[i]), pos2(left + width, origin + tops[i + 1] - gap));
                    if page_rect.max.y < origin + viewport.min.y || page_rect.min.y > origin + viewport.max.y {
                        continue;
                    }
                    match r.loader.get(r.key(i)) {
                        PageState::Ready(p) => paint_page(ui, p, page_rect),
                        state => {
                            if placeholder(ui, state, page_rect, fg) {
                                retry.push(i);
                            }
                        }
                    }
                }
                // The current page: the one across the middle of the view.
                let mid = viewport.center().y;
                r.page = tops.iter().take(n).rposition(|t| *t <= mid).unwrap_or(0);
                r.at_bottom = viewport.max.y >= total - 1.0;
                if resp.clicked() {
                    clicked = resp.interact_pointer_pos();
                }
            })
        })
        .inner;
    r.strip = Some((out.state.clamped_offset().y, tops));
    for i in retry {
        let key = r.key(i);
        r.loader.retry(key);
    }
    if clicked.is_some() {
        r.menu = !r.menu;
    }
}

fn menu(ui: &mut Ui, r: &mut Reader, layout: PageLayout, rect: Rect, p: &M3Palette, events: &mut Vec<Event>) {
    let ctx = ui.ctx().clone();
    let bar_fill = p.surface_container;
    // Top bar: back, titles, bookmark, mode, fullscreen.
    let top = Rect::from_min_size(rect.min, vec2(rect.width(), 64.0));
    ui.painter().rect_filled(top, CornerRadius::ZERO, bar_fill);
    let _ = ui.interact(top, ui.scope_id().with("menu-top"), Sense::click());
    ui.scope_builder(UiBuilder::new().max_rect(top.shrink2(vec2(12.0, 8.0))), |ui| {
        ui.horizontal_centered(|ui| {
            if widgets::icon_button(ui, ICON_ARROW_BACK, "Back (Esc)", true).clicked() {
                events.push(Event::Close);
            }
            ui.add_space(6.0);
            let text_w = (ui.available_width() - 200.0).max(80.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                let t = widgets::truncated_galley(ui, r.manga.title(), widgets::title_font(16.0), p.on_surface, text_w);
                ui.label(t);
                let s = widgets::truncated_galley(ui, &r.chapter().name, egui::FontId::proportional(13.0), p.on_surface_variant, text_w);
                ui.label(s);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (icon, tip) = if r.fullscreen { (ICON_FULLSCREEN_EXIT, "Exit fullscreen (F11)") } else { (ICON_FULLSCREEN, "Fullscreen (F11)") };
                if widgets::icon_button(ui, icon, tip, true).clicked() {
                    toggle_fullscreen(&ctx, r);
                }
                let mode_icon = match r.mode {
                    ReadingMode::RightToLeft | ReadingMode::LeftToRight => ICON_MENU_BOOK,
                    ReadingMode::Vertical => ICON_VIEW_DAY,
                    ReadingMode::Webtoon => ICON_CHROME_READER_MODE,
                };
                let resp = widgets::icon_button(ui, mode_icon, &format!("Reading mode: {}", r.mode.label()), true);
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_min_width(200.0);
                    for m in ReadingMode::ALL {
                        if widgets::menu_item(ui, None, m.label(), "", m == r.mode) {
                            events.push(Event::Mode(m));
                            ui.close();
                        }
                    }
                });
                if !r.mode.continuous() {
                    let icon = if r.double { ICON_IMPORT_CONTACTS } else { ICON_CROP_PORTRAIT };
                    let resp = widgets::icon_button(ui, icon, "Page layout (D)", true);
                    egui::Popup::menu(&resp).show(|ui| {
                        ui.set_min_width(240.0);
                        for l in PageLayout::ALL {
                            if widgets::menu_item(ui, None, l.label(), "", l == layout) {
                                events.push(Event::Layout(l));
                                ui.close();
                            }
                        }
                        ui.separator();
                        if widgets::menu_item(ui, Some(ICON_SWAP_HORIZ), "Shift pairs", "S", r.shift) {
                            r.shift = !r.shift;
                            ui.close();
                        }
                    });
                }
                let marked = r.chapter().bookmark;
                let tip = if marked { "Remove bookmark" } else { "Bookmark this chapter" };
                if widgets::icon_button_sized(ui, ICON_BOOKMARK, tip, true, 36.0, marked).clicked() {
                    events.push(Event::Bookmark(!marked));
                }
            });
        });
    });
    // Bottom bar: chapters and the page slider.
    let bottom = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 64.0), rect.max);
    ui.painter().rect_filled(bottom, CornerRadius::ZERO, bar_fill);
    let _ = ui.interact(bottom, ui.scope_id().with("menu-bottom"), Sense::click());
    let n = r.page_count();
    let rtl = r.mode == ReadingMode::RightToLeft;
    ui.scope_builder(UiBuilder::new().max_rect(bottom.shrink2(vec2(12.0, 8.0))), |ui| {
        ui.horizontal_centered(|ui| {
            // In right-to-left the previous chapter sits on the right.
            let (first, second) = if rtl { (r.next_index(), r.prev_index()) } else { (r.prev_index(), r.next_index()) };
            let (first_tip, second_tip) = if rtl { ("Next chapter", "Previous chapter") } else { ("Previous chapter", "Next chapter") };
            if widgets::icon_button(ui, ICON_SKIP_PREVIOUS, first_tip, first.is_some()).clicked() {
                if let Some(i) = first {
                    events.push(Event::Chapter(i, false));
                }
            }
            let label_w = 70.0;
            let slider_w = (ui.available_width() - 36.0 - label_w - 16.0).max(60.0);
            if n > 1 {
                let mut value = r.page.min(n - 1) as f32;
                ui.spacing_mut().slider_width = slider_w;
                let last = (n - 1) as f32;
                let range = if rtl { last..=0.0 } else { 0.0..=last };
                let resp = ui.add(egui::Slider::new(&mut value, range).step_by(1.0).show_value(false).trailing_fill(!rtl));
                if resp.changed() {
                    r.page = value.round() as usize;
                    if r.mode.continuous() {
                        r.jump = Some(r.page);
                    }
                }
            } else {
                ui.add_space(slider_w);
            }
            ui.add_sized(vec2(label_w, 20.0), egui::Label::new(egui::RichText::new(page_text(r)).color(p.on_surface).size(13.0)));
            if widgets::icon_button(ui, ICON_SKIP_NEXT, second_tip, second.is_some()).clicked() {
                if let Some(i) = second {
                    events.push(Event::Chapter(i, false));
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(pages: usize, chapters: usize, current: usize) -> Reader {
        let ctx = egui::Context::default();
        let chapters: Vec<Chapter> = (0..chapters).map(|i| Chapter { id: i as i64 + 1, name: format!("Chapter {}", chapters - i), ..Default::default() }).collect();
        let id = chapters[current].id;
        let mut r = Reader::new(&ctx, Manga::default(), chapters, id, ReadingMode::RightToLeft, 2048);
        r.pages = Some((0..pages).map(|i| PageRef::File(format!("/x/{i}.png").into())).collect());
        r
    }

    #[test]
    fn turning_pages_moves_between_chapters() {
        // Chapters newest first: index 1 is the middle one.
        let mut r = reader(3, 3, 1);
        let mut events = Vec::new();
        forward(&mut r, true, &mut events);
        forward(&mut r, true, &mut events);
        forward(&mut r, true, &mut events);
        assert_eq!(r.page, 3, "the end card");
        assert!(events.is_empty());
        forward(&mut r, true, &mut events);
        assert!(matches!(events.as_slice(), [Event::Chapter(0, false)]), "on to the newer chapter");
        events.clear();
        r.page = 0;
        backward(&mut r, &mut events);
        assert!(matches!(events.as_slice(), [Event::Chapter(2, true)]), "back to the older one's last page");
        events.clear();
        let mut r = reader(1, 1, 0);
        forward(&mut r, true, &mut events);
        forward(&mut r, true, &mut events);
        backward(&mut r, &mut events);
        assert!(events.is_empty());
        assert_eq!(r.page, 0);
    }

    #[test]
    fn strip_layout_uses_known_sizes() {
        let mut r = reader(3, 1, 0);
        r.sizes.insert(0, vec2(100.0, 300.0));
        let (tops, total) = strip_layout(&r, 100.0, 0.0);
        assert_eq!(tops, vec![TRANSITION, TRANSITION + 300.0, TRANSITION + 600.0, TRANSITION + 900.0]);
        assert_eq!(total, TRANSITION * 2.0 + 900.0);
    }

    fn sizes(wide: &[usize], n: usize) -> HashMap<usize, egui::Vec2> {
        (0..n).map(|i| (i, if wide.contains(&i) { vec2(1600.0, 1200.0) } else { vec2(800.0, 1200.0) })).collect()
    }

    #[test]
    fn pairs_pages_around_the_cover_and_spreads() {
        // Cover alone, then pairs.
        assert_eq!(spreads(5, &sizes(&[], 5), true), vec![(0, None), (1, Some(2)), (3, Some(4))]);
        // Without the cover rule (or shifted), pairs start at the first page.
        assert_eq!(spreads(5, &sizes(&[], 5), false), vec![(0, Some(1)), (2, Some(3)), (4, None)]);
        // A wide page stands alone, and pairing starts again after it:
        // page 3 has no partner before the spread, 5 and 6 pair after it.
        assert_eq!(spreads(7, &sizes(&[4], 7), true), vec![(0, None), (1, Some(2)), (3, None), (4, None), (5, Some(6))]);
        // A wide cover is a spread on its own either way.
        assert_eq!(spreads(3, &sizes(&[0], 3), false), vec![(0, None), (1, Some(2))]);
        // Unknown sizes count as single pages.
        assert_eq!(spreads(3, &HashMap::new(), true), vec![(0, None), (1, Some(2))]);
        assert!(spreads(0, &HashMap::new(), true).is_empty());
    }

    #[test]
    fn two_page_view_turns_by_spread() {
        let mut r = reader(6, 2, 1);
        r.double = true;
        r.sizes = sizes(&[3], 6);
        let mut events = Vec::new();
        // (0) (1,2) (3) (4,5) then the end card.
        let mut seen = vec![r.page];
        for _ in 0..4 {
            forward(&mut r, true, &mut events);
            seen.push(r.page);
        }
        assert_eq!(seen, vec![0, 1, 3, 4, 6]);
        backward(&mut r, &mut events);
        assert_eq!(r.page, 4, "back from the end card to the last spread");
        backward(&mut r, &mut events);
        backward(&mut r, &mut events);
        assert_eq!(r.page, 1);
        // Shift flips the cover rule.
        r.shift = true;
        assert_eq!(r.spreads().unwrap()[0], (0, Some(1)));
        assert!(events.is_empty());
    }
}
