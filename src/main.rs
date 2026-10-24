//! Material 3 Comics: a manga and comics reader in the Material 3 apps'
//! style, after Mihon. A library, sources (your files, MangaDex, and
//! Mihon's extensions through Suwayomi-Server), downloads, updates and
//! AniList tracking.

mod app;
mod archive;
mod config;
mod covers;
mod db;
mod downloads;
mod model;
mod pages;
mod reader;
mod sources;
mod tracking;
mod ui;
mod updates;

use std::path::PathBuf;

const USAGE: &str = "\
Material 3 Comics — a manga and comics reader

Usage: m3-comics [FILE | FOLDER]
       m3-comics update

Opens a comic (CBZ, CBR, CB7, ZIP, EPUB or a folder of pictures) in the
reader, or the library with no argument. `update` checks the library for
new chapters without opening a window and sends a notification (for the
m3-comics-update.timer systemd user timer).

Reader keys: ←/→ turn pages, Space next, [ ] chapters, M menu,
F11 fullscreen, Esc back.

Options:
  -h, --help       show this help
  -V, --version    show the version
";

fn main() -> eframe::Result {
    let mut open = None;
    let mut args = std::env::args_os().skip(1).peekable();
    if args.peek().and_then(|a| a.to_str()) == Some("update") && !std::path::Path::new("update").exists() {
        std::process::exit(update());
    }
    for arg in args {
        match arg.to_str() {
            Some("-h" | "--help") => {
                print!("{USAGE}");
                return Ok(());
            }
            Some("-V" | "--version") => {
                println!("m3-comics {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Some(s) if s.starts_with('-') && s != "-" && open.is_none() && !std::path::Path::new(s).exists() => {
                eprintln!("m3-comics: unknown option {s}\n\n{USAGE}");
                std::process::exit(2);
            }
            _ if open.is_none() && !arg.is_empty() => open = Some(file_arg(arg)),
            _ => {}
        }
    }
    let cfg = config::Config::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Comics")
            .with_app_id("material_3_comics")
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([640.0, 420.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("material_3_comics", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, cfg, open)))))
}

/// `m3-comics update`: check the library, notify about new chapters.
fn update() -> i32 {
    let cfg = config::Config::load();
    let db = match db::Db::open(&config::data_dir().join("library.db")) {
        Ok(db) => std::sync::Arc::new(db),
        Err(e) => {
            eprintln!("m3-comics: can't open the library: {e}");
            return 1;
        }
    };
    let sources = sources::Sources::new(&cfg);
    if sources.server().is_some() {
        if let Err(e) = sources.refresh_remote() {
            eprintln!("m3-comics: Suwayomi: {e}");
        }
    }
    let progress = std::sync::Arc::new(updates::Progress::default());
    let report = updates::update_library(&db, &sources, &progress);
    println!("Checked {} series. {}.", report.checked, report.summary());
    for (title, e) in &report.errors {
        eprintln!("  {title}: {e}");
    }
    if cfg.notify_new_chapters && report.new_chapters() > 0 {
        updates::notify(&report);
    }
    if report.checked > 0 && report.errors.len() == report.checked {
        1
    } else {
        0
    }
}

/// A path or a file:// URL (some apps pass those).
fn file_arg(arg: std::ffi::OsString) -> PathBuf {
    if let Some(s) = arg.to_str() {
        if let Some(rest) = s.strip_prefix("file://") {
            return PathBuf::from(percent_decode(rest));
        }
    }
    PathBuf::from(arg)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls() {
        assert_eq!(file_arg("file:///home/a/My%20Comic.cbz".into()), PathBuf::from("/home/a/My Comic.cbz"));
        assert_eq!(file_arg("/tmp/x.cbz".into()), PathBuf::from("/tmp/x.cbz"));
        assert_eq!(percent_decode("100%"), "100%");
    }
}
