//! Best-effort importer for Renoise `.xrns` projects (zip + `Song.xml`).
//!
//! The element names below come from memory of the format, NOT from a spec.
//! First job for whoever continues this: run it against real `.xrns` files,
//! fix whatever differs, and add them (if licensing allows) as test fixtures.
//!
//! Imported so far: song metadata, tempo, track list, pattern lengths, note
//! columns, effect columns, pattern sequence, instruments with their samples
//! (name, volume, pan, transpose/finetune, loop mode and points, key mapping, audio
//! decoded from `SampleData/...`). Not yet imported: envelopes, devices/automation,
//! phrases, groups, send routing. Anything skipped is reported as a warning.

use roxmltree::{Document, Node};
use std::io::{Read, Seek};
use thiserror::Error;
use trakr_core::note::parse_note;
use trakr_core::{
    EffectCmd, Instrument, LoopMode, Pattern, Song, Track, TrackKind, EMPTY, MAX_EFFECT_COLUMNS,
    MAX_NOTE_COLUMNS,
};

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

/// Import a project, including sample audio. Skipped or undecodable parts are listed in the warnings.
pub fn import_with_report<R: Read + Seek>(r: R) -> Result<(Song, Vec<String>), XrnsError> {
    let mut zip = zip::ZipArchive::new(r)?;
    let mut xml = String::new();
    zip.by_name("Song.xml")?.read_to_string(&mut xml)?;
    let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
    let mut warnings = Vec::new();
    let song = import_xml_with(
        &xml,
        &mut |inst, sample, file_name| {
            let entry = find_sample_entry(&names, inst, sample, file_name)?;
            let mut bytes = Vec::new();
            zip.by_name(&entry).ok()?.read_to_end(&mut bytes).ok()?;
            let ext = entry.rsplit('.').next().map(str::to_ascii_lowercase);
            Some((bytes, ext))
        },
        &mut warnings,
    )?;
    Ok((song, warnings))
}

pub fn import<R: Read + Seek>(r: R) -> Result<Song, XrnsError> {
    import_with_report(r).map(|(s, _)| s)
}

pub fn import_path(path: &std::path::Path) -> Result<Song, XrnsError> {
    import(std::fs::File::open(path)?)
}

pub fn import_path_with_report(path: &std::path::Path) -> Result<(Song, Vec<String>), XrnsError> {
    import_with_report(std::fs::File::open(path)?)
}

/// Find a sample's audio file in the archive: by `SampleData/Instrument<ii>*/Sample<ss>*` numbering,
/// falling back to the file name given in the XML.
fn find_sample_entry(
    names: &[String],
    inst: usize,
    sample: usize,
    file_name: Option<&str>,
) -> Option<String> {
    let inst_prefix = format!("SampleData/Instrument{inst:02}");
    let sample_tag = format!("/Sample{sample:02}");
    let by_number =
        names.iter().find(|n| n.starts_with(&inst_prefix) && n[inst_prefix.len()..].contains(&sample_tag));
    if let Some(n) = by_number {
        return Some(n.clone());
    }
    let wanted = file_name?.rsplit(['/', '\\']).next()?;
    names.iter().find(|n| n.starts_with("SampleData/") && n.ends_with(wanted)).cloned()
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

/// Import `Song.xml` text only (no sample audio is available, so samples are left empty).
pub fn import_xml(xml: &str) -> Result<Song, XrnsError> {
    import_xml_with(xml, &mut |_, _, _| None, &mut Vec::new())
}

/// Supplies a sample's encoded bytes and extension given (instrument index, sample index, XML file name).
type SampleSource<'a> = dyn FnMut(usize, usize, Option<&str>) -> Option<(Vec<u8>, Option<String>)> + 'a;

fn import_xml_with(
    xml: &str,
    samples: &mut SampleSource,
    warnings: &mut Vec<String>,
) -> Result<Song, XrnsError> {
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

    if let Some(instruments) = child(root, "Instruments") {
        for (ii, inst) in instruments.children().filter(|c| c.has_tag_name("Instrument")).enumerate() {
            song.instruments.push(read_instrument(ii, inst, samples, warnings));
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
            song.patterns.push(std::sync::Arc::new(pat));
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

fn float(n: Node, name: &str) -> Option<f32> {
    text(n, name)?.parse().ok()
}

fn read_instrument(
    ii: usize,
    inst: Node,
    samples: &mut SampleSource,
    warnings: &mut Vec<String>,
) -> Instrument {
    let mut out = Instrument::new(text(inst, "Name").unwrap_or("Instrument"));
    if let Some(v) = float(inst, "Volume") {
        out.volume = v;
    }
    // Samples live under SampleGenerator/Samples; some versions put them directly under the instrument.
    let list =
        child(inst, "SampleGenerator").and_then(|g| child(g, "Samples")).or_else(|| child(inst, "Samples"));
    let Some(list) = list else {
        if child(inst, "PluginProperties").is_some() || child(inst, "MidiProperties").is_some() {
            warnings.push(format!("instrument {ii:02}: plugin/MIDI instruments are not supported"));
        }
        return out;
    };
    for (si, s) in list.children().filter(|c| c.has_tag_name("Sample")).enumerate() {
        let name = text(s, "Name").unwrap_or("Sample").to_string();
        let Some((bytes, ext)) = samples(ii, si, text(s, "FileName")) else {
            warnings.push(format!("instrument {ii:02} sample {si:02} ({name}): audio file not found"));
            continue;
        };
        let mut sample = match trakr_sample::decode_bytes(bytes, ext.as_deref(), &name) {
            Ok(s) => s,
            Err(e) => {
                warnings.push(format!("instrument {ii:02} sample {si:02} ({name}): {e}"));
                continue;
            }
        };
        sample.name = name;
        if let Some(v) = float(s, "Volume") {
            sample.volume = v;
        }
        if let Some(p) = float(s, "Panning") {
            sample.panning = ((p - 0.5) * 2.0).clamp(-1.0, 1.0); // 0..1 with 0.5 in the middle
        }
        let transpose = float(s, "Transpose").unwrap_or(0.0).round() as i32;
        sample.fine_tune = float(s, "Finetune").unwrap_or(0.0).round().clamp(-128.0, 127.0) as i8;
        let frames = sample.frames();
        sample.loop_mode = match text(s, "LoopMode") {
            Some("Forward") => LoopMode::Forward,
            Some("Reverse") => LoopMode::Reverse,
            Some("PingPong") => LoopMode::PingPong,
            _ => LoopMode::Off,
        };
        sample.loop_start = float(s, "LoopStart").map_or(0, |v| v.max(0.0) as usize).min(frames);
        sample.loop_end =
            float(s, "LoopEnd").map_or(frames, |v| v.max(0.0) as usize).clamp(sample.loop_start, frames);
        let map = child(s, "Mapping").unwrap_or(s);
        let base = text(map, "BaseNote").and_then(|v| v.parse::<i32>().ok()).unwrap_or(48);
        // Transposing up makes the sample sound higher at the same key: same as a lower base note.
        sample.base_note = (base - transpose).clamp(0, 119) as u8;
        let lo = text(map, "NoteStart").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0).clamp(0, 119) as u8;
        let hi = text(map, "NoteEnd").and_then(|v| v.parse::<i32>().ok()).unwrap_or(119).clamp(0, 119) as u8;
        sample.key_range = (lo.min(hi), hi.max(lo));
        out.samples.push(sample);
    }
    out
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

#[cfg(test)]
mod import_tests {
    use super::*;
    use std::io::{Cursor, Write};
    use trakr_core::Sample;

    fn tone_wav() -> Vec<u8> {
        let data: Vec<f32> = (0..2000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let s = Sample::new("t", 22_050, 1, data);
        trakr_sample::encode_wav(&s, 0, s.frames(), trakr_sample::WavFormat::Pcm16)
    }

    fn project(samples_xml: &str, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let xml = format!(
            r#"<?xml version="1.0"?><RenoiseSong>
<GlobalSongData><BeatsPerMin>120</BeatsPerMin></GlobalSongData>
<Instruments><Instrument><Name>Lead</Name><Volume>0.8</Volume><SampleGenerator><Samples>{samples_xml}</Samples></SampleGenerator></Instrument>
<Instrument><Name>Empty</Name></Instrument></Instruments>
<Tracks><SequencerTrack><Name>T</Name></SequencerTrack><SequencerMasterTrack><Name>Master</Name></SequencerMasterTrack></Tracks>
<PatternPool><Patterns><Pattern><NumberOfLines>16</NumberOfLines><Tracks><PatternTrack><Lines/></PatternTrack></Tracks></Pattern></Patterns></PatternPool>
</RenoiseSong>"#
        );
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("Song.xml", o).unwrap();
            z.write_all(xml.as_bytes()).unwrap();
            for (name, bytes) in files {
                z.start_file(*name, o).unwrap();
                z.write_all(bytes).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn imports_instruments_with_sample_audio() {
        let sample = r#"<Sample><Name>Tone</Name><FileName>tone.wav</FileName><Volume>0.5</Volume><Panning>1.0</Panning>
<Transpose>2</Transpose><Finetune>-10</Finetune><LoopMode>PingPong</LoopMode><LoopStart>100</LoopStart><LoopEnd>900</LoopEnd>
<Mapping><BaseNote>48</BaseNote><NoteStart>36</NoteStart><NoteEnd>72</NoteEnd></Mapping></Sample>"#;
        let zip = project(sample, &[("SampleData/Instrument00 (Lead)/Sample00 (Tone).wav", tone_wav())]);
        let (song, warnings) = import_with_report(Cursor::new(zip)).unwrap();
        assert_eq!(warnings.len(), 0, "{warnings:?}");
        assert_eq!(song.instruments.len(), 2);
        let inst = &song.instruments[0];
        assert_eq!((inst.name.as_str(), inst.volume), ("Lead", 0.8));
        let s = &inst.samples[0];
        assert_eq!((s.sample_rate, s.channels, s.frames()), (22_050, 1, 2000));
        assert_eq!((s.volume, s.panning, s.fine_tune), (0.5, 1.0, -10));
        assert_eq!(s.base_note, 46, "transpose +2 lowers the base note");
        assert_eq!((s.loop_mode, s.loop_start, s.loop_end), (LoopMode::PingPong, 100, 900));
        assert_eq!(s.key_range, (36, 72));
        assert!(s.peak(0, s.frames()) > 0.4);
        assert!(song.instruments[1].samples.is_empty());
    }

    #[test]
    fn missing_or_broken_audio_becomes_a_warning_not_a_failure() {
        let sample = "<Sample><Name>Gone</Name><FileName>gone.flac</FileName></Sample><Sample><Name>Bad</Name></Sample>";
        let zip = project(sample, &[("SampleData/Instrument00 (Lead)/Sample01 (Bad).wav", vec![1, 2, 3, 4])]);
        let (song, warnings) = import_with_report(Cursor::new(zip)).unwrap();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(song.instruments[0].samples.is_empty());
        assert_eq!(song.patterns[0].number_of_lines, 16, "the rest of the song still imports");
    }

    #[test]
    fn falls_back_to_the_file_name() {
        let sample = "<Sample><Name>Tone</Name><FileName>C:\\samples\\tone.wav</FileName></Sample>";
        let zip = project(sample, &[("SampleData/Other/tone.wav", tone_wav())]);
        let (song, warnings) = import_with_report(Cursor::new(zip)).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(song.instruments[0].samples.len(), 1);
    }

    #[test]
    fn imported_instruments_survive_a_trkr_roundtrip() {
        // (The format crate is not a dependency here; check the model is serde-clean instead.)
        let sample = "<Sample><Name>Tone</Name></Sample>";
        let zip = project(sample, &[("SampleData/Instrument00 (Lead)/Sample00 (Tone).wav", tone_wav())]);
        let (song, _) = import_with_report(Cursor::new(zip)).unwrap();
        let json = serde_json::to_string(&song).unwrap();
        let back: Song = serde_json::from_str(&json).unwrap();
        assert_eq!(back.instruments[0].samples[0].name, "Tone");
    }

    #[test]
    fn mangled_xml_never_panics() {
        let base = r#"<RenoiseSong><GlobalSongData><BeatsPerMin>140</BeatsPerMin></GlobalSongData>
<Instruments><Instrument><Name>A</Name><SampleGenerator><Samples><Sample><Name>s</Name><LoopStart>9999999999</LoopStart><Transpose>-9999</Transpose><Panning>nan</Panning></Sample></Samples></SampleGenerator></Instrument></Instruments>
<Tracks><SequencerTrack><Name>T</Name><NumberOfVisibleNoteColumns>999</NumberOfVisibleNoteColumns></SequencerTrack><SequencerMasterTrack/></Tracks>
<PatternPool><Patterns><Pattern><NumberOfLines>99999</NumberOfLines><Tracks><PatternTrack><Lines><Line index="70000"><NoteColumns><NoteColumn><Note>Z-9</Note><Instrument>GG</Instrument></NoteColumn></NoteColumns></Line><Line index="x"/></Lines></PatternTrack></Tracks></Pattern></Patterns></PatternPool>
<PatternSequence><SequenceEntries><SequenceEntry><Pattern>77</Pattern></SequenceEntry></SequenceEntries></PatternSequence></RenoiseSong>"#;
        // hostile values in a well-formed document
        if let Ok(song) = import_xml(base) {
            assert!(song
                .patterns
                .iter()
                .all(|p| p.number_of_lines >= 1 && p.number_of_lines <= trakr_core::MAX_LINES));
        }
        let mut seed = 1u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed >> 8
        };
        let bytes = base.as_bytes();
        for _ in 0..2000 {
            let mut b = bytes.to_vec();
            for _ in 0..1 + rnd() % 6 {
                let i = rnd() as usize % b.len();
                b[i] = b" <>/=\"0aZ-9\n"[rnd() as usize % 12];
            }
            if let Ok(text) = String::from_utf8(b) {
                let _ = import_xml(&text);
            }
        }
    }
}
