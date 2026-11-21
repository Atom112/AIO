//! Installed-font discovery for the app font setting.
//!
//! The frontend renders its dropdown from this list, so the command only needs the
//! *family* names a CSS `font-family` can match. Names are read from the font files
//! themselves (`name` table, typographic family first, then legacy family, then full
//! name), which keeps the implementation identical on every platform and avoids
//! pulling in a per-OS font backend.
//!
//! Scanning is read-only and best-effort: unreadable directories, corrupt files and
//! unsupported formats are skipped, so one broken font never empties the list.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ttf_parser::name_id::{FAMILY, FULL_NAME, TYPOGRAPHIC_FAMILY};
use ttf_parser::PlatformId;

/// Font file extensions we can read family names from.
const FONT_EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];
/// Font directories nest a few levels deep at most; the cap also guards symlink loops.
const MAX_SCAN_DEPTH: usize = 8;
/// Skip pathological files (a full CJK collection is tens of MB, never > 100 MB).
const MAX_FONT_FILE_BYTES: u64 = 100 * 1024 * 1024;

/// Platform font directories, most-specific first.
fn font_directories() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut dirs = Vec::new();
        let windows_dir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        dirs.push(windows_dir.join("Fonts"));
        // Per-user installs (dragging a font into Settings) land here.
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
        dirs
    }
    #[cfg(target_os = "macos")]
    {
        let mut dirs = vec![
            PathBuf::from("/System/Library/Fonts"),
            PathBuf::from("/Library/Fonts"),
        ];
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join("Library").join("Fonts"));
        }
        dirs
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut dirs = vec![
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/usr/local/share/fonts"),
        ];
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join(".local").join("share").join("fonts"));
            dirs.push(home.join(".fonts"));
        }
        dirs
    }
}

/// Whether the path has a font extension we can parse.
fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| FONT_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Recursively collect font files under `root` (directories are read best-effort).
fn collect_font_files(root: &Path, files: &mut Vec<PathBuf>) {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                if depth < MAX_SCAN_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if file_type.is_file() && is_font_file(&path) {
                files.push(path);
            }
        }
    }
}

/// Pick the best family name from one face.
///
/// Preference: English typographic family → English legacy family → any-language
/// typographic family → any-language family → English full name. English records keep
/// the list stable across locales; a font without any (e.g. a Chinese-only family)
/// falls back to its localized name, which the WebView still matches.
fn family_name(face: &ttf_parser::Face) -> Option<String> {
    let mut english_typographic: Option<String> = None;
    let mut english_family: Option<String> = None;
    let mut any_typographic: Option<String> = None;
    let mut any_family: Option<String> = None;
    let mut english_full: Option<String> = None;

    for name in face.names() {
        if !matches!(name.name_id, FAMILY | TYPOGRAPHIC_FAMILY | FULL_NAME) {
            continue;
        }
        let Some(value) = name.to_string() else {
            continue;
        };
        let value = value.trim().to_string();
        // Private/system fonts (".SF NS" etc.) are not user-selectable families.
        if value.is_empty() || value.starts_with('.') {
            continue;
        }
        let is_english = name.platform_id == PlatformId::Windows
            && (name.language_id == 0x0409 || name.language_id == 0);
        match name.name_id {
            TYPOGRAPHIC_FAMILY if is_english => {
                english_typographic.get_or_insert(value);
            }
            FAMILY if is_english => {
                english_family.get_or_insert(value);
            }
            TYPOGRAPHIC_FAMILY => {
                any_typographic.get_or_insert(value);
            }
            FAMILY => {
                any_family.get_or_insert(value);
            }
            FULL_NAME if is_english => {
                english_full.get_or_insert(value);
            }
            _ => {}
        }
    }

    english_typographic
        .or(english_family)
        .or(any_typographic)
        .or(any_family)
        .or(english_full)
}

/// Read every family name contained in a font file (collections yield one per face).
fn families_in_font_file(path: &Path) -> Vec<String> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if metadata.len() == 0 || metadata.len() > MAX_FONT_FILE_BYTES {
        return Vec::new();
    }
    let Ok(data) = std::fs::read(path) else {
        return Vec::new();
    };
    let face_count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
    let mut families = Vec::new();
    for index in 0..face_count {
        let Ok(face) = ttf_parser::Face::parse(&data, index) else {
            continue;
        };
        if let Some(family) = family_name(&face) {
            families.push(family);
        }
    }
    families
}

/// Trim, deduplicate (case-insensitively) and sort family names.
fn normalize_families(families: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique: Vec<String> = Vec::new();
    for family in families {
        let family = family.trim().to_string();
        if family.is_empty() {
            continue;
        }
        if seen.insert(family.to_lowercase()) {
            unique.push(family);
        }
    }
    unique.sort_by_key(|family| family.to_lowercase());
    unique
}

/// Scan the platform font directories and return the installed family names.
fn collect_installed_fonts() -> Vec<String> {
    let mut files = Vec::new();
    for directory in font_directories() {
        collect_font_files(&directory, &mut files);
    }
    let mut families = Vec::new();
    for file in files {
        families.extend(families_in_font_file(&file));
    }
    normalize_families(families)
}

/// List font families installed for the current user (sorted, deduplicated).
///
/// # Returns
/// Family names usable as a CSS `font-family` value. An empty list means no readable
/// system font directory was found (the frontend then keeps the system default).
#[tauri::command]
pub async fn list_installed_fonts() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(collect_installed_fonts)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aio-fonts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn detection_accepts_font_extensions_case_insensitively() {
        assert!(is_font_file(Path::new("a.ttf")));
        assert!(is_font_file(Path::new("a.OTF")));
        assert!(is_font_file(Path::new("a.ttc")));
        assert!(is_font_file(Path::new("a.otc")));
        assert!(!is_font_file(Path::new("a.txt")));
        assert!(!is_font_file(Path::new("a")));
    }

    #[test]
    fn scan_recurses_and_ignores_non_font_files() {
        let dir = temp_dir();
        let nested = dir.join("family").join("sub");
        std::fs::create_dir_all(&nested).expect("nested dir");
        std::fs::write(dir.join("one.ttf"), b"not-a-real-font").expect("write ttf");
        std::fs::write(nested.join("two.otf"), b"not-a-real-font").expect("write otf");
        std::fs::write(nested.join("readme.txt"), b"text").expect("write txt");

        let mut files = Vec::new();
        collect_font_files(&dir, &mut files);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"one.ttf".to_string()));
        assert!(names.contains(&"two.otf".to_string()));
        assert!(!names.contains(&"readme.txt".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_font_file_yields_no_families() {
        let dir = temp_dir();
        let file = dir.join("broken.ttf");
        std::fs::write(&file, b"definitely not a font").expect("write");
        assert!(families_in_font_file(&file).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn families_are_trimmed_deduplicated_and_sorted() {
        let families = normalize_families(vec![
            " Times New Roman ".to_string(),
            "Arial".to_string(),
            "arial".to_string(),
            String::new(),
            "Microsoft YaHei".to_string(),
        ]);
        assert_eq!(families, ["Arial", "Microsoft YaHei", "Times New Roman"]);
    }
}
