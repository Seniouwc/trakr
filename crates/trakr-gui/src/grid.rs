//! The pattern editor: a centered, scrolling grid like a tracker should have. The cursor (or the
//! playhead, when following) stays on the middle row and the pattern scrolls underneath it.

use crate::GuiApp;
use eframe::egui::{self, Align2, Color32, EventFilter, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use trakr_core::MuteState;
use trakr_editor::{cell_text_hidden, note_layout, Action, Editor, FX_HL, FX_SUBS, NOTE_SUBS};

const FX_COL_W: usize = 5;

/// Width in characters of one note column of a track, with a separating space.
fn note_col_w(ed: &Editor, t: usize) -> usize {
    note_layout(ed.note_hidden(t)).0 + 1
}

fn track_width(ed: &Editor, t: usize) -> usize {
    let (n, f) = ed.cols(t);
    n * note_col_w(ed, t) + f * FX_COL_W + 1
}

/// One drawn column, for mouse hit-testing.
struct ColRect {
    track: usize,
    col: usize,
    notes: usize,
    x: f32,
    hide: [bool; 3],
}

enum Click {
    Cell { track: usize, col: usize, sub: usize, line: u16 },
    Header { track: usize },
    Mute { track: usize },
    Solo { track: usize },
}

impl GuiApp {
    pub fn pattern_view(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette;
        let font = FontId::monospace(self.editor.prefs.font_size.clamp(9.0, 32.0));
        let (cw, rh) = ui.fonts(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let area = resp.rect;

        // Keep keyboard focus on the grid (Tab and arrows must reach the editor, not egui's focus system).
        let id = resp.id;
        // Compute these first: Response methods lock the context, and so does `memory_mut`.
        let took_pointer = resp.clicked() || resp.drag_started();
        if took_pointer {
            self.phrase_focus = false;
        }
        ui.memory_mut(|m| {
            if m.focused().is_none() || took_pointer {
                m.request_focus(id);
            }
            m.set_focus_lock_filter(
                id,
                EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
            );
        });

        painter.rect_filled(area, 0.0, pal.bg);

        let (cols, header_hits, top_line) = {
            let ed = &self.editor;
            let cur = ed.cursor;
            let s = &ed.song;
            let (playing, play_seq, play_line) = ed.playhead();
            let gutter = 3usize;
            let width_chars = (area.width() / cw) as usize;

            let mut first = 0;
            while first < cur.track
                && (first..=cur.track).map(|t| track_width(ed, t)).sum::<usize>() + gutter > width_chars
            {
                first += 1;
            }
            let mut tracks = Vec::new();
            let mut used = gutter;
            for t in first..s.tracks.len() {
                let w = track_width(ed, t);
                if used + w > width_chars && t > cur.track {
                    break;
                }
                used += w;
                tracks.push(t);
            }

            // Header row.
            let mut header_hits: Vec<(usize, f32, f32)> = Vec::new();
            let mut x = area.left() + gutter as f32 * cw;
            for &t in &tracks {
                let w = track_width(ed, t) as f32 * cw;
                let tr = &s.tracks[t];
                let rect = Rect::from_min_size(Pos2::new(x, area.top()), Vec2::new(w - cw, rh));
                let bg = pal.header_fill(tr.color, t == cur.track);
                painter.rect_filled(rect, 2.0, bg);
                let muted = tr.mute != MuteState::Active;
                let color = if muted { pal.header_text_muted } else { pal.header_text };
                painter.with_clip_rect(rect).text(
                    Pos2::new(x + 2.0, area.top()),
                    Align2::LEFT_TOP,
                    &tr.name,
                    font.clone(),
                    color,
                );
                if muted {
                    painter.text(
                        Pos2::new(rect.right() - cw, area.top()),
                        Align2::LEFT_TOP,
                        "M",
                        font.clone(),
                        Color32::LIGHT_RED,
                    );
                }
                if tr.solo {
                    painter.text(
                        Pos2::new(rect.right() - 2.0 * cw, area.top()),
                        Align2::LEFT_TOP,
                        "S",
                        font.clone(),
                        Color32::YELLOW,
                    );
                }
                header_hits.push((t, x, w));
                x += w;
            }

            let height = ((area.height() / rh) as usize).saturating_sub(1).max(1);
            let nlines = ed.lines_in_pattern() as isize;
            let top = cur.line as isize - (height / 2) as isize;
            let lpb = if ed.prefs.highlight_beat > 0 {
                ed.prefs.highlight_beat as isize
            } else {
                s.lpb.max(1) as isize
            };
            let bar = lpb * if ed.prefs.highlight_bar > 0 { ed.prefs.highlight_bar as isize } else { 4 };
            let decimal = ed.prefs.decimal_lines;
            let sel = ed.selection();
            let mut cols: Vec<ColRect> = Vec::new();

            // The cursor row sits at a fixed screen row; draw its band first so it shows through gaps.
            let mid_y = area.top() + ((height / 2) + 1) as f32 * rh;
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(area.left(), mid_y), Vec2::new(area.width(), rh)),
                0.0,
                pal.band,
            );

            // Lines outside the current pattern come from the neighbouring slots, so the view scrolls
            // continuously through the sequence (dimmed), like the song really plays.
            let nseq = s.sequence.len() as isize;
            let lines_of = |slot: isize| {
                s.sequence
                    .get(slot as usize)
                    .and_then(|&p| s.patterns.get(p))
                    .map_or(1, |p| p.number_of_lines) as isize
            };
            let resolve = |li: isize| -> Option<(usize, isize, bool)> {
                if (0..nlines).contains(&li) {
                    return Some((ed.seq_pos, li, true));
                }
                let (mut slot, mut l) = (ed.seq_pos as isize, li);
                for _ in 0..16 {
                    if l < 0 {
                        slot = (slot - 1).rem_euclid(nseq);
                        l += lines_of(slot);
                    } else if l >= lines_of(slot) {
                        l -= lines_of(slot);
                        slot = (slot + 1).rem_euclid(nseq);
                    } else {
                        return Some((slot as usize, l, false));
                    }
                }
                None
            };

            for r in 0..height {
                let Some((slot, li, in_current)) = resolve(top + r as isize) else { continue };
                let pat = s.sequence.get(slot).and_then(|&p| s.patterns.get(p));
                let y = area.top() + (r + 1) as f32 * rh;
                let is_cursor_row = in_current && li == cur.line as isize;
                let is_play_row = playing && play_seq == slot && play_line as isize == li;
                let row = Rect::from_min_size(Pos2::new(area.left(), y), Vec2::new(area.width(), rh));
                if is_play_row {
                    painter.rect_filled(row, 0.0, if in_current { pal.play_row } else { pal.play_row_dim });
                } else if li % lpb == 0 && !is_cursor_row {
                    painter.rect_filled(row, 0.0, pal.beat_row);
                }
                if li == 0 && r > 0 {
                    painter.line_segment(
                        [Pos2::new(area.left(), y), Pos2::new(area.right(), y)],
                        Stroke::new(1.0_f32, pal.divider),
                    );
                }
                let fg = if !in_current {
                    pal.text_other
                } else if li % bar == 0 {
                    pal.text_bar
                } else if li % lpb == 0 {
                    pal.text_beat
                } else {
                    pal.text
                };
                painter.text(
                    Pos2::new(area.left() + 2.0, y),
                    Align2::LEFT_TOP,
                    if decimal { format!("{li:02}") } else { format!("{li:02X}") },
                    font.clone(),
                    fg,
                );

                let mut x = area.left() + gutter as f32 * cw;
                for &t in &tracks {
                    let (n, fxn) = ed.cols(t);
                    let line = pat.and_then(|p| p.line(t, li as u16));
                    let flat0 = ed.flat_index(t, 0);
                    let note_w = note_col_w(ed, t);
                    let (_, note_hl) = note_layout(ed.note_hidden(t));
                    let hide = ed.note_hidden(t);
                    let muted = s.tracks[t].mute != MuteState::Active;
                    for c in 0..n + fxn {
                        let chars = if c < n { note_w } else { FX_COL_W };
                        if r == 0 {
                            cols.push(ColRect { track: t, col: c, notes: n, x, hide });
                        }
                        let selected = in_current
                            && sel.is_some_and(|b| {
                                (b.col0..=b.col1).contains(&(flat0 + c))
                                    && (b.line0 as isize..=b.line1 as isize).contains(&li)
                            });
                        if selected {
                            painter.rect_filled(
                                Rect::from_min_size(Pos2::new(x, y), Vec2::new(chars as f32 * cw, rh)),
                                0.0,
                                pal.selection,
                            );
                        }
                        if is_cursor_row && t == cur.track && c == cur.col {
                            let (a, b) = if c < n {
                                note_hl[cur.sub.min(NOTE_SUBS - 1)]
                            } else {
                                FX_HL[cur.sub.min(FX_SUBS - 1)]
                            };
                            let hl = Rect::from_min_size(
                                Pos2::new(x + a as f32 * cw, y),
                                Vec2::new((b - a) as f32 * cw, rh),
                            );
                            painter.rect_filled(
                                hl,
                                0.0,
                                if ed.edit_mode { pal.cursor_edit } else { pal.cursor_view },
                            );
                        }
                        let text_color = if muted && in_current { pal.text_muted } else { fg };
                        painter.text(
                            Pos2::new(x, y),
                            Align2::LEFT_TOP,
                            cell_text_hidden(line, c, n, hide),
                            font.clone(),
                            text_color,
                        );
                        x += chars as f32 * cw;
                    }
                    painter.text(Pos2::new(x, y), Align2::LEFT_TOP, "│", font.clone(), pal.column_sep);
                    x += cw;
                }
            }
            (cols, header_hits, top)
        };

        self.grid_info = crate::GridInfo {
            area,
            char_w: cw,
            row_h: rh,
            cursor_row_y: area.top()
                + (((area.height() / rh) as usize).saturating_sub(1).max(1) / 2 + 1) as f32 * rh,
            cols: cols
                .iter()
                .map(|c| {
                    (c.track, c.col, c.x, if c.col < c.notes { note_layout(c.hide).0 + 1 } else { FX_COL_W })
                })
                .collect(),
        };

        // Mouse: build a click description from the hit position.
        let hit = |pos: Pos2| -> Option<Click> {
            let row = ((pos.y - area.top()) / rh).floor() as isize;
            if row < 0 {
                return None;
            }
            if row == 0 {
                let &(track, x, w) = header_hits.iter().find(|(_, x, w)| pos.x >= *x && pos.x < x + w)?;
                return Some(if pos.x >= x + w - 2.0 * cw {
                    Click::Mute { track }
                } else if pos.x >= x + w - 4.0 * cw {
                    Click::Solo { track }
                } else {
                    Click::Header { track }
                });
            }
            let nlines = self.editor.lines_in_pattern() as isize;
            let line = (top_line + row - 1).clamp(0, nlines - 1) as u16;
            let c = cols.iter().rev().find(|c| pos.x >= c.x)?;
            let chars = ((pos.x - c.x) / cw).floor().max(0.0) as usize;
            // visible sub-fields only: (sub index, start, end)
            let ranges: Vec<(usize, usize, usize)> = if c.col < c.notes {
                note_layout(c.hide)
                    .1
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.1 > r.0)
                    .map(|(i, r)| (i, r.0, r.1))
                    .collect()
            } else {
                FX_HL.iter().enumerate().map(|(i, r)| (i, r.0, r.1)).collect()
            };
            let sub = ranges
                .iter()
                .find(|(_, a, b)| chars >= *a && chars < *b)
                .or_else(|| ranges.iter().rev().find(|(_, a, _)| *a <= chars))
                .map_or(0, |r| r.0);
            Some(Click::Cell { track: c.track, col: c.col, sub, line })
        };

        if resp.drag_started() || resp.clicked() || resp.secondary_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                match hit(pos) {
                    Some(Click::Cell { track, col, sub, line }) => {
                        let inside = self.editor.selection().is_some_and(|b| {
                            let f = self.editor.flat_index(track, col);
                            (b.col0..=b.col1).contains(&f) && (b.line0..=b.line1).contains(&line)
                        });
                        if !(resp.secondary_clicked() && inside) {
                            self.editor.set_cursor(track, col, sub, line);
                        }
                    }
                    Some(Click::Header { track }) => {
                        let line = self.editor.cursor.line;
                        self.editor.set_cursor(track, 0, 0, line);
                    }
                    Some(Click::Mute { track }) => self.editor.track_toggle_mute_at(track),
                    Some(Click::Solo { track }) => self.editor.track_toggle_solo(track),
                    None => {}
                }
            }
        } else if resp.dragged() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if let Some(Click::Cell { track, col, line, .. }) = hit(pos) {
                    self.editor.select_to(track, col, line);
                }
            }
        }

        if resp.hovered() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            let steps = (dy / rh).round() as i32;
            if steps != 0 {
                self.editor.move_lines(-steps * 2);
            }
        }

        resp.context_menu(|ui| {
            for a in [Action::Cut, Action::Copy, Action::Paste, Action::Clear, Action::SelectAll] {
                if ui.add(egui::Button::new(a.label()).shortcut_text(self.editor.shortcut_for(a))).clicked() {
                    self.run_action(a);
                    ui.close_menu();
                }
            }
            ui.separator();
            for a in [Action::TransposeUp, Action::TransposeDown, Action::Reverse, Action::Interpolate] {
                if ui.add(egui::Button::new(a.label()).shortcut_text(self.editor.shortcut_for(a))).clicked() {
                    self.run_action(a);
                    ui.close_menu();
                }
            }
            ui.separator();
            let track = self.editor.cursor.track;
            let hidden = self.editor.note_hidden(track);
            for (k, label) in
                ["Show volume column", "Show panning column", "Show delay column"].into_iter().enumerate()
            {
                let mut shown = !hidden[k];
                if ui.checkbox(&mut shown, label).changed() {
                    self.editor.track_set_sub_column(track, k, shown);
                }
            }
            ui.menu_button("Track color", |ui| {
                const COLORS: [(&str, Option<[u8; 3]>); 8] = [
                    ("None", None),
                    ("Red", Some([200, 70, 70])),
                    ("Orange", Some([210, 140, 50])),
                    ("Yellow", Some([200, 190, 60])),
                    ("Green", Some([80, 170, 90])),
                    ("Cyan", Some([60, 170, 180])),
                    ("Blue", Some([80, 120, 210])),
                    ("Purple", Some([150, 90, 200])),
                ];
                for (name, color) in COLORS {
                    if ui.button(name).clicked() {
                        self.editor.track_set_color(track, color);
                        ui.close_menu();
                    }
                }
            });
            ui.separator();
            for a in [
                Action::TrackAdd,
                Action::TrackRemove,
                Action::TrackMute,
                Action::NoteColumnAdd,
                Action::NoteColumnRemove,
                Action::EffectColumnAdd,
                Action::EffectColumnRemove,
            ] {
                if ui.button(a.label()).clicked() {
                    self.run_action(a);
                    ui.close_menu();
                }
            }
        });
    }
}
