//! Best-effort importer for Renoise `.xrns` projects (zip + `Song.xml`).
//!
//! The element names below come from memory of the format, NOT from a spec.
//! First job for whoever continues this: run it against real `.xrns` files,
//! fix whatever differs, and add them (if licensing allows) as test fixtures.
//!
//! Imported so far: song metadata, tempo, track list, pattern lengths, note
//! columns, effect columns, pattern sequence. Not yet imported: instruments and
//! samples, envelopes, devices/automation, phrases, groups, send routing.

use roxmltree::{Document, Node};
use std::io::{Read, Seek};
use thiserror::Error;
use trakr_core::note::parse_note;
use trakr_core::{EffectCmd, Pattern, Song, Track, TrackKind, EMPTY, MAX_EFFECT_COLUMNS, MAX_NOTE_COLUMNS};

#[derive(Debug, Error)]
pub enum XrnsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("xml: {0}")]
    Xml(#[from] roxmltree::Error),
    #[error("malformed xrns: {0}")]
    Malformed(&'static str),
}

pub fn import<R: Read + Seek>(r: R) -> Result<Song, XrnsError> {
    let mut zip = zip::ZipArchive::new(r)?;
    let mut xml = String::new();
    zip.by_name("Song.xml")?.read_to_string(&mut xml)?;
    import_xml(&xml)
}

pub fn import_path(path: &std::path::Path) -> Result<Song, XrnsError> {
    import(std::fs::File::open(path)?)
}

fn child<'a, 'b>(n: Node<'a, 'b>, name: &str) -> Option<Node<'a, 'b>> {
    n.children().find(|c| c.has_tag_name(name))
}

fn text<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(n, name).and_then(|c| c.text()).map(str::trim).filter(|s| !s.is_empty())
}

fn hex(n: Node, name: &str) -> Option<u8> {
    u8::from_str_radix(text(n, name)?, 16).ok()
}

pub fn import_xml(xml: &str) -> Result<Song, XrnsError> {
    let doc = Document::parse(xml)?;
    let root = doc.root_element();
    let mut song = Song::default();
    song.patterns.clear();
    song.sequence.clear();
    song.tracks.clear();

    if let Some(g) = child(root, "GlobalSongData") {
        if let Some(v) = text(g, "BeatsPerMin").and_then(|s| s.parse().ok()) {
            song.bpm = v;
        }
        if let Some(v) = text(g, "LinesPerBeat").and_then(|s| s.parse().ok()) {
            song.lpb = v;
        }
        if let Some(v) = text(g, "TicksPerLine").and_then(|s| s.parse().ok()) {
            song.tpl = v;
        }
        song.name = text(g, "SongName").unwrap_or("").to_string();
        song.author = text(g, "Artist").unwrap_or("").to_string();
    }

    if let Some(tracks) = child(root, "Tracks") {
        for t in tracks.children().filter(|c| c.is_element()) {
            let kind = match t.tag_name().name() {
                "SequencerTrack" => TrackKind::Sequencer,
                "SequencerGroupTrack" => TrackKind::Group,
                "SequencerSendTrack" => TrackKind::Send,
                "SequencerMasterTrack" => TrackKind::Master,
                _ => continue,
            };
            let mut track = Track::new(text(t, "Name").unwrap_or("Track"), kind);
            if let Some(v) = text(t, "NumberOfVisibleNoteColumns").and_then(|s| s.parse().ok()) {
                track.visible_note_columns = v;
            }
            if let Some(v) = text(t, "NumberOfVisibleEffectColumns").and_then(|s| s.parse().ok()) {
                track.visible_effect_columns = v;
            }
            song.tracks.push(track);
        }
    }

    if let Some(patterns) = child(root, "PatternPool").and_then(|p| child(p, "Patterns")) {
        for p in patterns.children().filter(|c| c.has_tag_name("Pattern")) {
            let lines = text(p, "NumberOfLines").and_then(|s| s.parse().ok()).unwrap_or(64);
            let mut pat = Pattern::new(song.tracks.len(), lines);
            pat.name = text(p, "Name").unwrap_or("").to_string();
            if let Some(tracks) = child(p, "Tracks") {
                let it = tracks.children().filter(|c| c.is_element() && c.has_tag_name("PatternTrack"));
                for (ti, pt) in it.enumerate() {
                    read_pattern_track(&mut pat, ti, pt);
                }
            }
            song.patterns.push(pat);
        }
    }

    if let Some(entries) = child(root, "PatternSequence").and_then(|s| child(s, "SequenceEntries")) {
        for e in entries.children().filter(|c| c.has_tag_name("SequenceEntry")) {
            if let Some(i) = text(e, "Pattern").and_then(|s| s.parse().ok()) {
                song.sequence.push(i);
            }
        }
    }

    if song.tracks.is_empty() || song.patterns.is_empty() {
        return Err(XrnsError::Malformed("no tracks or no patterns found"));
    }
    if song.sequence.is_empty() {
        song.sequence.push(0);
    }
    Ok(song)
}

fn read_pattern_track(pat: &mut Pattern, ti: usize, pt: Node) {
    let Some(lines) = child(pt, "Lines") else { return };
    for l in lines.children().filter(|c| c.has_tag_name("Line")) {
        let Some(idx) = l.attribute("index").and_then(|s| s.parse::<u16>().ok()) else { continue };
        let line = pat.line_mut(ti, idx);
        if let Some(cols) = child(l, "NoteColumns") {
            for (ci, c) in cols.children().filter(|c| c.has_tag_name("NoteColumn")).enumerate() {
                if ci >= MAX_NOTE_COLUMNS {
                    break;
                }
                let col = &mut line.notes[ci];
                if let Some(n) = text(c, "Note").and_then(parse_note) {
                    col.note = n;
                }
                col.instrument = hex(c, "Instrument").unwrap_or(EMPTY);
                col.volume = hex(c, "Volume").unwrap_or(EMPTY); // TODO: special volume-column chars
                col.panning = hex(c, "Panning").unwrap_or(EMPTY);
                col.delay = hex(c, "Delay").unwrap_or(0);
                col.effect = effect(c, "EffectNumber", "EffectValue");
            }
        }
        if let Some(cols) = child(l, "EffectColumns") {
            for (ci, c) in cols.children().filter(|c| c.has_tag_name("EffectColumn")).enumerate() {
                if ci >= MAX_EFFECT_COLUMNS {
                    break;
                }
                line.effects[ci] = effect(c, "Number", "Value");
            }
        }
    }
}

fn effect(n: Node, id: &str, val: &str) -> EffectCmd {
    match text(n, id).map(str::as_bytes) {
        Some([a, b]) => EffectCmd { id: [*a, *b], amount: hex(n, val).unwrap_or(0) },
        _ => EffectCmd::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<RenoiseSong doc_version="1">
  <GlobalSongData><BeatsPerMin>140</BeatsPerMin><LinesPerBeat>4</LinesPerBeat><TicksPerLine>12</TicksPerLine><SongName>t</SongName></GlobalSongData>
  <Tracks><SequencerTrack><Name>Lead</Name></SequencerTrack><SequencerMasterTrack><Name>Master</Name></SequencerMasterTrack></Tracks>
  <PatternPool><Patterns><Pattern><NumberOfLines>32</NumberOfLines><Tracks>
    <PatternTrack><Lines><Line index="4"><NoteColumns><NoteColumn><Note>C-4</Note><Instrument>01</Instrument><Volume>40</Volume></NoteColumn></NoteColumns>
      <EffectColumns><EffectColumn><Number>0A</Number><Value>37</Value></EffectColumn></EffectColumns></Line></Lines></PatternTrack>
  </Tracks></Pattern></Patterns></PatternPool>
  <PatternSequence><SequenceEntries><SequenceEntry><Pattern>0</Pattern></SequenceEntry></SequenceEntries></PatternSequence>
</RenoiseSong>"#;

    #[test]
    fn imports_minimal_song() {
        let s = import_xml(SAMPLE).unwrap();
        assert_eq!(s.bpm, 140.0);
        assert_eq!(s.tracks.len(), 2);
        assert_eq!(s.patterns[0].number_of_lines, 32);
        let line = s.patterns[0].line(0, 4).unwrap();
        assert_eq!(line.notes[0].note, 48);
        assert_eq!(line.notes[0].instrument, 1);
        assert_eq!(line.notes[0].volume, 0x40);
        assert_eq!(&line.effects[0].id, b"0A");
        assert_eq!(line.effects[0].amount, 0x37);
    }
}
