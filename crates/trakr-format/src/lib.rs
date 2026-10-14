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
}
