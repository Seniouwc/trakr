use crate::app::{App, FX_SUBS, NOTE_SUBS};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as TLine, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use trakr_core::note::note_name;
use trakr_core::{Line, EMPTY};

const NOTE_COL_W: usize = 16; // "C-4 01 40 80 00" + 1
const FX_COL_W: usize = 5; // "0A40" + 1

// Char ranges of each sub-field inside a column's text.
const NOTE_HL: [(usize, usize); NOTE_SUBS] =
    [(0, 3), (4, 5), (5, 6), (7, 8), (8, 9), (10, 11), (11, 12), (13, 14), (14, 15)];
const FX_HL: [(usize, usize); FX_SUBS] = [(0, 1), (1, 2), (2, 3), (3, 4)];

fn hex2(v: u8, empty: u8) -> String {
    if v == empty {
        "..".into()
    } else {
        format!("{v:02X}")
    }
}

fn cell_text(line: Option<&Line>, c: usize, notes: usize) -> String {
    if c < notes {
        let n = line.map(|l| l.notes[c]).unwrap_or_default();
        format!(
            "{} {} {} {} {}",
            note_name(n.note),
            hex2(n.instrument, EMPTY),
            hex2(n.volume, EMPTY),
            hex2(n.panning, EMPTY),
            hex2(n.delay, 0)
        )
    } else {
        let e = line.map(|l| l.effects[c - notes]).unwrap_or_default();
        if e.is_empty() {
            "....".into()
        } else {
            format!("{}{}{:02X}", e.id[0] as char, e.id[1] as char, e.amount)
        }
    }
}

fn track_width(app: &App, t: usize) -> usize {
    let (n, f) = app.cols(t);
    n * NOTE_COL_W + f * FX_COL_W + 1
}

pub fn draw(f: &mut Frame, app: &App) {
    let [head, body, foot] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(5), Constraint::Length(1)]).areas(f.area());

    let (playing, play_seq, play_line) = app.engine.as_ref().map_or((false, 0, 0), |e| e.pos.get());
    let s = &app.song;
    let info = format!(
        " {}{}  BPM {}  LPB {}  TPL {}  |  Oct {}  Step {}  Inst {:02X}  |  {}  |  {}",
        s.name,
        if app.dirty { "*" } else { "" },
        s.bpm,
        s.lpb,
        s.tpl,
        app.octave,
        app.edit_step,
        app.instrument,
        if app.edit_mode { "EDIT" } else { "----" },
        if playing { format!("PLAY {play_seq:02X}:{play_line:02X}") } else { "STOP".into() },
    );
    f.render_widget(
        Paragraph::new(info).block(Block::default().borders(Borders::ALL).title(" trakr ")),
        head,
    );

    let pat_title = format!(
        " Pattern {:02X}  seq {:02X}/{:02X}  {} lines ",
        app.pattern_index().unwrap_or(0),
        app.seq_pos,
        s.sequence.len().saturating_sub(1),
        app.lines_in_pattern()
    );
    let block = Block::default().borders(Borders::ALL).title(pat_title);
    let inner = block.inner(body);
    f.render_widget(block, body);

    let width = inner.width as usize;
    let height = inner.height.saturating_sub(1) as usize; // one row for track headers
    let cur = app.cursor;

    // Horizontal scroll: first track such that the cursor track fits.
    let mut first = 0;
    while first < cur.track && (first..=cur.track).map(|t| track_width(app, t)).sum::<usize>() + 3 > width {
        first += 1;
    }
    let mut tracks = Vec::new();
    let mut used = 3;
    for t in first..s.tracks.len() {
        let w = track_width(app, t);
        if used + w > width && t > cur.track {
            break;
        }
        used += w;
        tracks.push(t);
    }

    let mut rows: Vec<TLine> = Vec::with_capacity(height + 1);
    let mut header = vec![Span::raw("   ")];
    for &t in &tracks {
        let w = track_width(app, t);
        let name: String = s.tracks[t].name.chars().take(w - 1).collect();
        let style = if t == cur.track {
            Style::new().add_modifier(Modifier::BOLD).fg(Color::Yellow)
        } else {
            Style::new().fg(Color::Gray)
        };
        header.push(Span::styled(format!("{name:<w$}"), style));
    }
    rows.push(TLine::from(header));

    let nlines = app.lines_in_pattern() as usize;
    let mut top = (cur.line as usize).saturating_sub(height / 2);
    if nlines >= height && top + height > nlines {
        top = nlines - height;
    }
    let pat = app.pattern_index().and_then(|i| s.patterns.get(i));
    let lpb = s.lpb.max(1) as usize;

    for r in 0..height {
        let li = top + r;
        if li >= nlines {
            rows.push(TLine::from(""));
            continue;
        }
        let is_cursor_row = li == cur.line as usize;
        let is_play_row = playing && play_seq == app.seq_pos && play_line as usize == li;
        let row_bg = if is_play_row {
            Some(Color::Indexed(22))
        } else if is_cursor_row {
            Some(Color::Indexed(237))
        } else {
            None
        };
        let fg = if li % (lpb * 4) == 0 {
            Color::White
        } else if li % lpb == 0 {
            Color::Gray
        } else {
            Color::DarkGray
        };
        let mut base = Style::new().fg(fg);
        if let Some(bg) = row_bg {
            base = base.bg(bg);
        }
        let cur_style =
            Style::new().fg(Color::Black).bg(if app.edit_mode { Color::Red } else { Color::Cyan });

        let mut spans = vec![Span::styled(format!("{li:02X} "), base)];
        for &t in &tracks {
            let (n, fxn) = app.cols(t);
            let line = pat.and_then(|p| p.line(t, li as u16));
            for c in 0..n + fxn {
                let text = cell_text(line, c, n);
                let hl = (is_cursor_row && t == cur.track && c == cur.col).then(|| {
                    if c < n {
                        NOTE_HL[cur.sub.min(NOTE_SUBS - 1)]
                    } else {
                        FX_HL[cur.sub.min(FX_SUBS - 1)]
                    }
                });
                match hl {
                    None => spans.push(Span::styled(text, base)),
                    Some((a, b)) => {
                        spans.push(Span::styled(text[..a].to_string(), base));
                        spans.push(Span::styled(text[a..b].to_string(), cur_style));
                        spans.push(Span::styled(text[b..].to_string(), base));
                    }
                }
                spans.push(Span::styled(" ", base));
            }
            spans.push(Span::styled("│", base.fg(Color::DarkGray)));
        }
        rows.push(TLine::from(spans));
    }
    f.render_widget(Paragraph::new(rows), inner);

    let help = format!(
        " F5 song  F6 loop pat  F7 from cursor  F8 stop  Space edit  F1/F2 oct  F3/F4 step  F9/F10 inst  ^S save  ^Q quit  | {}",
        app.status
    );
    f.render_widget(Paragraph::new(help).style(Style::new().fg(Color::DarkGray)), foot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use trakr_core::Song;

    #[test]
    fn renders_demo_song_without_panicking() {
        let app = App::new(Song::demo(), None, None);
        for (w, h) in [(120u16, 40u16), (40, 10), (20, 6)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| draw(f, &app)).unwrap();
        }
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
        let text: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("C-4 00"), "pattern grid missing first note");
        assert!(text.contains("Track 01"));
    }
}
