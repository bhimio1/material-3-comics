//! ~/.config/m3-comics/config.toml

use material_3_ui::sys::arch::AurHelper;
use material_3_ui::sys::paths;
use material_3_ui::theme::ThemeSettings;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ReadingMode {
    /// Pages one at a time, turning right to left (manga).
    #[default]
    RightToLeft,
    /// Pages one at a time, left to right (comics).
    LeftToRight,
    /// Pages one at a time, scrolling down.
    Vertical,
    /// One long strip without gaps (webtoons, manhwa).
    Webtoon,
}

impl ReadingMode {
    pub const ALL: [ReadingMode; 4] = [ReadingMode::RightToLeft, ReadingMode::LeftToRight, ReadingMode::Vertical, ReadingMode::Webtoon];

    pub fn label(self) -> &'static str {
        match self {
            ReadingMode::RightToLeft => "Right to left",
            ReadingMode::LeftToRight => "Left to right",
            ReadingMode::Vertical => "Vertical",
            ReadingMode::Webtoon => "Webtoon",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ReadingMode::RightToLeft => "right-to-left",
            ReadingMode::LeftToRight => "left-to-right",
            ReadingMode::Vertical => "vertical",
            ReadingMode::Webtoon => "webtoon",
        }
    }

    pub fn from_key(s: &str) -> Option<ReadingMode> {
        ReadingMode::ALL.into_iter().find(|m| m.key() == s)
    }

    /// Scrolling modes show pages one under another.
    pub fn continuous(self) -> bool {
        matches!(self, ReadingMode::Vertical | ReadingMode::Webtoon)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Fit {
    /// The whole page on screen.
    #[default]
    Screen,
    Width,
    Height,
    /// One image pixel per screen pixel.
    Original,
}

impl Fit {
    pub const ALL: [Fit; 4] = [Fit::Screen, Fit::Width, Fit::Height, Fit::Original];

    pub fn label(self) -> &'static str {
        match self {
            Fit::Screen => "Fit screen",
            Fit::Width => "Fit width",
            Fit::Height => "Fit height",
            Fit::Original => "Original size",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Background {
    #[default]
    Black,
    Gray,
    White,
    /// The app's surface color.
    Theme,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LibrarySort {
    #[default]
    Title,
    LastRead,
    Added,
    Unread,
    LatestChapter,
}

impl LibrarySort {
    pub const ALL: [LibrarySort; 5] = [LibrarySort::Title, LibrarySort::LastRead, LibrarySort::Added, LibrarySort::Unread, LibrarySort::LatestChapter];

    pub fn label(self) -> &'static str {
        match self {
            LibrarySort::Title => "Title",
            LibrarySort::LastRead => "Last read",
            LibrarySort::Added => "Date added",
            LibrarySort::Unread => "Unread chapters",
            LibrarySort::LatestChapter => "Latest chapter",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Reader {
    pub mode: ReadingMode,
    pub fit: Fit,
    pub background: Background,
    /// Width of the strip in the scrolling modes, as a share of the window.
    pub strip_width: f32,
    /// Pages to load ahead.
    pub preload: usize,
    pub show_page_number: bool,
    /// Clicking the left or right third of the page turns it.
    pub tap_zones: bool,
    /// Go on to the next chapter at the end of one.
    pub continue_to_next: bool,
}

impl Default for Reader {
    fn default() -> Self {
        Reader {
            mode: ReadingMode::RightToLeft,
            fit: Fit::Screen,
            background: Background::Black,
            strip_width: 0.6,
            preload: 4,
            show_page_number: true,
            tap_zones: true,
            continue_to_next: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub appearance: ThemeSettings,
    pub reader: Reader,
    /// Folders the Local files source reads (each subfolder is a series).
    pub library_folders: Vec<String>,
    /// Where downloaded chapters go; empty is ~/.local/share/m3-comics/downloads.
    pub downloads_folder: String,
    pub library_sort: LibrarySort,
    pub library_sort_descending: bool,
    /// Width of a cover in the library grid.
    pub cover_width: f32,
    pub unread_badges: bool,
    pub download_badges: bool,
    /// Check the library for new chapters on start when the last check is
    /// older than this many hours (0: only by hand).
    pub update_interval_hours: u32,
    pub notify_new_chapters: bool,
    pub download_new_chapters: bool,
    pub delete_after_reading: bool,
    /// Comma-separated MangaDex languages (en, ja, es-la, pt-br, …).
    pub mangadex_languages: String,
    pub mangadex_data_saver: bool,
    pub show_nsfw: bool,
    /// The Suwayomi server that runs Mihon's extensions, e.g.
    /// http://127.0.0.1:4567. Empty: none.
    pub suwayomi_url: String,
    pub suwayomi_user: String,
    pub suwayomi_password: String,
    /// Sources pinned to the top of Browse.
    pub pinned_sources: Vec<String>,
    /// Languages of Suwayomi sources to show (empty: all).
    pub source_languages: Vec<String>,
    /// The AniList API client to sign in with (anilist.co/settings/developer).
    pub anilist_client_id: String,
    /// Update tracked progress after finishing a chapter.
    pub auto_track: bool,
    /// Terminal for installing missing tools; empty picks one.
    pub terminal: String,
    pub aur_helper: AurHelper,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            appearance: ThemeSettings::default(),
            reader: Reader::default(),
            library_folders: vec!["~/Comics".into()],
            downloads_folder: String::new(),
            library_sort: LibrarySort::Title,
            library_sort_descending: false,
            cover_width: 150.0,
            unread_badges: true,
            download_badges: true,
            update_interval_hours: 12,
            notify_new_chapters: true,
            download_new_chapters: false,
            delete_after_reading: false,
            mangadex_languages: "en".into(),
            mangadex_data_saver: false,
            show_nsfw: false,
            suwayomi_url: String::new(),
            suwayomi_user: String::new(),
            suwayomi_password: String::new(),
            pinned_sources: Vec::new(),
            source_languages: Vec::new(),
            anilist_client_id: String::new(),
            auto_track: true,
            terminal: String::new(),
            aur_helper: AurHelper::Auto,
        }
    }
}

/// `~/.local/share/m3-comics`.
pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| paths::home().join(".local/share")).join("m3-comics")
}

/// `~/.cache/m3-comics`.
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| paths::home().join(".cache")).join("m3-comics")
}

impl Config {
    pub fn path() -> PathBuf {
        paths::config_home().join("m3-comics/config.toml")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        // It can hold the Suwayomi password.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn local_folders(&self) -> Vec<PathBuf> {
        self.library_folders.iter().map(|f| f.trim()).filter(|f| !f.is_empty()).map(paths::expand).collect()
    }

    pub fn downloads_dir(&self) -> PathBuf {
        match self.downloads_folder.trim() {
            "" => data_dir().join("downloads"),
            f => paths::expand(f),
        }
    }

    pub fn mangadex_languages(&self) -> Vec<String> {
        let v: Vec<String> = self.mangadex_languages.split([',', ' ']).map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect();
        if v.is_empty() {
            vec!["en".into()]
        } else {
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        let mut cfg = Config { mangadex_languages: "en, es-la".into(), suwayomi_url: "http://nas:4567".into(), ..Default::default() };
        cfg.reader.mode = ReadingMode::Webtoon;
        cfg.reader.fit = Fit::Width;
        cfg.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path), cfg);
        assert_eq!(cfg.mangadex_languages(), vec!["en", "es-la"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn missing_and_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(Config::load_from(&path), Config::default());
        std::fs::write(&path, "show_nsfw = true\n[reader]\nmode = \"left-to-right\"\n").unwrap();
        let cfg = Config::load_from(&path);
        assert!(cfg.show_nsfw);
        assert_eq!(cfg.reader.mode, ReadingMode::LeftToRight);
        assert_eq!(cfg.reader.preload, 4);
        assert_eq!(cfg.library_folders, vec!["~/Comics"]);
        assert_eq!(Config { mangadex_languages: " ".into(), ..Default::default() }.mangadex_languages(), vec!["en"]);
    }

    #[test]
    fn mode_keys() {
        for m in ReadingMode::ALL {
            assert_eq!(ReadingMode::from_key(m.key()), Some(m));
        }
        assert!(ReadingMode::Webtoon.continuous());
        assert!(!ReadingMode::RightToLeft.continuous());
    }
}
