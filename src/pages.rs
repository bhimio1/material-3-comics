//! Loading the reader's pages: fetched and decoded on worker threads in the
//! order the reader asks for them (the current page first, then the ones
//! ahead), and kept as textures while they're near the current page.
//!
//! Long webtoon strips are cut into pieces no taller than the GPU's
//! texture limit, and pages wider than it are scaled down.

use crate::model::PageRef;
use egui::{ColorImage, TextureHandle, TextureOptions, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex};

const WORKERS: usize = 3;
const STRIP: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageKey {
    pub chapter: i64,
    pub index: usize,
}

/// A decoded page: its pieces, top to bottom.
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub strips: Vec<ColorImage>,
}

pub struct Page {
    /// Pixel size of the whole page.
    pub size: Vec2,
    /// (texture, height in pixels), top to bottom.
    pub strips: Vec<(TextureHandle, f32)>,
}

pub enum PageState {
    Loading,
    Ready(Arc<Page>),
    Failed(String),
}

#[derive(Default)]
struct Wanted {
    list: Vec<(PageKey, PageRef)>,
    busy: HashSet<PageKey>,
    /// Loaded (or failed) and still held by the UI.
    held: HashSet<PageKey>,
}

struct Shared {
    wanted: Mutex<Wanted>,
    wake: Condvar,
}

pub struct PageLoader {
    shared: Arc<Shared>,
    results: flume::Receiver<(PageKey, Result<Decoded, String>)>,
    cache: HashMap<PageKey, PageState>,
    ctx: egui::Context,
    last_wanted: Vec<PageKey>,
}

/// Decode page bytes, scale them to fit `max_side` wide, and cut them
/// into strips.
pub fn decode(bytes: &[u8], max_side: u32) -> Result<Decoded, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("Can't show this page: {e}"))?;
    let img = if img.width() > max_side {
        let h = ((img.height() as f64) * max_side as f64 / img.width() as f64).round().max(1.0) as u32;
        img.resize_exact(max_side, h, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let strip = STRIP.min(max_side).max(256);
    let mut strips = Vec::new();
    let mut y = 0;
    while y < h {
        let sh = strip.min(h - y);
        let start = (y * w * 4) as usize;
        let end = ((y + sh) * w * 4) as usize;
        strips.push(ColorImage::from_rgba_unmultiplied([w as usize, sh as usize], &rgba.as_raw()[start..end]));
        y += sh;
    }
    Ok(Decoded { width: w, height: h, strips })
}

fn worker(shared: Arc<Shared>, tx: flume::Sender<(PageKey, Result<Decoded, String>)>, ctx: egui::Context, max_side: u32) {
    let agent = crate::sources::agent();
    loop {
        let (key, page) = {
            let mut w = shared.wanted.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let next = w.list.iter().find(|(k, _)| !w.busy.contains(k) && !w.held.contains(k)).cloned();
                if let Some(n) = next {
                    w.busy.insert(n.0);
                    break n;
                }
                w = shared.wake.wait(w).unwrap_or_else(|e| e.into_inner());
            }
        };
        let result = crate::sources::fetch_page(&agent, &page).map_err(|e| e.to_string()).and_then(|b| decode(&b, max_side));
        {
            let mut w = shared.wanted.lock().unwrap_or_else(|e| e.into_inner());
            w.busy.remove(&key);
            w.held.insert(key);
        }
        if tx.send((key, result)).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

impl PageLoader {
    pub fn new(ctx: &egui::Context, max_side: u32) -> Self {
        let shared = Arc::new(Shared { wanted: Mutex::new(Wanted::default()), wake: Condvar::new() });
        let (tx, results) = flume::unbounded();
        for i in 0..WORKERS {
            let shared = shared.clone();
            let tx = tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("m3-comics-page-{i}"))
                .spawn(move || worker(shared, tx, ctx, max_side))
                .expect("spawn a page thread");
        }
        PageLoader { shared, results, cache: HashMap::new(), ctx: ctx.clone(), last_wanted: Vec::new() }
    }

    /// The pages to have ready, most urgent first. Pages not listed are
    /// dropped.
    pub fn want(&mut self, list: Vec<(PageKey, PageRef)>) {
        let keys: Vec<PageKey> = list.iter().map(|(k, _)| *k).collect();
        if keys == self.last_wanted {
            return;
        }
        let keep: HashSet<PageKey> = keys.iter().copied().collect();
        self.cache.retain(|k, _| keep.contains(k));
        {
            let mut w = self.shared.wanted.lock().unwrap_or_else(|e| e.into_inner());
            w.list = list;
            w.held.retain(|k| keep.contains(k));
        }
        self.last_wanted = keys;
        self.shared.wake.notify_all();
    }

    pub fn poll(&mut self) {
        for (key, result) in self.results.try_iter() {
            if !self.last_wanted.contains(&key) {
                let mut w = self.shared.wanted.lock().unwrap_or_else(|e| e.into_inner());
                w.held.remove(&key);
                continue;
            }
            let state = match result {
                Ok(d) => {
                    let opts = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
                    let strips = d
                        .strips
                        .into_iter()
                        .enumerate()
                        .map(|(i, img)| {
                            let h = img.size[1] as f32;
                            (self.ctx.load_texture(format!("page-{}-{}-{i}", key.chapter, key.index), img, opts), h)
                        })
                        .collect();
                    PageState::Ready(Arc::new(Page { size: Vec2::new(d.width as f32, d.height as f32), strips }))
                }
                Err(e) => PageState::Failed(e),
            };
            self.cache.insert(key, state);
        }
    }

    pub fn get(&self, key: PageKey) -> &PageState {
        self.cache.get(&key).unwrap_or(&PageState::Loading)
    }

    pub fn ready(&self, key: PageKey) -> Option<Arc<Page>> {
        match self.cache.get(&key) {
            Some(PageState::Ready(p)) => Some(p.clone()),
            _ => None,
        }
    }

    /// Try a failed page again.
    pub fn retry(&mut self, key: PageKey) {
        self.cache.remove(&key);
        let mut w = self.shared.wanted.lock().unwrap_or_else(|e| e.into_inner());
        w.held.remove(&key);
        drop(w);
        self.shared.wake.notify_all();
    }

    pub fn clear(&mut self) {
        self.want(Vec::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        bytes
    }

    #[test]
    fn tall_pages_are_cut_into_strips() {
        let d = decode(&png(100, 2500), 1024).unwrap();
        assert_eq!((d.width, d.height), (100, 2500));
        assert_eq!(d.strips.iter().map(|s| s.size[1]).collect::<Vec<_>>(), vec![1024, 1024, 452]);
        assert_eq!(d.strips[2].pixels[0], egui::Color32::from_rgb(10, 20, 30));
    }

    #[test]
    fn wide_pages_are_scaled_down() {
        let d = decode(&png(4000, 2000), 2048).unwrap();
        assert_eq!((d.width, d.height), (2048, 1024));
        assert!(decode(b"junk", 2048).is_err());
    }

    #[test]
    fn loads_in_the_wanted_order() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut loader = PageLoader::new(&ctx, 2048);
        let mut list = Vec::new();
        for i in 0..3 {
            let p = dir.path().join(format!("{i}.png"));
            std::fs::write(&p, png(10, 10 + i as u32)).unwrap();
            list.push((PageKey { chapter: 1, index: i }, PageRef::File(p)));
        }
        list.push((PageKey { chapter: 1, index: 9 }, PageRef::File(dir.path().join("missing.png"))));
        loader.want(list);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            loader.poll();
            if loader.cache.len() == 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(loader.ready(PageKey { chapter: 1, index: 2 }).unwrap().size, Vec2::new(10.0, 12.0));
        assert!(matches!(loader.get(PageKey { chapter: 1, index: 9 }), PageState::Failed(_)));
        assert!(matches!(loader.get(PageKey { chapter: 2, index: 0 }), PageState::Loading));
        // Pages no longer wanted are dropped.
        loader.want(vec![]);
        assert!(loader.ready(PageKey { chapter: 1, index: 2 }).is_none());
    }
}
