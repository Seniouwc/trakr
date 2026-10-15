//! Audio file export: WAV, FLAC, Ogg Vorbis, MP3, AAC (ADTS) and Ogg Opus, with a single 0..=10 quality
//! level that every lossy codec maps onto its own scale (10 is the highest quality it offers).

use std::path::Path;
use trakr_core::sample_fx::dither;
use trakr_core::Sample;

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("{0}")]
    Codec(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("nothing to encode")]
    Empty,
}

fn codec_err(e: impl std::fmt::Display) -> EncodeError {
    EncodeError::Codec(e.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Codec {
    Wav,
    Flac,
    Vorbis,
    Mp3,
    Aac,
    Opus,
}

impl Codec {
    pub const ALL: [Codec; 6] = [Codec::Wav, Codec::Flac, Codec::Vorbis, Codec::Mp3, Codec::Aac, Codec::Opus];

    pub fn name(self) -> &'static str {
        match self {
            Codec::Wav => "WAV",
            Codec::Flac => "FLAC",
            Codec::Vorbis => "Ogg Vorbis",
            Codec::Mp3 => "MP3",
            Codec::Aac => "AAC",
            Codec::Opus => "Opus",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Codec::Wav => "wav",
            Codec::Flac => "flac",
            Codec::Vorbis => "ogg",
            Codec::Mp3 => "mp3",
            Codec::Aac => "aac",
            Codec::Opus => "opus",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Codec> {
        let ext = ext.to_ascii_lowercase();
        Codec::ALL.into_iter().find(|c| c.extension() == ext || (*c == Codec::Vorbis && ext == "oga"))
    }

    pub fn lossy(self) -> bool {
        matches!(self, Codec::Vorbis | Codec::Mp3 | Codec::Aac | Codec::Opus)
    }

    /// Whether the quality level has an effect (WAV and FLAC are lossless).
    pub fn uses_quality(self) -> bool {
        self.lossy()
    }

    /// Whether the bit depth setting has an effect.
    pub fn uses_bit_depth(self) -> bool {
        matches!(self, Codec::Wav | Codec::Flac)
    }
}

pub const MAX_QUALITY: u8 = 10;

const MP3_KBPS: [u32; 11] = [64, 80, 96, 112, 128, 160, 192, 224, 256, 0, 320]; // 0 = VBR V0
const OPUS_KBPS: [u32; 11] = [24, 32, 48, 64, 80, 96, 128, 160, 192, 256, 510];
const AAC_KBPS: [u32; 11] = [64, 96, 112, 128, 160, 192, 224, 256, 320, 384, 512];

/// Everything the export dialog chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExportFormat {
    pub codec: Codec,
    /// 0..=10 for lossy codecs; 10 is the best the codec offers.
    pub quality: u8,
    /// 16, 24 or 32 (32 = float, WAV only). Used by WAV and FLAC (FLAC caps at 24).
    pub bits: u8,
    /// Triangular dither when reducing to 16 or 24 bits.
    pub dither: bool,
    /// Output sample rate; `None` keeps the source rate (lossy codecs pick the nearest rate they support).
    pub sample_rate: Option<u32>,
}

impl Default for ExportFormat {
    fn default() -> Self {
        ExportFormat { codec: Codec::Wav, quality: 8, bits: 16, dither: true, sample_rate: None }
    }
}

impl ExportFormat {
    pub fn wav(bits: u8) -> Self {
        ExportFormat { codec: Codec::Wav, bits, ..Default::default() }
    }

    pub fn extension(&self) -> &'static str {
        self.codec.extension()
    }

    /// Human readable description of the current settings, e.g. "MP3 320 kbps".
    pub fn describe(&self) -> String {
        let q = self.quality.min(MAX_QUALITY) as usize;
        match self.codec {
            Codec::Wav => match self.bits {
                32 => "WAV 32-bit float".into(),
                b => format!("WAV {b}-bit"),
            },
            Codec::Flac => format!("FLAC {}-bit lossless", self.bits.min(24)),
            Codec::Vorbis => {
                format!("Ogg Vorbis quality {:.2} (about {} kbps)", vorbis_quality(q as u8), vorbis_kbps(q))
            }
            Codec::Mp3 if MP3_KBPS[q] == 0 => "MP3 VBR V0 (about 245 kbps)".into(),
            Codec::Mp3 => format!("MP3 {} kbps", MP3_KBPS[q]),
            Codec::Aac => format!("AAC {} kbps", AAC_KBPS[q]),
            Codec::Opus => format!("Opus {} kbps", OPUS_KBPS[q]),
        }
    }

    /// Write frames `a..b` of `s` to `path`. The path's extension is replaced by the codec's.
    pub fn write(&self, path: &Path, s: &Sample, a: usize, b: usize) -> Result<(), EncodeError> {
        let bytes = self.encode(s, a, b)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Encode frames `a..b` of `s` into a complete file image.
    pub fn encode(&self, s: &Sample, a: usize, b: usize) -> Result<Vec<u8>, EncodeError> {
        let b = b.min(s.frames());
        let a = a.min(b);
        if a == b {
            return Err(EncodeError::Empty);
        }
        let channels = (s.channels.clamp(1, 2)) as usize;
        let mut part = Sample::new(s.name.clone(), s.sample_rate.max(1), channels as u8, s.copy_range(a, b));
        if s.channels > 2 {
            // keep the first two channels
            let src = s.copy_range(a, b);
            let n = s.channels as usize;
            part.channels = 2;
            part.data = std::sync::Arc::new(src.chunks(n).flat_map(|f| [f[0], f[1]]).collect());
        }
        let target = self.target_rate(part.sample_rate);
        if target != part.sample_rate {
            part.resample_to(target);
        }
        let q = self.quality.min(MAX_QUALITY);
        let mut pcm: Vec<f32> =
            part.data.iter().map(|v| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }).collect();
        match self.codec {
            Codec::Wav => {
                let fmt = match self.bits {
                    32 => trakr_sample::WavFormat::Float32,
                    24 => trakr_sample::WavFormat::Pcm24,
                    _ => trakr_sample::WavFormat::Pcm16,
                };
                if self.dither && self.bits < 32 {
                    dither(&mut pcm, channels, self.bits as u32, false);
                }
                part.data = std::sync::Arc::new(pcm);
                Ok(trakr_sample::encode_wav(&part, 0, part.frames(), fmt))
            }
            Codec::Flac => {
                let bits = if self.bits >= 24 { 24 } else { 16 };
                if self.dither {
                    dither(&mut pcm, channels, bits, false);
                }
                encode_flac(&pcm, channels, bits, part.sample_rate)
            }
            Codec::Vorbis => encode_vorbis(&pcm, channels, part.sample_rate, q),
            Codec::Mp3 => encode_mp3(&pcm, channels, part.sample_rate, q),
            Codec::Aac => encode_aac(&pcm, channels, part.sample_rate, q),
            Codec::Opus => encode_opus(&pcm, channels, part.sample_rate, q),
        }
    }

    /// The rate the file will actually have for a source at `src`.
    pub fn target_rate(&self, src: u32) -> u32 {
        let want = self.sample_rate.unwrap_or(src).clamp(8000, 192_000);
        match self.codec {
            Codec::Wav | Codec::Flac | Codec::Vorbis => want,
            Codec::Opus => nearest(want, &[8000, 12000, 16000, 24000, 48000]),
            Codec::Mp3 => nearest(want, &[8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000]),
            Codec::Aac => nearest(
                want,
                &[8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000],
            ),
        }
    }
}

fn nearest(v: u32, options: &[u32]) -> u32 {
    // prefer the next rate up so that nothing is thrown away; fall back to the highest
    options.iter().copied().find(|&o| o >= v).unwrap_or(*options.last().unwrap())
}

fn to_i16(pcm: &[f32]) -> Vec<i16> {
    pcm.iter().map(|v| (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16).collect()
}

// ---- FLAC ------------------------------------------------------------------------------------

fn encode_flac(pcm: &[f32], channels: usize, bits: u32, rate: u32) -> Result<Vec<u8>, EncodeError> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let scale = (1i64 << (bits - 1)) as f32;
    let max = scale - 1.0;
    let samples: Vec<i32> = pcm.iter().map(|v| (v * scale).round().clamp(-scale, max) as i32).collect();
    // The library pads a short last block with silence, which would change the length. Pick a block size
    // that divides the length exactly; only a long prime length falls back to padding.
    let total = samples.len() / channels;
    let (block, samples) = match flac_block_size(total) {
        Some(b) => (b, samples),
        None => {
            let mut padded = samples;
            padded.resize(total.div_ceil(4096) * 4096 * channels, 0);
            (4096, padded)
        }
    };
    let mut config = flacenc::config::Encoder::default();
    config.block_size = block;
    let config = config.into_verified().map_err(|e| codec_err(format!("{e:?}")))?;
    let source = flacenc::source::MemSource::from_samples(&samples, channels, bits as usize, rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, block)
        .map_err(|e| codec_err(format!("{e:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| codec_err(format!("{e:?}")))?;
    Ok(sink.as_slice().to_vec())
}

// ---- Ogg Vorbis ------------------------------------------------------------------------------

fn vorbis_quality(q: u8) -> f32 {
    (-0.1 + q.min(MAX_QUALITY) as f32 * 0.11).clamp(-0.2, 1.0)
}

fn vorbis_kbps(q: usize) -> u32 {
    [45, 64, 80, 96, 112, 128, 160, 192, 224, 256, 500][q.min(10)]
}

fn encode_vorbis(pcm: &[f32], channels: usize, rate: u32, q: u8) -> Result<Vec<u8>, EncodeError> {
    use std::num::{NonZeroU32, NonZeroU8};
    use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};
    let mut out = Vec::new();
    let mut builder = VorbisEncoderBuilder::new(
        NonZeroU32::new(rate).ok_or_else(|| codec_err("zero sample rate"))?,
        NonZeroU8::new(channels as u8).ok_or_else(|| codec_err("no channels"))?,
        &mut out,
    )
    .map_err(codec_err)?;
    builder.bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
        target_quality: vorbis_quality(q),
    });
    let mut enc = builder.build().map_err(codec_err)?;
    let frames = pcm.len() / channels;
    let mut at = 0;
    while at < frames {
        let n = (frames - at).min(1024);
        let block: Vec<Vec<f32>> =
            (0..channels).map(|c| (0..n).map(|i| pcm[(at + i) * channels + c]).collect()).collect();
        enc.encode_audio_block(&block).map_err(codec_err)?;
        at += n;
    }
    enc.finish().map_err(codec_err)?;
    Ok(out)
}

// ---- MP3 -------------------------------------------------------------------------------------

fn encode_mp3(pcm: &[f32], channels: usize, rate: u32, q: u8) -> Result<Vec<u8>, EncodeError> {
    use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, InterleavedPcm, Mode, MonoPcm, Quality, VbrMode};
    let mut b = Builder::new().ok_or_else(|| codec_err("LAME could not start"))?;
    b.set_sample_rate(rate).map_err(codec_err)?;
    b.set_num_channels(channels as u8).map_err(codec_err)?;
    b.set_mode(if channels == 1 { Mode::Mono } else { Mode::JointStereo }).map_err(codec_err)?;
    b.set_quality(Quality::Best).map_err(codec_err)?;
    let kbps = MP3_KBPS[q as usize];
    if kbps == 0 {
        b.set_vbr_mode(VbrMode::Mtrh).map_err(codec_err)?;
        b.set_vbr_quality(Quality::Best).map_err(codec_err)?;
    } else {
        let rate_enum = match kbps {
            64 => Bitrate::Kbps64,
            80 => Bitrate::Kbps80,
            96 => Bitrate::Kbps96,
            112 => Bitrate::Kbps112,
            128 => Bitrate::Kbps128,
            160 => Bitrate::Kbps160,
            192 => Bitrate::Kbps192,
            224 => Bitrate::Kbps224,
            256 => Bitrate::Kbps256,
            _ => Bitrate::Kbps320,
        };
        b.set_brate(rate_enum).map_err(codec_err)?;
    }
    let mut enc = b.build().map_err(codec_err)?;
    let mut out = Vec::new();
    let mut at = 0;
    let chunk = 8192 * channels;
    while at < pcm.len() {
        let end = (at + chunk).min(pcm.len());
        let slice = &pcm[at..end];
        out.reserve(slice.len() * 5 / 4 + 7200);
        if channels == 1 {
            enc.encode_to_vec(MonoPcm(slice), &mut out).map_err(codec_err)?;
        } else {
            enc.encode_to_vec(InterleavedPcm(slice), &mut out).map_err(codec_err)?;
        }
        at = end;
    }
    out.reserve(7200);
    enc.flush_to_vec::<FlushNoGap>(&mut out).map_err(codec_err)?;
    Ok(out)
}

// ---- AAC (ADTS) ------------------------------------------------------------------------------

fn encode_aac(pcm: &[f32], channels: usize, rate: u32, q: u8) -> Result<Vec<u8>, EncodeError> {
    use fdk_aac::enc::{AudioObjectType, BitRate, ChannelMode, Encoder, EncoderParams, Transport};
    let mut kbps = AAC_KBPS[q as usize];
    let mut last_err = None;
    let mut enc = None;
    // The encoder rejects bitrates above its limit for the layout; step down until one is accepted.
    while kbps >= 32 {
        match Encoder::new(EncoderParams {
            bit_rate: BitRate::Cbr(kbps * 1000),
            sample_rate: rate,
            transport: Transport::Adts,
            channels: if channels == 1 { ChannelMode::Mono } else { ChannelMode::Stereo },
            audio_object_type: AudioObjectType::Mpeg4LowComplexity,
        }) {
            Ok(e) => {
                enc = Some(e);
                break;
            }
            Err(e) => {
                last_err = Some(e);
                kbps = kbps * 3 / 4;
            }
        }
    }
    let enc = enc.ok_or_else(|| codec_err(format!("AAC encoder: {:?}", last_err)))?;
    let ints = to_i16(pcm);
    let mut out = Vec::new();
    let mut buf = vec![0u8; 20_480];
    let frame = 1024 * channels;
    let mut at = 0;
    while at < ints.len() {
        let end = (at + frame).min(ints.len());
        let mut chunk = ints[at..end].to_vec();
        chunk.resize(frame, 0);
        let info = enc.encode(&chunk, &mut buf).map_err(|e| codec_err(format!("{e:?}")))?;
        out.extend_from_slice(&buf[..info.output_size]);
        at = end;
    }
    // flush the encoder's delay with silence until it stops producing output
    for _ in 0..2 {
        let info = enc.encode(&vec![0i16; frame], &mut buf).map_err(|e| codec_err(format!("{e:?}")))?;
        out.extend_from_slice(&buf[..info.output_size]);
    }
    Ok(out)
}

// ---- Ogg Opus --------------------------------------------------------------------------------

fn encode_opus(pcm: &[f32], channels: usize, rate: u32, q: u8) -> Result<Vec<u8>, EncodeError> {
    use ogg::writing::{PacketWriteEndInfo, PacketWriter};
    let ch = if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo };
    let mut enc = opus::Encoder::new(rate, ch, opus::Application::Audio).map_err(codec_err)?;
    enc.set_bitrate(opus::Bitrate::Bits((OPUS_KBPS[q as usize] * 1000) as i32)).map_err(codec_err)?;
    enc.set_vbr(true).map_err(codec_err)?;
    enc.set_complexity(10).map_err(codec_err)?;
    let pre_skip = enc.get_lookahead().map_err(codec_err)?.max(0) as u16;
    let serial = 0x7472_6b72; // "trkr"
    let mut out = Vec::new();
    {
        let mut w = PacketWriter::new(&mut out);
        let mut head = Vec::with_capacity(19);
        head.extend(b"OpusHead");
        head.push(1);
        head.push(channels as u8);
        head.extend(pre_skip.to_le_bytes());
        head.extend(rate.to_le_bytes());
        head.extend(0i16.to_le_bytes());
        head.push(0);
        w.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0).map_err(codec_err)?;
        let vendor = b"trakr";
        let mut tags = Vec::new();
        tags.extend(b"OpusTags");
        tags.extend((vendor.len() as u32).to_le_bytes());
        tags.extend(vendor);
        tags.extend(0u32.to_le_bytes());
        w.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0).map_err(codec_err)?;

        let frame_len = (rate / 50) as usize; // 20 ms
        let total = pcm.len() / channels;
        let packets = (total + pre_skip as usize).div_ceil(frame_len).max(1);
        // The granule position counts 48 kHz samples regardless of the input rate.
        let scale = 48_000 / rate as u64;
        let mut padded: Vec<f32> = vec![0.0; pre_skip as usize * channels];
        padded.extend_from_slice(pcm);
        padded.resize(packets * frame_len * channels, 0.0);
        for (i, chunk) in padded.chunks(frame_len * channels).enumerate() {
            let data = enc.encode_vec_float(chunk, 4000).map_err(codec_err)?;
            let last = i + 1 == packets;
            let granule = if last {
                (total as u64 + pre_skip as u64) * scale
            } else {
                (i as u64 + 1) * frame_len as u64 * scale
            };
            let end = if last { PacketWriteEndInfo::EndStream } else { PacketWriteEndInfo::NormalPacket };
            w.write_packet(data, serial, end, granule).map_err(codec_err)?;
        }
    }
    Ok(out)
}

/// A FLAC block size (16..=65535) dividing `total` exactly, as close to 4096 as possible.
fn flac_block_size(total: usize) -> Option<usize> {
    if total == 0 {
        return None;
    }
    (16..=total.min(65_535)).filter(|b| total % b == 0).min_by_key(|b| b.abs_diff(4096))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, channels: u8, seconds: f32) -> Sample {
        let n = (rate as f32 * seconds) as usize;
        let mut d = Vec::new();
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let l = (t * 440.0 * std::f32::consts::TAU).sin() * 0.5;
            d.push(l);
            if channels == 2 {
                d.push((t * 660.0 * std::f32::consts::TAU).sin() * 0.4);
            }
        }
        Sample::new("tone", rate, channels, d)
    }

    /// Decode an Ogg Opus file (symphonia has no Opus support): (interleaved samples, channels, pre-skip).
    fn decode_opus(bytes: &[u8]) -> (Vec<f32>, usize, usize) {
        let mut r = ogg::PacketReader::new(std::io::Cursor::new(bytes.to_vec()));
        let head = r.read_packet_expected().unwrap().data;
        assert_eq!(&head[..8], b"OpusHead");
        let ch = head[9] as usize;
        let pre = u16::from_le_bytes([head[10], head[11]]) as usize;
        let tags = r.read_packet_expected().unwrap().data;
        assert_eq!(&tags[..8], b"OpusTags");
        let mut dec =
            opus::Decoder::new(48_000, if ch == 1 { opus::Channels::Mono } else { opus::Channels::Stereo })
                .unwrap();
        let mut out = Vec::new();
        let mut buf = vec![0f32; 5760 * ch];
        while let Some(p) = r.read_packet().unwrap() {
            let n = dec.decode_float(&p.data, &mut buf, false).unwrap();
            out.extend_from_slice(&buf[..n * ch]);
        }
        (out, ch, pre)
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
    }

    #[test]
    fn every_codec_encodes_and_decodes() {
        let src = tone(44_100, 2, 1.0);
        for codec in Codec::ALL {
            for quality in [0u8, 5, 10] {
                let fmt = ExportFormat { codec, quality, ..Default::default() };
                let bytes =
                    fmt.encode(&src, 0, src.frames()).unwrap_or_else(|e| panic!("{codec:?} q{quality}: {e}"));
                assert!(bytes.len() > 500, "{codec:?} q{quality} produced {} bytes", bytes.len());
                if codec == Codec::Aac {
                    assert_eq!(bytes[0], 0xFF, "ADTS sync");
                    continue; // the decoder crate set has no ADTS support
                }
                if codec == Codec::Opus {
                    let (pcm, ch, pre) = decode_opus(&bytes);
                    assert_eq!(ch, 2);
                    let frames = pcm.len() / 2 - pre;
                    assert!((47_000..50_500).contains(&frames), "opus length {frames}");
                    assert!(rms(&pcm[pre * 2..]) > 0.2);
                    continue;
                }
                let back = trakr_sample::decode_bytes(bytes, Some(codec.extension()), "x")
                    .unwrap_or_else(|e| panic!("{codec:?} q{quality} decode: {e}"));
                assert_eq!(back.channels, 2, "{codec:?}");
                let secs = back.frames() as f32 / back.sample_rate as f32;
                assert!((0.9..1.2).contains(&secs), "{codec:?} length {secs}");
                assert!(rms(&back.data) > 0.2, "{codec:?} too quiet: {}", rms(&back.data));
            }
        }
    }

    #[test]
    fn lossless_codecs_roundtrip() {
        let src = tone(48_000, 2, 0.5);
        for (codec, bits) in
            [(Codec::Wav, 16u8), (Codec::Wav, 24), (Codec::Wav, 32), (Codec::Flac, 16), (Codec::Flac, 24)]
        {
            let fmt = ExportFormat { codec, bits, dither: false, ..Default::default() };
            let back = trakr_sample::decode_bytes(
                fmt.encode(&src, 0, src.frames()).unwrap(),
                Some(codec.extension()),
                "x",
            )
            .unwrap();
            assert_eq!(back.frames(), src.frames(), "{codec:?} {bits}");
            let tol = if bits == 16 { 1e-3 } else { 1e-5 };
            for (a, b) in src.data.iter().zip(back.data.iter()) {
                assert!((a - b).abs() < tol, "{codec:?} {bits}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn higher_quality_is_bigger() {
        let src = tone(44_100, 2, 1.0);
        for codec in [Codec::Mp3, Codec::Opus, Codec::Aac] {
            let size = |q| {
                ExportFormat { codec, quality: q, ..Default::default() }
                    .encode(&src, 0, src.frames())
                    .unwrap()
                    .len()
            };
            assert!(size(0) < size(10), "{codec:?}");
        }
    }

    #[test]
    fn mono_resampling_and_ranges() {
        let src = tone(22_050, 1, 1.0);
        let fmt = ExportFormat { codec: Codec::Opus, quality: 6, ..Default::default() };
        assert_eq!(fmt.target_rate(22_050), 24_000);
        let (pcm, ch, _) = decode_opus(&fmt.encode(&src, 0, src.frames()).unwrap());
        assert_eq!(ch, 1);
        assert!(pcm.len() >= 24_000);
        let half = ExportFormat::wav(16).encode(&src, 1000, 2000).unwrap();
        assert_eq!(trakr_sample::decode_bytes(half, Some("wav"), "x").unwrap().frames(), 1000);
        assert!(matches!(ExportFormat::wav(16).encode(&src, 5, 5), Err(EncodeError::Empty)));
    }

    #[test]
    fn descriptions_and_extensions() {
        assert_eq!(Codec::from_extension("FLAC"), Some(Codec::Flac));
        assert_eq!(Codec::from_extension("xyz"), None);
        let max = |codec| ExportFormat { codec, quality: 10, ..Default::default() }.describe();
        assert!(max(Codec::Mp3).contains("320"));
        assert!(max(Codec::Opus).contains("510"));
        assert!(max(Codec::Vorbis).contains("1.00"));
    }

    #[test]
    fn silence_and_extremes_do_not_fail() {
        let silent = Sample::new("s", 44_100, 2, vec![0.0; 44_100 * 2]);
        let loud = Sample::new("l", 44_100, 2, vec![5.0; 4410 * 2]);
        let tiny = Sample::new("t", 44_100, 1, vec![0.1; 10]);
        for codec in Codec::ALL {
            for s in [&silent, &loud, &tiny] {
                let fmt = ExportFormat { codec, quality: 10, ..Default::default() };
                fmt.encode(s, 0, s.frames()).unwrap_or_else(|e| panic!("{codec:?}: {e}"));
            }
        }
    }
}
