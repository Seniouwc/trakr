//! Native UI for trakr (egui/eframe). Thin renderer over `trakr-editor`.

use eframe::egui::{self, Align2, Color32, FontId, Key, Rect, Sense, Vec2};
use std::path::PathBuf;
use trakr_core::Song;
use trakr_editor::{cell_text, Editor, KeyCode, KeyEvent, FX_HL, FX_SUBS, NOTE_HL, NOTE_SUBS};
use trakr_engine::EngineHandle;

const NOTE_COL_W: usize = 16;
const FX_COL_W: usize = 5;
const FONT_SIZE: f32 = 15.0;

/// Run the GUI until the window closes. `engine` is `None` when no audio device is available.
pub fn run(song: Song, path: Option<PathBuf>, engine: Option<EngineHandle>) -> anyhow::Result<()> {
    let editor = Editor::new(song, path, engine);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 700.0]).with_title("trakr"),
        ..Default::default()
    };
    eframe::run_native("trakr", options, Box::new(|_cc| Ok(Box::new(GuiApp { editor }))))
        .map_err(|e| anyhow::anyhow!("{e}"))
}

struct GuiApp {
    editor: Editor,
}

fn nav_key(k: Key) -> Option<KeyCode> {
    Some(match k {
        Key::ArrowUp => KeyCode::Up,
        Key::ArrowDown => KeyCode::Down,
        Key::ArrowLeft => KeyCode::Left,
        Key::ArrowRight => KeyCode::Right,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::Tab => KeyCode::Tab,
        Key::Backspace => KeyCode::Backspace,
        Key::Delete => KeyCode::Delete,
        Key::Insert => KeyCode::Insert,
        Key::Escape => KeyCode::Esc,
        Key::F1 => KeyCode::F(1),
        Key::F2 => KeyCode::F(2),
        Key::F3 => KeyCode::F(3),
        Key::F4 => KeyCode::F(4),
        Key::F5 => KeyCode::F(5),
        Key::F6 => KeyCode::F(6),
        Key::F7 => KeyCode::F(7),
        Key::F8 => KeyCode::F(8),
        Key::F9 => KeyCode::F(9),
        Key::F10 => KeyCode::F(10),
        _ => return None,
    })
}

fn letter_key(k: Key) -> Option<char> {
    Some(match k {
        Key::A => 'a',
        Key::N => 'n',
        Key::Q => 'q',
        Key::S => 's',
        Key::Y => 'y',
        Key::Z => 'z',
        _ => return None,
    })
}

/// Translate egui events: navigation/function keys and Ctrl combos from key events, typed characters from text events.
fn collect_keys(ctx: &egui::Context) -> Vec<KeyEvent> {
    let mut out = Vec::new();
    ctx.input(|i| {
        for ev in &i.events {
            match ev {
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    let mut k = KeyEvent {
                        code: KeyCode::Tab,
                        ctrl: modifiers.command,
                        alt: modifiers.alt,
                        shift: modifiers.shift,
                    };
                    if let Some(c) = letter_key(*key).filter(|_| modifiers.command) {
                        k.code = KeyCode::Char(c);
                        out.push(k);
                    } else if let Some(code) = nav_key(*key) {
                        k.code =
                            if code == KeyCode::Tab && modifiers.shift { KeyCode::BackTab } else { code };
                        out.push(k);
                    }
                }
                egui::Event::Copy => out.push(KeyEvent::ctrl('c')),
                egui::Event::Cut => out.push(KeyEvent::ctrl('x')),
                egui::Event::Paste(_) => out.push(KeyEvent::ctrl('v')),
                egui::Event::Text(t) => {
                    out.extend(
                        t.chars().filter(|c| !c.is_control()).map(|c| KeyEvent::new(KeyCode::Char(c))),
                    );
                }
                _ => {}
            }
        }
    });
    out
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for k in collect_keys(ctx) {
            self.editor.on_key(k);
        }
        self.editor.tick();
        if self.editor.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        let ed = &self.editor;
        let (playing, play_seq, play_line) = ed.playhead();
        let s = &ed.song;

        egui::TopBottomPanel::top("info").show(ctx, |ui| {
            ui.monospace(format!(
                "{}{}  BPM {}  LPB {}  TPL {}  |  Oct {}  Step {}  Inst {:02X}  |  {}  |  {}",
                s.name,
                if ed.dirty { "*" } else { "" },
                s.bpm,
                s.lpb,
                s.tpl,
                ed.octave,
                ed.edit_step,
                ed.instrument,
                if ed.edit_mode { "EDIT" } else { "----" },
                if playing { format!("PLAY {play_seq:02X}:{play_line:02X}") } else { "STOP".into() },
            ));
        });
        egui::TopBottomPanel::bottom("help").show(ctx, |ui| {
            ui.monospace(format!(
                "F5 song  F6 loop pat  F7 from cursor  F8 stop  Space edit  F1/F2 oct  F3/F4 step  F9/F10 inst  Ctrl-S save  Ctrl-Q quit  | {}",
                ed.status
            ));
        });
        egui::CentralPanel::default().show(ctx, |ui| draw_grid(ui, ed, playing, play_seq, play_line));
    }
}

fn track_width(ed: &Editor, t: usize) -> usize {
    let (n, f) = ed.cols(t);
    n * NOTE_COL_W + f * FX_COL_W + 1
}

fn draw_grid(ui: &mut egui::Ui, ed: &Editor, playing: bool, play_seq: usize, play_line: u16) {
    let font = FontId::monospace(FONT_SIZE);
    let (cw, rh) = ui.fonts(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
    let area = resp.rect;
    let cur = ed.cursor;
    let s = &ed.song;

    painter.rect_filled(area, 0.0, Color32::from_gray(18));

    // Horizontal scroll: first track such that the cursor track fits.
    let width_chars = (area.width() / cw) as usize;
    let gutter = 3;
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

    // Track header row.
    let mut x = area.left() + gutter as f32 * cw;
    for &t in &tracks {
        let color = if t == cur.track { Color32::YELLOW } else { Color32::GRAY };
        painter.text(egui::pos2(x, area.top()), Align2::LEFT_TOP, &s.tracks[t].name, font.clone(), color);
        x += track_width(ed, t) as f32 * cw;
    }

    let height = ((area.height() / rh) as usize).saturating_sub(1);
    let nlines = ed.lines_in_pattern() as usize;
    let mut top = (cur.line as usize).saturating_sub(height / 2);
    if nlines >= height && top + height > nlines {
        top = nlines - height;
    }
    let pat = ed.pattern_index().and_then(|i| s.patterns.get(i));
    let lpb = s.lpb.max(1) as usize;

    for r in 0..height {
        let li = top + r;
        if li >= nlines {
            break;
        }
        let y = area.top() + (r + 1) as f32 * rh;
        let is_cursor_row = li == cur.line as usize;
        let is_play_row = playing && play_seq == ed.seq_pos && play_line as usize == li;
        let row = Rect::from_min_size(egui::pos2(area.left(), y), Vec2::new(area.width(), rh));
        if is_play_row {
            painter.rect_filled(row, 0.0, Color32::from_rgb(0, 70, 20));
        } else if is_cursor_row {
            painter.rect_filled(row, 0.0, Color32::from_gray(45));
        }
        let fg = if li % (lpb * 4) == 0 {
            Color32::WHITE
        } else if li % lpb == 0 {
            Color32::GRAY
        } else {
            Color32::DARK_GRAY
        };
        painter.text(egui::pos2(area.left(), y), Align2::LEFT_TOP, format!("{li:02X}"), font.clone(), fg);

        let mut x = area.left() + gutter as f32 * cw;
        for &t in &tracks {
            let (n, fxn) = ed.cols(t);
            let line = pat.and_then(|p| p.line(t, li as u16));
            for c in 0..n + fxn {
                if is_cursor_row && t == cur.track && c == cur.col {
                    let (a, b) = if c < n {
                        NOTE_HL[cur.sub.min(NOTE_SUBS - 1)]
                    } else {
                        FX_HL[cur.sub.min(FX_SUBS - 1)]
                    };
                    let hl = Rect::from_min_size(
                        egui::pos2(x + a as f32 * cw, y),
                        Vec2::new((b - a) as f32 * cw, rh),
                    );
                    painter.rect_filled(
                        hl,
                        0.0,
                        if ed.edit_mode { Color32::RED } else { Color32::from_rgb(0, 160, 190) },
                    );
                }
                painter.text(egui::pos2(x, y), Align2::LEFT_TOP, cell_text(line, c, n), font.clone(), fg);
                x += if c < n { NOTE_COL_W } else { FX_COL_W } as f32 * cw;
            }
            painter.text(egui::pos2(x, y), Align2::LEFT_TOP, "│", font.clone(), Color32::DARK_GRAY);
            x += cw;
        }
    }
}
