//! Native trakr project format (`.trkr`).
//!
//! A zip container:
//! - `song.json`: the full `Song` (version-tagged, human-diffable)
//! - `samples/{instrument}_{sample}.f32`: raw little-endian interleaved f32 PCM
//!
//! TODO: compress sample data (FLAC), embed plugin state blobs, add migrations
//! keyed on `FORMAT_VERSION`.

use std::io::{Read, Seek, Write};
use std::sync::Arc;
use thiserror::Error;
use trakr_core::Song;
use zip::write::SimpleFileOptions;

pub const FORMAT_VERSION: u32 = 1;
pub const EXTENSION: &str = "trkr";

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported format version {0} (this build supports up to {FORMAT_VERSION})")]
    Version(u32),
    #[error("malformed project: {0}")]
    Malformed(String),
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Envelope {
    format_version: u32,
    song: Song,
}

fn sample_path(inst: usize, sample: usize) -> String {
    format!("samples/{inst}_{sample}.f32")
}

pub fn save<W: Write + Seek>(song: &Song, w: W) -> Result<(), FormatError> {
    let mut zip = zip::ZipWriter::new(w);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("song.json", opts)?;
    serde_json::to_writer_pretty(&mut zip, &Envelope { format_version: FORMAT_VERSION, song: song.clone() })?;
    for (i, inst) in song.instruments.iter().enumerate() {
        for (s, sample) in inst.samples.iter().enumerate() {
            zip.start_file(sample_path(i, s), opts)?;
            let mut bytes = Vec::with_capacity(sample.data.len() * 4);
            for v in sample.data.iter() {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            zip.write_all(&bytes)?;
        }
    }
    zip.finish()?;
    Ok(())
}

pub fn load<R: Read + Seek>(r: R) -> Result<Song, FormatError> {
    let mut zip = zip::ZipArchive::new(r)?;
    let env: Envelope = serde_json::from_reader(zip.by_name("song.json")?)?;
    if env.format_version > FORMAT_VERSION {
        return Err(FormatError::Version(env.format_version));
    }
    let mut song = env.song;
    for (i, inst) in song.instruments.iter_mut().enumerate() {
        for (s, sample) in inst.samples.iter_mut().enumerate() {
            let mut bytes = Vec::new();
            zip.by_name(&sample_path(i, s))
                .map_err(|e| FormatError::Malformed(format!("missing sample {i}/{s}: {e}")))?
                .read_to_end(&mut bytes)?;
            if bytes.len() % 4 != 0 {
                return Err(FormatError::Malformed(format!("sample {i}/{s} has a ragged length")));
            }
            sample.data = Arc::new(
                bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
            );
        }
    }
    Ok(song)
}

pub const INSTRUMENT_EXTENSION: &str = "trki";

#[derive(serde::Serialize, serde::Deserialize)]
struct InstrumentEnvelope {
    format_version: u32,
    instrument: trakr_core::Instrument,
}

/// Write one instrument (samples included) as a `.trki` file, for an instrument bank.
pub fn save_instrument<W: Write + Seek>(inst: &trakr_core::Instrument, w: W) -> Result<(), FormatError> {
    let mut zip = zip::ZipWriter::new(w);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("instrument.json", opts)?;
    serde_json::to_writer_pretty(
        &mut zip,
        &InstrumentEnvelope { format_version: FORMAT_VERSION, instrument: inst.clone() },
    )?;
    for (s, sample) in inst.samples.iter().enumerate() {
        zip.start_file(format!("samples/{s}.f32"), opts)?;
        let mut bytes = Vec::with_capacity(sample.data.len() * 4);
        for v in sample.data.iter() {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        zip.write_all(&bytes)?;
    }
    zip.finish()?;
    Ok(())
}

pub fn load_instrument<R: Read + Seek>(r: R) -> Result<trakr_core::Instrument, FormatError> {
    let mut zip = zip::ZipArchive::new(r)?;
    let env: InstrumentEnvelope = serde_json::from_reader(zip.by_name("instrument.json")?)?;
    if env.format_version > FORMAT_VERSION {
        return Err(FormatError::Version(env.format_version));
    }
    let mut inst = env.instrument;
    for (s, sample) in inst.samples.iter_mut().enumerate() {
        let mut bytes = Vec::new();
        zip.by_name(&format!("samples/{s}.f32"))
            .map_err(|e| FormatError::Malformed(format!("missing sample {s}: {e}")))?
            .read_to_end(&mut bytes)?;
        if bytes.len() % 4 != 0 {
            return Err(FormatError::Malformed(format!("sample {s} has a ragged length")));
        }
        sample.data =
            Arc::new(bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect());
    }
    Ok(inst)
}

pub fn save_instrument_path(
    inst: &trakr_core::Instrument,
    path: &std::path::Path,
) -> Result<(), FormatError> {
    save_instrument(inst, std::fs::File::create(path)?)
}

pub fn load_instrument_path(path: &std::path::Path) -> Result<trakr_core::Instrument, FormatError> {
    load_instrument(std::fs::File::open(path)?)
}

pub fn save_path(song: &Song, path: &std::path::Path) -> Result<(), FormatError> {
    save(song, std::fs::File::create(path)?)
}

pub fn load_path(path: &std::path::Path) -> Result<Song, FormatError> {
    load(std::fs::File::open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn phrases_survive_a_roundtrip() {
        use trakr_core::{Phrase, PhraseMode};
        let mut song = Song::demo();
        let mut p = Phrase::new("arp", 8);
        p.line_mut(2).notes[0].note = 50;
        p.base_note = 40;
        p.key_range = (30, 60);
        p.looping = false;
        song.instruments[0].phrases.push(std::sync::Arc::new(p));
        song.instruments[0].phrase_mode = PhraseMode::Keymap;
        let mut buf = Cursor::new(Vec::new());
        save(&song, &mut buf).unwrap();
        buf.set_position(0);
        let back = load(buf).unwrap();
        assert_eq!(back.instruments[0].phrases, song.instruments[0].phrases);
        assert_eq!(back.instruments[0].phrase_mode, PhraseMode::Keymap);
    }

    #[test]
    fn roundtrip_demo() {
        let song = Song::demo();
        let mut buf = Cursor::new(Vec::new());
        save(&song, &mut buf).unwrap();
        buf.set_position(0);
        let back = load(buf).unwrap();
        assert_eq!(back.patterns, song.patterns);
        assert_eq!(back.sequence, song.sequence);
        assert_eq!(back.bpm, song.bpm);
        assert_eq!(back.instruments[0].samples[0].data, song.instruments[0].samples[0].data);
    }

    #[test]
    fn instruments_roundtrip_through_a_trki_file() {
        use trakr_core::{Instrument, Sample};
        let mut inst = Instrument::new("Lead");
        inst.drive = 0.4;
        inst.volume = 0.7;
        let mut s = Sample::new("saw", 22_050, 2, vec![0.1, -0.2, 0.3, -0.4]);
        s.loop_release = true;
        inst.samples.push(s);
        inst.samples.push(Sample::new("empty-ish", 8000, 1, vec![0.5]));
        let mut buf = Cursor::new(Vec::new());
        save_instrument(&inst, &mut buf).unwrap();
        let back = load_instrument(Cursor::new(buf.into_inner())).unwrap();
        assert_eq!((back.name.as_str(), back.volume, back.drive), ("Lead", 0.7, 0.4));
        assert_eq!(back.samples.len(), 2);
        assert_eq!(*back.samples[0].data, vec![0.1, -0.2, 0.3, -0.4]);
        assert!(back.samples[0].loop_release);
        assert_eq!(back.samples[0].sample_rate, 22_050);
        assert!(load_instrument(Cursor::new(b"not a zip".to_vec())).is_err());
        // a song file is not an instrument file
        let mut song_buf = Cursor::new(Vec::new());
        save(&Song::demo(), &mut song_buf).unwrap();
        assert!(load_instrument(Cursor::new(song_buf.into_inner())).is_err());
    }
}
