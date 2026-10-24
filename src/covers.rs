//! Cover art: fetched on worker threads, cached on disk
//! (`~/.cache/m3-comics/covers`) and as textures in an LRU. Covers can be
//! web URLs, `file://` paths or `archive://` pages of a comic file.

use egui::{epaint::RectShape, pos2, Color32, ColorImage, CornerRadius, Rect, TextureHandle, TextureOptions, Ui};
use lru::LruCache;
use material_3_ui::icons::MaterialIcon;
use material_3_ui::widgets;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::PathBuf;

const MAX_SIDE: u32 = 420;
const WORKERS: usize = 4;

enum Slot {
    Loading,
    Ready(TextureHandle),
    Failed,
}

type Request = (String, Vec<(String, String)>);

pub struct Covers {
    ctx: egui::Context,
    cache: LruCache<String, Slot>,
    requests: flume::Sender<Request>,
    results: flume::Receiver<(String, Option<ColorImage>)>,
}

fn cache_dir() -> PathBuf {
    crate::config::cache_dir().join("covers")
}

fn hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

pub fn decode(bytes: &[u8], max_side: u32) -> Option<ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > max_side || img.height() > max_side { img.thumbnail(max_side, max_side) } else { img };
    let rgba = img.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

fn load(agent: &ureq::Agent, url: &str, headers: &[(String, String)]) -> Option<ColorImage> {
    if let Some(path) = url.strip_prefix("file://") {
        return decode(&std::fs::read(path).ok()?, MAX_SIDE);
    }
    if let Some((path, entry)) = crate::archive::parse_cover_ref(url) {
        return decode(&crate::archive::read(&path, &entry).ok()?, MAX_SIDE);
    }
    let disk = cache_dir().join(format!("{:016x}", hash(url)));
    if let Ok(bytes) = std::fs::read(&disk) {
        if let Some(img) = decode(&bytes, MAX_SIDE) {
            return Some(img);
        }
    }
    let bytes = crate::sources::get_bytes(agent, url, headers).ok()?;
    let img = decode(&bytes, MAX_SIDE)?;
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(&disk, &bytes);
    Some(img)
}

impl Covers {
    pub fn new(ctx: egui::Context) -> Self {
        let (requests, req_rx) = flume::unbounded::<Request>();
        let (res_tx, results) = flume::unbounded();
        let agent = crate::sources::agent();
        for i in 0..WORKERS {
            let rx = req_rx.clone();
            let tx = res_tx.clone();
            let ctx = ctx.clone();
            let agent = agent.clone();
            std::thread::Builder::new()
                .name(format!("m3-comics-cover-{i}"))
                .spawn(move || {
                    for (url, headers) in rx.iter() {
                        let img = load(&agent, &url, &headers);
                        let _ = tx.send((url, img));
                        ctx.request_repaint();
                    }
                })
                .expect("spawn a cover thread");
        }
        Covers { ctx, cache: LruCache::new(NonZeroUsize::new(600).expect("non-zero")), requests, results }
    }

    pub fn poll(&mut self) {
        for (url, img) in self.results.try_iter() {
            let slot = match img {
                Some(img) => Slot::Ready(self.ctx.load_texture(&url, img, TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR })),
                None => Slot::Failed,
            };
            self.cache.put(url, slot);
        }
    }

    pub fn get(&mut self, url: &str, headers: &[(String, String)]) -> Option<TextureHandle> {
        if !self.cache.contains(url) {
            self.cache.put(url.to_string(), Slot::Loading);
            let _ = self.requests.send((url.to_string(), headers.to_vec()));
        }
        match self.cache.get(url)? {
            Slot::Ready(t) => Some(t.clone()),
            _ => None,
        }
    }
}

/// UV rect that center-crops an image of `size` to the aspect of `rect`.
pub fn crop_uv(size: [usize; 2], rect: Rect) -> Rect {
    let (w, h) = (size[0] as f32, size[1] as f32);
    if w <= 0.0 || h <= 0.0 {
        return Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    }
    let target = rect.width() / rect.height().max(1.0);
    let actual = w / h;
    if actual > target {
        let keep = target / actual;
        let off = (1.0 - keep) / 2.0;
        Rect::from_min_max(pos2(off, 0.0), pos2(off + keep, 1.0))
    } else {
        let keep = actual / target;
        let off = (1.0 - keep) / 2.0;
        Rect::from_min_max(pos2(0.0, off), pos2(1.0, off + keep))
    }
}

/// Paint a cover filling `rect` (center-cropped), or a placeholder.
pub fn paint(ui: &Ui, covers: &mut Covers, url: Option<&str>, headers: &[(String, String)], rect: Rect, radius: u8, fallback: MaterialIcon) {
    let p = widgets::palette(ui);
    let corner = CornerRadius::same(radius);
    match url.and_then(|u| covers.get(u, headers)) {
        Some(tex) => {
            let uv = crop_uv(tex.size(), rect);
            ui.painter().add(RectShape::filled(rect, corner, Color32::WHITE).with_texture(tex.id(), uv));
        }
        None => {
            ui.painter().rect_filled(rect, corner, p.surface_container_highest);
            widgets::paint_icon(ui, fallback, rect.center(), (rect.width().min(rect.height()) * 0.35).clamp(16.0, 64.0), p.on_surface_variant.gamma_multiply(0.6));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_keeps_aspect() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(100.0, 150.0));
        let uv = crop_uv([200, 300], r);
        assert!((uv.width() - 1.0).abs() < 1e-3 && (uv.height() - 1.0).abs() < 1e-3);
        let uv = crop_uv([300, 300], r);
        assert!((uv.width() - 0.6667).abs() < 1e-3 && uv.height() == 1.0);
    }

    #[test]
    fn decodes_and_shrinks() {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(1000, 500, image::Rgba([1, 2, 3, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        let img = decode(&bytes, 420).unwrap();
        assert_eq!(img.size, [420, 210]);
        assert!(decode(b"nope", 420).is_none());
    }
}
