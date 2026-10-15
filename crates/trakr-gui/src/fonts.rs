//! Custom fonts: finding font files on the system and building egui font definitions from them.

use eframe::egui;
use std::path::{Path, PathBuf};

/// A font file found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFile {
    pub name: String,
    pub path: PathBuf,
}

/// Where fonts usually live on Linux.
pub fn default_font_dirs() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        v.push(home.join(".local/share/fonts"));
        v.push(home.join(".fonts"));
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        v.push(PathBuf::from(x).join("fonts"));
    }
    v
}

/// Every `.ttf` / `.otf` under `dirs` (a few levels deep), sorted by name. Collections (`.ttc`) are skipped.
pub fn scan_fonts(dirs: &[PathBuf]) -> Vec<FontFile> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<FontFile>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 5 {
                    walk(&p, depth + 1, out);
                }
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf"))
            {
                if let Some(stem) = p.file_stem() {
                    out.push(FontFile { name: stem.to_string_lossy().into_owned(), path: p });
                }
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        walk(d, 0, &mut out);
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.path.cmp(&b.path)));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

/// Read a font file and check that it parses (egui panics on broken font data).
pub fn load_font(path: &str) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    ab_glyph::FontArc::try_from_vec(bytes.clone())
        .map_err(|_| format!("{path} is not a usable TTF/OTF font"))?;
    Ok(bytes)
}

/// egui font definitions: the built-in fonts, with the chosen files put first so they win and the built-in
/// ones still cover glyphs the custom fonts lack. Returns the definitions and any problems found.
pub fn definitions(ui_font: &str, grid_font: &str) -> (egui::FontDefinitions, Vec<String>) {
    let mut defs = egui::FontDefinitions::default();
    let mut problems = Vec::new();
    for (key, path, family) in [
        ("custom-ui", ui_font, egui::FontFamily::Proportional),
        ("custom-grid", grid_font, egui::FontFamily::Monospace),
    ] {
        if path.trim().is_empty() {
            continue;
        }
        match load_font(path) {
            Ok(bytes) => {
                defs.font_data
                    .insert(key.to_string(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                defs.families.entry(family).or_default().insert(0, key.to_string());
            }
            Err(e) => problems.push(e),
        }
    }
    (defs, problems)
}

/// Is every glyph in "iW0." the same width? (A quick monospace check for the picker.)
pub fn looks_monospaced(path: &str) -> bool {
    use ab_glyph::{Font, ScaleFont};
    let Ok(bytes) = std::fs::read(path) else { return false };
    let Ok(font) = ab_glyph::FontArc::try_from_vec(bytes) else { return false };
    let f = font.as_scaled(20.0);
    let w = |c: char| f.h_advance(f.glyph_id(c));
    (w('i') - w('W')).abs() < 0.01 && (w('0') - w('.')).abs() < 0.01
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";
    const SANS: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

    #[test]
    fn scans_and_validates_fonts() {
        let dir = std::env::temp_dir().join(format!("trakr-fonts-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("broken.ttf"), b"not a font").unwrap();
        std::fs::write(dir.join("sub/readme.txt"), b"x").unwrap();
        if std::path::Path::new(MONO).exists() {
            std::fs::copy(MONO, dir.join("sub/Mono.TTF")).unwrap();
        }
        let found = scan_fonts(&[dir.clone(), PathBuf::from("/nonexistent")]);
        assert!(found.iter().any(|f| f.name == "broken"));
        assert_eq!(found.iter().any(|f| f.name == "Mono"), std::path::Path::new(MONO).exists());
        assert!(load_font(&dir.join("broken.ttf").to_string_lossy()).is_err());
        assert!(load_font("/nonexistent.ttf").is_err());
        let (defs, problems) = definitions(&dir.join("broken.ttf").to_string_lossy(), "");
        assert_eq!(problems.len(), 1, "a broken font is reported, not used");
        assert!(!defs.font_data.contains_key("custom-ui"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_fonts_come_first_and_the_gui_draws_with_them() {
        if !std::path::Path::new(MONO).exists() {
            return; // no fonts on this machine
        }
        let (defs, problems) = definitions(SANS, MONO);
        assert!(problems.is_empty());
        assert_eq!(defs.families[&egui::FontFamily::Proportional][0], "custom-ui");
        assert_eq!(defs.families[&egui::FontFamily::Monospace][0], "custom-grid");
        assert!(defs.families[&egui::FontFamily::Monospace].len() > 1, "built-in fonts remain as fallback");
        assert!(looks_monospaced(MONO) && !looks_monospaced(SANS) && !looks_monospaced("/nonexistent"));
    }
}
