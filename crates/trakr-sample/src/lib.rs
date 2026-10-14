//! Sample file decoding (WAV, AIFF, FLAC, OGG Vorbis, MP3) into [`trakr_core::Sample`].

use std::fs::File;
use std::io::Cursor;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use trakr_core::Sample;

#[derive(Debug, thiserror::Error)]
pub enum SampleError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode: {0}")]
    Decode(#[from] SymError),
    #[error("no audio track found")]
    NoTrack,
    #[error("file contains no audio")]
    Empty,
}

/// Decode a file. Channels beyond the first two are dropped.
pub fn decode_file(path: &Path) -> Result<Sample, SampleError> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_owned);
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("sample").to_owned();
    decode(Box::new(File::open(path)?), ext.as_deref(), name)
}

/// Decode in-memory file contents; `ext` is an optional format hint such as `"wav"`.
pub fn decode_bytes(bytes: Vec<u8>, ext: Option<&str>, name: &str) -> Result<Sample, SampleError> {
    decode(Box::new(Cursor::new(bytes)), ext, name.to_owned())
}

fn decode(src: Box<dyn MediaSource>, ext: Option<&str>, name: String) -> Result<Sample, SampleError> {
    let mss = MediaSourceStream::new(src, Default::default());
    let mut hint = Hint::new();
    if let Some(e) = ext {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(SampleError::NoTrack)?;
    let id = track.id;
    let mut rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut channels = 0usize;
    let mut data: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                rate = spec.rate;
                let src_ch = spec.channels.count().max(1);
                channels = src_ch.min(2);
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
                sb.copy_interleaved_ref(buf);
                if src_ch == channels {
                    data.extend_from_slice(sb.samples());
                } else {
                    data.extend(
                        sb.samples().chunks_exact(src_ch).flat_map(|f| f[..channels].iter().copied()),
                    );
                }
            }
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    if data.is_empty() || channels == 0 {
        return Err(SampleError::Empty);
    }
    Ok(Sample::new(name, rate, channels as u8, data))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavFormat {
    Pcm16,
    Pcm24,
    Float32,
}

/// Encode frames `a..b` of `s` as a RIFF/WAVE file.
pub fn encode_wav(s: &Sample, a: usize, b: usize, format: WavFormat) -> Vec<u8> {
    let data = s.copy_range(a, b);
    let (tag, bits): (u16, u16) = match format {
        WavFormat::Pcm16 => (1, 16),
        WavFormat::Pcm24 => (1, 24),
        WavFormat::Float32 => (3, 32),
    };
    let bytes_per = (bits / 8) as usize;
    let data_len = (data.len() * bytes_per) as u32;
    let ch = s.channels as u16;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend(b"RIFF");
    v.extend((36 + data_len).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(tag.to_le_bytes());
    v.extend(ch.to_le_bytes());
    v.extend(s.sample_rate.to_le_bytes());
    v.extend((s.sample_rate * ch as u32 * bytes_per as u32).to_le_bytes());
    v.extend((ch * bytes_per as u16).to_le_bytes());
    v.extend(bits.to_le_bytes());
    v.extend(b"data");
    v.extend(data_len.to_le_bytes());
    for x in data {
        let x = x.clamp(-1.0, 1.0);
        match format {
            WavFormat::Pcm16 => v.extend(((x * 32767.0).round() as i16).to_le_bytes()),
            WavFormat::Pcm24 => v.extend(&((x * 8_388_607.0).round() as i32).to_le_bytes()[..3]),
            WavFormat::Float32 => v.extend(x.to_le_bytes()),
        }
    }
    v
}

pub fn write_wav(path: &Path, s: &Sample, a: usize, b: usize, format: WavFormat) -> Result<(), SampleError> {
    std::fs::write(path, encode_wav(s, a, b, format))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal 16-bit PCM WAV writer for fixtures.
    fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut v = Vec::new();
        v.extend(b"RIFF");
        v.extend((36 + data_len).to_le_bytes());
        v.extend(b"WAVEfmt ");
        v.extend(16u32.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(channels.to_le_bytes());
        v.extend(rate.to_le_bytes());
        v.extend((rate * channels as u32 * 2).to_le_bytes());
        v.extend((channels * 2).to_le_bytes());
        v.extend(16u16.to_le_bytes());
        v.extend(b"data");
        v.extend(data_len.to_le_bytes());
        for s in samples {
            v.extend(s.to_le_bytes());
        }
        v
    }

    #[test]
    fn decodes_mono_wav() {
        let bytes = wav(22_050, 1, &[0, 16384, -16384, 32767]);
        let s = decode_bytes(bytes, Some("wav"), "t").unwrap();
        assert_eq!((s.sample_rate, s.channels, s.frames()), (22_050, 1, 4));
        assert!((s.data[1] - 0.5).abs() < 1e-3);
        assert!((s.data[2] + 0.5).abs() < 1e-3);
    }

    #[test]
    fn decodes_stereo_wav() {
        let bytes = wav(44_100, 2, &[100, -100, 200, -200]);
        let s = decode_bytes(bytes, Some("wav"), "t").unwrap();
        assert_eq!((s.channels, s.frames()), (2, 2));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode_bytes(vec![1, 2, 3, 4, 5, 6, 7, 8], None, "x").is_err());
    }

    #[test]
    fn wav_roundtrip_all_formats() {
        let src = Sample::new("t", 32_000, 2, vec![0.0, 0.5, -0.5, 1.0, 0.25, -1.0]);
        for fmt in [WavFormat::Pcm16, WavFormat::Pcm24, WavFormat::Float32] {
            let back = decode_bytes(encode_wav(&src, 0, 3, fmt), Some("wav"), "x").unwrap();
            assert_eq!((back.sample_rate, back.channels, back.frames()), (32_000, 2, 3), "{fmt:?}");
            for (a, b) in src.data.iter().zip(back.data.iter()) {
                assert!((a - b).abs() < 1e-3, "{fmt:?}: {a} vs {b}");
            }
        }
        // sub-range export
        let part = decode_bytes(encode_wav(&src, 1, 2, WavFormat::Pcm16), Some("wav"), "x").unwrap();
        assert_eq!(part.frames(), 1);
    }
}
