//! Comic book files: CBZ/ZIP/EPUB through the zip crate; CBR/RAR, CB7/7Z
//! and CBT/TAR through `bsdtar` (libarchive); and folders of images.
//! Pages are the image entries in natural order.

use crate::model::natural_cmp;
use anyhow::{anyhow, bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif", "jxl"];
const ZIP_EXTS: &[&str] = &["cbz", "zip", "epub"];
const BSDTAR_EXTS: &[&str] = &["cbr", "rar", "cb7", "7z", "cbt", "tar"];
/// Biggest page we read into memory.
const MAX_ENTRY: u64 = 64 << 20;

fn ext_of(name: &str) -> String {
    Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

pub fn is_image_name(name: &str) -> bool {
    IMAGE_EXTS.contains(&ext_of(name).as_str())
}

/// A file this module can read pages from.
pub fn is_comic_file(path: &Path) -> bool {
    let ext = ext_of(&path.to_string_lossy());
    ZIP_EXTS.contains(&ext.as_str()) || BSDTAR_EXTS.contains(&ext.as_str())
}

fn is_zip(path: &Path) -> bool {
    ZIP_EXTS.contains(&ext_of(&path.to_string_lossy()).as_str())
}

/// Skip macOS resource forks and hidden files.
fn wanted(name: &str) -> bool {
    !name.split('/').any(|part| part.starts_with('.') || part == "__MACOSX") && is_image_name(name)
}

fn sort_entries(entries: &mut [String]) {
    entries.sort_by(|a, b| natural_cmp(a, b));
}

/// The image entries of an archive, in reading order.
pub fn list(path: &Path) -> Result<Vec<String>> {
    let mut entries = if is_zip(path) {
        let file = std::fs::File::open(path).with_context(|| format!("Can't open {}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let mut names = Vec::new();
        for i in 0..zip.len() {
            if let Ok(f) = zip.by_index_raw(i) {
                if f.is_file() {
                    names.push(f.name().to_string());
                }
            }
        }
        names
    } else {
        bsdtar_list(path)?
    };
    entries.retain(|n| wanted(n));
    sort_entries(&mut entries);
    Ok(entries)
}

/// The bytes of one entry.
pub fn read(path: &Path, entry: &str) -> Result<Vec<u8>> {
    if is_zip(path) {
        let file = std::fs::File::open(path).with_context(|| format!("Can't open {}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let f = zip.by_name(entry).map_err(|e| anyhow!("{entry}: {e}"))?;
        let mut out = Vec::with_capacity(f.size().min(MAX_ENTRY) as usize);
        f.take(MAX_ENTRY).read_to_end(&mut out)?;
        Ok(out)
    } else {
        bsdtar_read(path, entry)
    }
}

/// A small text entry such as `ComicInfo.xml`, matched case-insensitively
/// at any depth.
pub fn read_text(path: &Path, file_name: &str) -> Option<String> {
    if is_zip(path) {
        let file = std::fs::File::open(path).ok()?;
        let mut zip = zip::ZipArchive::new(file).ok()?;
        let name = (0..zip.len())
            .filter_map(|i| zip.by_index_raw(i).ok().map(|f| f.name().to_string()))
            .find(|n| n.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case(file_name)))?;
        let mut s = String::new();
        zip.by_name(&name).ok()?.take(1 << 20).read_to_string(&mut s).ok()?;
        Some(s)
    } else {
        let name = bsdtar_entries(path).ok()?.into_iter().find(|n| n.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case(file_name)))?;
        String::from_utf8(bsdtar_read(path, &name).ok()?).ok()
    }
}

fn bsdtar() -> Result<&'static str> {
    if material_3_ui::sys::cmd::exists("bsdtar") {
        Ok("bsdtar")
    } else {
        bail!("Reading CBR and CB7 files needs bsdtar (the libarchive package)")
    }
}

fn bsdtar_entries(path: &Path) -> Result<Vec<String>> {
    let out = std::process::Command::new(bsdtar()?)
        .arg("-tf")
        .arg(path)
        .output()
        .context("Couldn't run bsdtar")?;
    if !out.status.success() {
        bail!("{}: {}", path.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.ends_with('/')).map(str::to_string).collect())
}

fn bsdtar_list(path: &Path) -> Result<Vec<String>> {
    bsdtar_entries(path)
}

fn bsdtar_read(path: &Path, entry: &str) -> Result<Vec<u8>> {
    // bsdtar treats member names as patterns; escape the pattern characters.
    let pattern: String = entry.chars().flat_map(|c| if matches!(c, '*' | '?' | '[' | ']' | '\\') { vec!['\\', c] } else { vec![c] }).collect();
    let out = std::process::Command::new(bsdtar()?)
        .arg("-xOf")
        .arg(path)
        .arg(&pattern)
        .output()
        .context("Couldn't run bsdtar")?;
    if !out.status.success() || out.stdout.is_empty() {
        bail!("{entry}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// The images of a folder (not recursive), in reading order.
pub fn list_folder(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.starts_with('.') && is_image_name(n)))
        .collect();
    files.sort_by(|a, b| natural_cmp(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
    files
}

/// A cover reference for the first page of an archive:
/// `archive:///path/to/file.cbz#entry`.
pub fn cover_ref(path: &Path, entry: &str) -> String {
    format!("archive://{}#{}", path.display(), entry)
}

/// Split a reference made by [`cover_ref`].
pub fn parse_cover_ref(s: &str) -> Option<(PathBuf, String)> {
    let rest = s.strip_prefix("archive://")?;
    let (path, entry) = rest.rsplit_once('#')?;
    Some((PathBuf::from(path), entry.to_string()))
}

/// Write pages into a new CBZ (stored, since the images are already
/// compressed). Written to `<dest>.part`, then renamed.
pub fn write_cbz(dest: &Path, pages: &[(String, Vec<u8>)], comic_info: Option<&str>) -> Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = dest.with_extension("cbz.part");
    {
        let file = std::fs::File::create(&part)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in pages {
            zip.start_file(name.as_str(), opts)?;
            std::io::Write::write_all(&mut zip, bytes)?;
        }
        if let Some(xml) = comic_info {
            zip.start_file("ComicInfo.xml", opts.compression_method(zip::CompressionMethod::Deflated))?;
            std::io::Write::write_all(&mut zip, xml.as_bytes())?;
        }
        zip.finish()?;
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

/// The image type of some bytes, for naming downloaded pages.
pub fn image_ext(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xFF, 0xD8, ..] => "jpg",
        [b'G', b'I', b'F', ..] => "gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        [b'B', b'M', ..] => "bmp",
        [_, _, _, _, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f', ..] => "avif",
        [0xFF, 0x0A, ..] | [0, 0, 0, 0x0C, b'J', b'X', b'L', ..] => "jxl",
        _ => "jpg",
    }
}

/// The metadata Mihon and Komga read from `ComicInfo.xml`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComicInfo {
    pub series: String,
    pub title: String,
    pub number: String,
    pub volume: String,
    pub summary: String,
    pub writer: String,
    pub penciller: String,
    pub genre: Vec<String>,
    pub web: String,
    pub translator: String,
}

impl ComicInfo {
    pub fn parse(xml: &str) -> Option<ComicInfo> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let root = doc.root_element();
        let get = |tag: &str| {
            root.children()
                .find(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(tag))
                .and_then(|n| n.text())
                .map(|t| t.trim().to_string())
                .unwrap_or_default()
        };
        let split = |s: String| s.split(',').map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect::<Vec<_>>();
        Some(ComicInfo {
            series: get("Series"),
            title: get("Title"),
            number: get("Number"),
            volume: get("Volume"),
            summary: get("Summary"),
            writer: get("Writer"),
            penciller: get("Penciller"),
            genre: split(get("Genre")),
            web: get("Web"),
            translator: get("Translator"),
        })
    }

    pub fn to_xml(&self) -> String {
        fn esc(s: &str) -> String {
            s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        }
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ComicInfo xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n");
        let fields = [
            ("Title", self.title.clone()),
            ("Series", self.series.clone()),
            ("Number", self.number.clone()),
            ("Volume", self.volume.clone()),
            ("Summary", self.summary.clone()),
            ("Writer", self.writer.clone()),
            ("Penciller", self.penciller.clone()),
            ("Translator", self.translator.clone()),
            ("Genre", self.genre.join(", ")),
            ("Web", self.web.clone()),
        ];
        for (tag, value) in fields {
            if !value.is_empty() {
                out.push_str(&format!("  <{tag}>{}</{tag}>\n", esc(&value)));
            }
        }
        out.push_str("</ComicInfo>\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(2, 3, image::Rgba([9, 9, 9, 255]))
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn cbz_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/Ch 1.cbz");
        let info = ComicInfo { series: "Test & Co".into(), number: "1".into(), genre: vec!["Action".into(), "Drama".into()], ..Default::default() };
        let pages = vec![("010.png".to_string(), png()), ("002.png".to_string(), png()), ("001.png".to_string(), png())];
        write_cbz(&path, &pages, Some(&info.to_xml())).unwrap();
        assert!(!path.with_extension("cbz.part").exists());
        assert_eq!(list(&path).unwrap(), vec!["001.png", "002.png", "010.png"]);
        assert_eq!(read(&path, "002.png").unwrap(), png());
        let back = ComicInfo::parse(&read_text(&path, "comicinfo.xml").unwrap()).unwrap();
        assert_eq!(back, info);
        assert!(read(&path, "missing.png").is_err());
    }

    #[test]
    fn hidden_and_non_image_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.zip");
        let pages = vec![
            ("__MACOSX/._1.jpg".to_string(), vec![1]),
            ("pages/.hidden.jpg".to_string(), vec![1]),
            ("pages/notes.txt".to_string(), vec![1]),
            ("pages/1.JPG".to_string(), vec![1]),
        ];
        write_cbz(&path, &pages, None).unwrap();
        assert_eq!(list(&path).unwrap(), vec!["pages/1.JPG"]);
    }

    #[test]
    fn cover_refs() {
        let r = cover_ref(Path::new("/a/b#c.cbz"), "x/01.jpg");
        assert_eq!(parse_cover_ref(&r), Some((PathBuf::from("/a/b#c.cbz"), "x/01.jpg".to_string())));
        assert_eq!(parse_cover_ref("https://x"), None);
    }

    #[test]
    fn sniffing() {
        assert_eq!(image_ext(&png()), "png");
        assert_eq!(image_ext(&[0xFF, 0xD8, 0xFF]), "jpg");
        assert_eq!(image_ext(b"RIFF\0\0\0\0WEBPVP8"), "webp");
        assert!(is_comic_file(Path::new("/x/A.CBR")));
        assert!(!is_comic_file(Path::new("/x/a.pdf")));
    }

    #[test]
    fn folders() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["10.png", "2.png", ".x.png", "a.txt"] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        let names: Vec<_> = list_folder(dir.path()).iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, vec!["2.png", "10.png"]);
    }
}
