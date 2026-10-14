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

pub const THEMES: [(&str, &str); 3] = [("dark", "Dark"), ("light", "Light"), ("contrast", "High contrast")];

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
