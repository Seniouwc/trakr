//! Colour themes for the pattern grid and the surrounding egui visuals.

use eframe::egui::{self, Color32};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    /// Band behind the cursor row.
    pub band: Color32,
    pub beat_row: Color32,
    pub play_row: Color32,
    pub play_row_dim: Color32,
    pub divider: Color32,
    pub column_sep: Color32,
    pub text_bar: Color32,
    pub text_beat: Color32,
    pub text: Color32,
    /// Lines of neighbouring patterns.
    pub text_other: Color32,
    pub text_muted: Color32,
    pub selection: Color32,
    pub cursor_edit: Color32,
    pub cursor_view: Color32,
    pub header_text: Color32,
    pub header_text_muted: Color32,
    /// Added to a track colour for the selected track's header.
    pub header_lift: i32,
    pub header_base_div: u8,
    pub header_default: [u8; 3],
}

/// Built-in themes that are not colour schemes below.
pub const THEMES: [(&str, &str); 3] = [("dark", "Dark"), ("light", "Light"), ("contrast", "High contrast")];

/// A colour scheme: a handful of named colours that [`Palette`] and the egui visuals are derived from.
#[derive(Clone, Copy, Debug)]
pub struct Scheme {
    pub dark: bool,
    /// Panels and menus (a step away from `base`).
    pub mantle: Color32,
    /// Pattern grid and window background.
    pub base: Color32,
    pub surface: Color32,
    pub surface2: Color32,
    pub muted: Color32,
    pub subtle: Color32,
    pub text: Color32,
    pub red: Color32,
    pub green: Color32,
    pub blue: Color32,
    pub yellow: Color32,
    pub cyan: Color32,
    pub purple: Color32,
}

const fn c(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Order: mantle, base, surface, surface2, muted, subtle, text, red, green, blue, yellow, cyan, purple.
const fn scheme(dark: bool, v: [u32; 13]) -> Scheme {
    Scheme {
        dark,
        mantle: c(v[0]),
        base: c(v[1]),
        surface: c(v[2]),
        surface2: c(v[3]),
        muted: c(v[4]),
        subtle: c(v[5]),
        text: c(v[6]),
        red: c(v[7]),
        green: c(v[8]),
        blue: c(v[9]),
        yellow: c(v[10]),
        cyan: c(v[11]),
        purple: c(v[12]),
    }
}

/// Popular editor / terminal colour schemes, from their published palettes: (id, label, scheme).
pub const SCHEMES: &[(&str, &str, Scheme)] = &[
    (
        "catppuccin-latte",
        "Catppuccin Latte",
        scheme(
            false,
            [
                0xe6e9ef, 0xeff1f5, 0xccd0da, 0xbcc0cc, 0x9ca0b0, 0x6c6f85, 0x4c4f69, 0xd20f39, 0x40a02b,
                0x1e66f5, 0xdf8e1d, 0x04a5e5, 0x8839ef,
            ],
        ),
    ),
    (
        "catppuccin-frappe",
        "Catppuccin Frappe",
        scheme(
            true,
            [
                0x292c3c, 0x303446, 0x414559, 0x51576d, 0x737994, 0xa5adce, 0xc6d0f5, 0xe78284, 0xa6d189,
                0x8caaee, 0xe5c890, 0x99d1db, 0xca9ee6,
            ],
        ),
    ),
    (
        "catppuccin-macchiato",
        "Catppuccin Macchiato",
        scheme(
            true,
            [
                0x1e2030, 0x24273a, 0x363a4f, 0x494d64, 0x6e738d, 0xa5adcb, 0xcad3f5, 0xed8796, 0xa6da95,
                0x8aadf4, 0xeed49f, 0x91d7e3, 0xc6a0f6,
            ],
        ),
    ),
    (
        "catppuccin-mocha",
        "Catppuccin Mocha",
        scheme(
            true,
            [
                0x181825, 0x1e1e2e, 0x313244, 0x45475a, 0x6c7086, 0xa6adc8, 0xcdd6f4, 0xf38ba8, 0xa6e3a1,
                0x89b4fa, 0xf9e2af, 0x89dceb, 0xcba6f7,
            ],
        ),
    ),
    (
        "dracula",
        "Dracula",
        scheme(
            true,
            [
                0x21222c, 0x282a36, 0x3a3c4e, 0x44475a, 0x6272a4, 0xb4b8d4, 0xf8f8f2, 0xff5555, 0x50fa7b,
                0xbd93f9, 0xf1fa8c, 0x8be9fd, 0xff79c6,
            ],
        ),
    ),
    (
        "gruvbox-dark",
        "Gruvbox Dark",
        scheme(
            true,
            [
                0x1d2021, 0x282828, 0x3c3836, 0x504945, 0x7c6f64, 0xa89984, 0xebdbb2, 0xfb4934, 0xb8bb26,
                0x83a598, 0xfabd2f, 0x8ec07c, 0xd3869b,
            ],
        ),
    ),
    (
        "gruvbox-light",
        "Gruvbox Light",
        scheme(
            false,
            [
                0xf2e5bc, 0xfbf1c7, 0xebdbb2, 0xd5c4a1, 0xa89984, 0x7c6f64, 0x3c3836, 0x9d0006, 0x79740e,
                0x076678, 0xb57614, 0x427b58, 0x8f3f71,
            ],
        ),
    ),
    (
        "nord",
        "Nord",
        scheme(
            true,
            [
                0x242933, 0x2e3440, 0x3b4252, 0x434c5e, 0x4c566a, 0x8f9bb3, 0xe5e9f0, 0xbf616a, 0xa3be8c,
                0x81a1c1, 0xebcb8b, 0x88c0d0, 0xb48ead,
            ],
        ),
    ),
    (
        "solarized-dark",
        "Solarized Dark",
        scheme(
            true,
            [
                0x00212b, 0x002b36, 0x073642, 0x124b58, 0x586e75, 0x839496, 0x93a1a1, 0xdc322f, 0x859900,
                0x268bd2, 0xb58900, 0x2aa198, 0x6c71c4,
            ],
        ),
    ),
    (
        "solarized-light",
        "Solarized Light",
        scheme(
            false,
            [
                0xf5eed9, 0xfdf6e3, 0xe6dfc8, 0xd9d2bb, 0x93a1a1, 0x657b83, 0x586e75, 0xdc322f, 0x859900,
                0x268bd2, 0xb58900, 0x2aa198, 0x6c71c4,
            ],
        ),
    ),
    (
        "tokyo-night",
        "Tokyo Night",
        scheme(
            true,
            [
                0x16161e, 0x1a1b26, 0x292e42, 0x343a55, 0x565f89, 0x9aa5ce, 0xc0caf5, 0xf7768e, 0x9ece6a,
                0x7aa2f7, 0xe0af68, 0x7dcfff, 0xbb9af7,
            ],
        ),
    ),
    (
        "one-dark",
        "One Dark",
        scheme(
            true,
            [
                0x21252b, 0x282c34, 0x2c313c, 0x3e4451, 0x5c6370, 0x828997, 0xabb2bf, 0xe06c75, 0x98c379,
                0x61afef, 0xe5c07b, 0x56b6c2, 0xc678dd,
            ],
        ),
    ),
    (
        "rose-pine",
        "Rose Pine",
        scheme(
            true,
            [
                0x14121d, 0x191724, 0x1f1d2e, 0x26233a, 0x6e6a86, 0x908caa, 0xe0def4, 0xeb6f92, 0x7cc7a0,
                0x3e8fb0, 0xf6c177, 0x9ccfd8, 0xc4a7e7,
            ],
        ),
    ),
    (
        "everforest-dark",
        "Everforest Dark",
        scheme(
            true,
            [
                0x232a2e, 0x2d353b, 0x343f44, 0x3d484d, 0x7a8478, 0x9da9a0, 0xd3c6aa, 0xe67e80, 0xa7c080,
                0x7fbbb3, 0xdbbc7f, 0x83c092, 0xd699b6,
            ],
        ),
    ),
    (
        "monokai",
        "Monokai",
        scheme(
            true,
            [
                0x1e1f1c, 0x272822, 0x3e3d32, 0x49483e, 0x75715e, 0xa59f85, 0xf8f8f2, 0xf92672, 0xa6e22e,
                0x66d9ef, 0xe6db74, 0xa1efe4, 0xae81ff,
            ],
        ),
    ),
];

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

impl Scheme {
    /// Parse `#rrggbb` (or `rrggbb`).
    pub fn parse_hex(text: &str) -> Option<Color32> {
        let t = text.trim().trim_start_matches('#');
        if t.len() != 6 {
            return None;
        }
        u32::from_str_radix(t, 16).ok().map(c)
    }

    /// A scheme from a user theme file's fields. Only `base` and `text` are required; the other colours
    /// are derived from them or taken from sensible defaults.
    pub fn from_fields(dark: bool, get: impl Fn(&str) -> Option<Color32>) -> Option<Scheme> {
        let (base, text) = (get("base")?, get("text")?);
        let edge = if dark { Color32::BLACK } else { Color32::WHITE };
        let d = |key: &str, fallback: Color32| get(key).unwrap_or(fallback);
        Some(Scheme {
            dark,
            base,
            text,
            mantle: d("mantle", mix(base, edge, 0.25)),
            surface: d("surface", mix(base, text, 0.08)),
            surface2: d("surface2", mix(base, text, 0.16)),
            muted: d("muted", mix(base, text, 0.4)),
            subtle: d("subtle", mix(base, text, 0.7)),
            red: d("red", c(0xe06c75)),
            green: d("green", c(0x98c379)),
            blue: d("blue", c(0x61afef)),
            yellow: d("yellow", c(0xe5c07b)),
            cyan: d("cyan", c(0x56b6c2)),
            purple: d("purple", c(0xc678dd)),
        })
    }

    /// The scheme as the JSON text of a theme file (a starting point for your own).
    pub fn to_json(self, name: &str) -> String {
        let h = |c: Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
        format!(
            "{{\n  \"name\": \"{name}\",\n  \"dark\": {},\n  \"base\": \"{}\",\n  \"mantle\": \"{}\",\n  \"surface\": \"{}\",\n  \"surface2\": \"{}\",\n  \"muted\": \"{}\",\n  \"subtle\": \"{}\",\n  \"text\": \"{}\",\n  \"red\": \"{}\",\n  \"green\": \"{}\",\n  \"blue\": \"{}\",\n  \"yellow\": \"{}\",\n  \"cyan\": \"{}\",\n  \"purple\": \"{}\"\n}}\n",
            self.dark,
            h(self.base),
            h(self.mantle),
            h(self.surface),
            h(self.surface2),
            h(self.muted),
            h(self.subtle),
            h(self.text),
            h(self.red),
            h(self.green),
            h(self.blue),
            h(self.yellow),
            h(self.cyan),
            h(self.purple)
        )
    }

    /// Pattern-grid colours derived from the scheme.
    pub fn palette(&self) -> Palette {
        let play = mix(self.base, self.green, 0.4);
        let [r, g, b, _] = self.surface2.to_array();
        Palette {
            bg: self.base,
            band: self.surface,
            beat_row: mix(self.base, self.surface, 0.45),
            play_row: play,
            play_row_dim: Color32::from_rgba_unmultiplied(play.r(), play.g(), play.b(), 120),
            divider: mix(self.blue, self.base, 0.4),
            column_sep: self.surface2,
            text_bar: self.text,
            text_beat: self.subtle,
            text: mix(self.muted, self.subtle, 0.35),
            text_other: mix(self.muted, self.base, 0.45),
            text_muted: self.muted,
            selection: mix(self.base, self.blue, 0.4),
            cursor_edit: self.red,
            cursor_view: self.cyan,
            header_text: self.text,
            header_text_muted: self.muted,
            header_lift: if self.dark { 60 } else { -40 },
            header_base_div: if self.dark { 2 } else { 1 },
            header_default: [r, g, b],
        }
    }

    /// egui widget colours for the whole interface.
    pub fn visuals(&self) -> egui::Visuals {
        use egui::Stroke;
        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.dark_mode = self.dark;
        v.panel_fill = self.mantle;
        v.window_fill = self.base;
        v.extreme_bg_color = mix(self.mantle, self.base, 0.5);
        v.faint_bg_color = self.surface;
        v.code_bg_color = self.surface;
        v.hyperlink_color = self.blue;
        v.warn_fg_color = self.yellow;
        v.error_fg_color = self.red;
        v.window_stroke = Stroke::new(1.0_f32, self.surface2);
        v.selection.bg_fill = mix(self.base, self.blue, 0.5);
        v.selection.stroke = Stroke::new(1.0_f32, self.text);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.mantle;
        w.noninteractive.weak_bg_fill = self.mantle;
        w.noninteractive.bg_stroke = Stroke::new(1.0_f32, self.surface2);
        w.noninteractive.fg_stroke = Stroke::new(1.0_f32, self.text);
        w.inactive.bg_fill = self.surface;
        w.inactive.weak_bg_fill = self.surface;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0_f32, self.text);
        w.hovered.bg_fill = self.surface2;
        w.hovered.weak_bg_fill = self.surface2;
        w.hovered.bg_stroke = Stroke::new(1.0_f32, self.muted);
        w.hovered.fg_stroke = Stroke::new(1.5_f32, self.text);
        w.active.bg_fill = mix(self.surface2, self.blue, 0.4);
        w.active.weak_bg_fill = mix(self.surface2, self.blue, 0.4);
        w.active.bg_stroke = Stroke::new(1.0_f32, self.blue);
        w.active.fg_stroke = Stroke::new(2.0_f32, self.text);
        w.open.bg_fill = self.surface;
        w.open.weak_bg_fill = self.surface;
        w.open.fg_stroke = Stroke::new(1.0_f32, self.text);
        v
    }
}

/// Look up a built-in colour scheme by id.
pub fn scheme_named(id: &str) -> Option<Scheme> {
    SCHEMES.iter().find(|(i, _, _)| *i == id).map(|(_, _, s)| *s)
}

/// Read every `*.json` theme file in `dir`: (id `user:<file name>`, display name, scheme).
pub fn load_user_themes(dir: &std::path::Path) -> Vec<(String, String, Scheme)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().map_or(true, |x| x != "json") {
            continue;
        }
        let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
        let Some(v) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        else {
            continue;
        };
        let dark = v.get("dark").and_then(|d| d.as_bool()).unwrap_or(true);
        let get = |k: &str| v.get(k).and_then(|x| x.as_str()).and_then(Scheme::parse_hex);
        let Some(scheme) = Scheme::from_fields(dark, get) else { continue };
        let name = v.get("name").and_then(|n| n.as_str()).map_or_else(|| stem.clone(), str::to_string);
        out.push((format!("user:{stem}"), name, scheme));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

impl Palette {
    pub fn named(name: &str) -> Palette {
        match name {
            "light" => Palette {
                bg: Color32::from_gray(246),
                band: Color32::from_gray(222),
                beat_row: Color32::from_gray(236),
                play_row: Color32::from_rgb(170, 230, 180),
                play_row_dim: Color32::from_rgba_unmultiplied(170, 230, 180, 120),
                divider: Color32::from_rgb(120, 140, 180),
                column_sep: Color32::from_gray(190),
                text_bar: Color32::BLACK,
                text_beat: Color32::from_gray(60),
                text: Color32::from_gray(110),
                text_other: Color32::from_gray(185),
                text_muted: Color32::from_gray(175),
                selection: Color32::from_rgb(170, 200, 245),
                cursor_edit: Color32::from_rgb(230, 70, 70),
                cursor_view: Color32::from_rgb(70, 190, 220),
                header_text: Color32::BLACK,
                header_text_muted: Color32::from_gray(150),
                header_lift: -40,
                header_base_div: 1,
                header_default: [200, 200, 215],
            },
            "contrast" => Palette {
                bg: Color32::BLACK,
                band: Color32::from_gray(55),
                beat_row: Color32::from_gray(26),
                play_row: Color32::from_rgb(0, 110, 40),
                play_row_dim: Color32::from_rgba_unmultiplied(0, 110, 40, 120),
                divider: Color32::from_rgb(120, 160, 255),
                column_sep: Color32::from_gray(110),
                text_bar: Color32::WHITE,
                text_beat: Color32::from_gray(230),
                text: Color32::from_gray(190),
                text_other: Color32::from_gray(110),
                text_muted: Color32::from_gray(110),
                selection: Color32::from_rgb(0, 70, 160),
                cursor_edit: Color32::from_rgb(255, 40, 40),
                cursor_view: Color32::from_rgb(0, 200, 255),
                header_text: Color32::WHITE,
                header_text_muted: Color32::from_gray(120),
                header_lift: 90,
                header_base_div: 2,
                header_default: [70, 70, 90],
            },
            _ => Palette {
                bg: Color32::from_gray(18),
                band: Color32::from_gray(34),
                beat_row: Color32::from_gray(24),
                play_row: Color32::from_rgb(0, 90, 30),
                play_row_dim: Color32::from_rgba_unmultiplied(0, 90, 30, 120),
                divider: Color32::from_rgb(70, 90, 130),
                column_sep: Color32::from_gray(60),
                text_bar: Color32::WHITE,
                text_beat: Color32::from_gray(170),
                text: Color32::from_gray(110),
                text_other: Color32::from_gray(62),
                text_muted: Color32::from_gray(70),
                selection: Color32::from_rgb(30, 60, 110),
                cursor_edit: Color32::from_rgb(210, 40, 40),
                cursor_view: Color32::from_rgb(0, 150, 180),
                header_text: Color32::WHITE,
                header_text_muted: Color32::DARK_GRAY,
                header_lift: 60,
                header_base_div: 2,
                header_default: [60, 60, 70],
            },
        }
    }

    /// Header fill for a track: its colour (or the default), brighter when selected.
    pub fn header_fill(&self, track_color: Option<[u8; 3]>, selected: bool) -> Color32 {
        let [r, g, b] = track_color.unwrap_or(self.header_default);
        let f = |c: u8| -> u8 {
            let base = (c / self.header_base_div) as i32;
            (if selected { base + self.header_lift } else { base }).clamp(0, 255) as u8
        };
        Color32::from_rgb(f(r), f(g), f(b))
    }
}

pub fn visuals(name: &str) -> egui::Visuals {
    match name {
        "light" => egui::Visuals::light(),
        _ => egui::Visuals::dark(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Color32) -> f32 {
        let f = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_scheme_is_readable_and_has_a_unique_id() {
        let mut ids = std::collections::BTreeSet::new();
        let mut problems = Vec::new();
        for (id, name, s) in SCHEMES {
            assert!(ids.insert(*id) && !name.is_empty(), "{id}");
            let p = s.palette();
            let mut check = |ok: bool, what: &str, v: f32| {
                if !ok {
                    problems.push(format!("{id}: {what} {v:.2}"));
                }
            };
            let v = contrast(s.text, s.base);
            check(v >= 4.5, "text on base", v);
            let v = contrast(s.text, s.mantle);
            check(v >= 4.5, "text on panels", v);
            let v = contrast(p.text_beat, p.bg);
            check(v >= 3.0, "beat rows", v);
            let v = contrast(p.text, p.bg);
            check(v >= 1.8, "normal rows", v);
            let v = contrast(p.cursor_edit, p.bg);
            check(v >= 2.0, "edit cursor", v);
            let v = contrast(p.cursor_view, p.bg);
            check(v >= 2.0, "view cursor", v);
            check(s.dark == (lum(s.base) < 0.3), "dark flag", lum(s.base));
            assert!(scheme_named(id).is_some());
        }
        assert!(problems.is_empty(), "{problems:#?}");
        assert_eq!(SCHEMES.len(), 15);
    }

    #[test]
    fn user_theme_files_load_with_defaults_and_bad_ones_are_skipped() {
        let dir = std::env::temp_dir().join(format!("trakr-themes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mine.json"),
            r##"{"name": "Mine", "dark": true, "base": "#101820", "text": "#e0e0e0", "red": "#ff0000"}"##,
        )
        .unwrap();
        std::fs::write(dir.join("bare.json"), r##"{"base": "#ffffff", "text": "#000000", "dark": false}"##)
            .unwrap();
        std::fs::write(dir.join("broken.json"), "{ not json").unwrap();
        std::fs::write(dir.join("incomplete.json"), r##"{"base": "#123456"}"##).unwrap();
        std::fs::write(dir.join("badhex.json"), r##"{"base": "zzz", "text": "#000000"}"##).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        let t = load_user_themes(&dir);
        let ids: Vec<&str> = t.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(ids, ["user:bare", "user:mine"]);
        assert_eq!(t[1].1, "Mine");
        assert_eq!(t[0].1, "bare", "the file name is used when there is no name");
        assert_eq!(t[1].2.red, Color32::from_rgb(255, 0, 0));
        assert!(contrast(t[1].2.subtle, t[1].2.base) > 3.0, "derived colours stay readable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exported_theme_text_loads_back_identically() {
        let (_, _, s) = SCHEMES[3];
        let v: serde_json::Value = serde_json::from_str(&s.to_json("x")).unwrap();
        let back = Scheme::from_fields(v["dark"].as_bool().unwrap(), |k| {
            v.get(k).and_then(|x| x.as_str()).and_then(Scheme::parse_hex)
        })
        .unwrap();
        for (a, b) in [
            (s.base, back.base),
            (s.mantle, back.mantle),
            (s.text, back.text),
            (s.purple, back.purple),
            (s.surface2, back.surface2),
        ] {
            assert_eq!(a, b);
        }
        assert_eq!(Scheme::parse_hex("#ff8000"), Some(Color32::from_rgb(255, 128, 0)));
        assert_eq!(Scheme::parse_hex("ff80"), None);
    }
}
