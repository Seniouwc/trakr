//! Recording from an audio input, rendering the song, and sample-level editing with a clipboard.

use super::*;
use std::path::Path;
use trakr_core::Sample;
use trakr_encode::ExportFormat;

const REC_SAMPLE_CAP_SECONDS: usize = 600;
const RENDER_RATE: u32 = 44_100;
const RENDER_MAX_SECONDS: f32 = 3600.0;

impl Editor {
    // ---- selection -----------------------------------------------------------

    /// Index of the selected sample within the selected instrument, clamped to what exists.
    pub fn sample_index(&self) -> usize {
        let n = self.current_instrument().map_or(0, |i| i.samples.len());
        self.sample.min(n.saturating_sub(1))
    }

    pub fn current_sample(&self) -> Option<&Sample> {
        self.current_instrument()?.samples.get(self.sample_index())
    }

    pub fn select_sample(&mut self, i: usize) {
        self.sample = i;
    }

    // ---- importing ----------------------------------------------------------------

    /// Decode a file and add it as another sample of the selected instrument.
    pub fn import_sample_into_instrument(&mut self, path: &Path) {
        if self.current_instrument().is_none() {
            return self.import_sample(path);
        }
        match trakr_sample::decode_file(path) {
            Ok(sample) => {
                self.checkpoint();
                let idx = self.instrument as usize;
                let name = sample.name.clone();
                let inst = &mut self.song_mut().instruments[idx];
                inst.samples.push(sample);
                self.sample = inst.samples.len() - 1;
                self.status = format!("added sample {name}");
                self.sync();
            }
            Err(e) => self.status = format!("import failed: {e}"),
        }
    }

    pub fn sample_remove(&mut self, index: usize) {
        let idx = self.instrument as usize;
        if self.song.instruments.get(idx).and_then(|i| i.samples.get(index)).is_none() {
            return;
        }
        self.checkpoint();
        self.song_mut().instruments[idx].samples.remove(index);
        self.sample = self.sample.saturating_sub(1);
        self.sync();
    }

    // ---- clipboard and structural edits (frames a..b of the selected sample) ----------

    pub fn sample_copy(&mut self, a: usize, b: usize) {
        if let Some(s) = self.current_sample() {
            let (a, b) = s.clamp_range(a, b);
            if a < b {
                self.sample_clip = Some((s.copy_range(a, b), s.channels));
                self.status = format!("copied {} frames", b - a);
            }
        }
    }

    pub fn sample_cut(&mut self, a: usize, b: usize) {
        self.sample_copy(a, b);
        self.sample_delete(a, b);
    }

    pub fn sample_delete(&mut self, a: usize, b: usize) {
        let i = self.sample_index();
        self.edit_sample(i, |s| s.delete(a, b));
    }

    /// Keep only `a..b`.
    pub fn sample_crop(&mut self, a: usize, b: usize) {
        let i = self.sample_index();
        self.edit_sample(i, |s| s.trim(a, b));
    }

    /// Insert the sample clipboard at frame `at`, converting between mono and stereo as needed.
    pub fn sample_paste(&mut self, at: usize) {
        let Some((clip, clip_ch)) = self.sample_clip.clone() else {
            self.status = "sample clipboard is empty".into();
            return;
        };
        let i = self.sample_index();
        self.edit_sample(i, |s| {
            let data: Vec<f32> = match (clip_ch, s.channels) {
                (2, 1) => clip.chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect(),
                (1, 2) => clip.iter().flat_map(|&v| [v, v]).collect(),
                _ => clip,
            };
            s.insert(at, &data);
        });
    }

    /// Replace the selected sample with one sample per slice, mapped to consecutive keys from C-4.
    pub fn sample_split_slices(&mut self) {
        let Some(s) = self.current_sample() else { return };
        if s.slices.is_empty() {
            self.status = "add slice markers first".into();
            return;
        }
        let parts = s.split_at_slices(48);
        let (idx, at) = (self.instrument as usize, self.sample_index());
        self.checkpoint();
        let inst = &mut self.song_mut().instruments[idx];
        inst.samples.remove(at);
        for (k, p) in parts.into_iter().enumerate() {
            inst.samples.insert(at + k, p);
        }
        self.status = "split into one sample per slice (C-4 upward)".into();
        self.sync();
    }

    pub fn export_sample(&mut self, path: &Path, a: usize, b: usize, format: ExportFormat) {
        let Some(s) = self.current_sample() else { return };
        let (a, b) = if a == b { (0, s.frames()) } else { s.clamp_range(a, b) };
        match format.write(&path.with_extension(format.extension()), s, a, b) {
            Ok(()) => self.status = format!("exported {}", path.display()),
            Err(e) => self.status = format!("export failed: {e}"),
        }
    }

    // ---- recording ---------------------------------------------------------------------

    /// Give the editor a way to open the audio input (the binary provides one backed by cpal).
    pub fn set_recorder_factory(&mut self, factory: RecorderFactory) {
        self.recorder_factory = Some(factory);
    }

    pub fn can_record(&self) -> bool {
        self.recorder_factory.is_some()
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.is_some()
    }

    pub fn record_seconds(&self) -> f32 {
        self.recorder
            .as_ref()
            .map_or(0.0, |r| self.rec_data.len() as f32 / (r.sample_rate() as f32 * r.channels() as f32))
    }

    pub fn record_toggle(&mut self) {
        if self.is_recording() {
            self.record_stop();
        } else {
            self.record_start();
        }
    }

    pub fn record_start(&mut self) {
        let Some(factory) = &mut self.recorder_factory else {
            self.status = "no audio input available".into();
            return;
        };
        match factory(&self.prefs.audio) {
            Ok(r) => {
                self.recorder = Some(r);
                self.rec_data.clear();
                self.rec_peak = 0.0;
                self.status = "recording... press Ctrl-R again to stop".into();
            }
            Err(e) => self.status = format!("cannot record: {e:#}"),
        }
    }

    /// Stop and turn the take into a new instrument. Returns false if nothing was captured.
    pub fn record_stop(&mut self) -> bool {
        self.record_poll();
        let Some(r) = self.recorder.take() else { return false };
        let data = std::mem::take(&mut self.rec_data);
        self.rec_peak = 0.0;
        if data.is_empty() {
            self.status = "nothing recorded".into();
            return false;
        }
        let n = self.song.instruments.len() + 1;
        let name = format!("Recording {n:02}");
        let mut sample = Sample::new(name.clone(), r.sample_rate(), r.channels(), data);
        let secs = sample.frames() as f32 / sample.sample_rate as f32;
        sample.name = name.clone();
        self.checkpoint();
        let mut inst = Instrument::new(name);
        inst.samples.push(sample);
        self.song_mut().instruments.push(inst);
        self.instrument = (self.song.instruments.len() - 1).min(0xFE) as u8;
        self.sample = 0;
        self.status = format!("recorded {secs:.1}s into instrument {:02X}", self.instrument);
        self.sync();
        true
    }

    /// Pull captured audio into the take buffer; called from `tick`.
    pub(crate) fn record_poll(&mut self) {
        let Some(r) = &mut self.recorder else { return };
        let before = self.rec_data.len();
        r.drain(&mut self.rec_data);
        let peak = self.rec_data[before..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        self.rec_peak = peak.max(self.rec_peak * 0.9);
        let cap = REC_SAMPLE_CAP_SECONDS * r.sample_rate() as usize * r.channels() as usize;
        if self.rec_data.len() >= cap {
            self.record_stop();
            self.status = "recording stopped: 10 minute limit".into();
        }
    }

    // ---- rendering ----------------------------------------------------------------------

    pub(crate) fn render(&mut self) -> Vec<f32> {
        self.render_at(RENDER_RATE)
    }

    fn render_at(&mut self, rate: u32) -> Vec<f32> {
        let chains = self
            .song
            .tracks
            .iter()
            .map(|t| {
                t.devices
                    .iter()
                    .map(|d| {
                        make_device(
                            &mut self.plugin_host,
                            d,
                            &self.song.instruments,
                            RENDER_RATE,
                            &mut self.plugin_errors,
                        )
                    })
                    .collect()
            })
            .collect();
        let inst = self.instrument_devices_for(rate);
        trakr_engine::render_song_with_plugins(self.song.clone(), rate, RENDER_MAX_SECONDS, chains, inst)
    }

    /// Render the whole song to a new stereo instrument (bounce), e.g. for resampling it further.
    pub fn render_to_sample(&mut self) {
        let data = self.render();
        if data.is_empty() {
            self.status = "nothing to render (song is silent)".into();
            return;
        }
        let secs = data.len() as f32 / 2.0 / RENDER_RATE as f32;
        self.checkpoint();
        let mut inst = Instrument::new("Song render");
        inst.samples.push(Sample::new("Song render", RENDER_RATE, 2, data));
        self.song_mut().instruments.push(inst);
        self.instrument = (self.song.instruments.len() - 1).min(0xFE) as u8;
        self.sample = 0;
        self.status = format!("rendered {secs:.1}s to instrument {:02X}", self.instrument);
        self.sync();
    }

    /// Instrument plugin devices for an offline render, as (instrument index, device).
    fn instrument_devices_for(&mut self, rate: u32) -> Vec<(usize, Box<dyn trakr_engine::Device>)> {
        let mut out = Vec::new();
        for i in 0..self.song.instruments.len().min(255) {
            if let Some(spec) = self.song.instruments[i].plugin.clone() {
                let dev = make_device(
                    &mut self.plugin_host,
                    &spec,
                    &self.song.instruments,
                    rate,
                    &mut self.plugin_errors,
                );
                out.push((i, dev));
            }
        }
        out
    }

    /// Chains with plugins loaded, one per track.
    fn chains_for(&mut self, rate: u32) -> Vec<Vec<Box<dyn trakr_engine::Device>>> {
        self.song
            .tracks
            .iter()
            .map(|t| {
                t.devices
                    .iter()
                    .map(|d| {
                        make_device(
                            &mut self.plugin_host,
                            d,
                            &self.song.instruments,
                            rate,
                            &mut self.plugin_errors,
                        )
                    })
                    .collect()
            })
            .collect()
    }

    /// Render `song` offline through the plugin-aware chains.
    fn render_song_copy(&mut self, song: Song, rate: u32) -> Vec<f32> {
        let chains = self.chains_for(rate);
        let inst = self.instrument_devices_for(rate);
        trakr_engine::render_song_with_plugins(Arc::new(song), rate, RENDER_MAX_SECONDS, chains, inst)
    }

    /// Render each sequencer track on its own (through its group, sends and master) into
    /// `<dir>/<base>-<track name>.wav`. Returns how many files were written.
    pub fn export_stems(&mut self, base: &Path, format: ExportFormat) -> usize {
        let rate = format.target_rate(RENDER_RATE);
        let dir = base.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = base.file_stem().map_or("stem".to_string(), |s| s.to_string_lossy().into_owned());
        let mut written = 0;
        for t in 0..self.song.tracks.len() {
            if self.song.tracks[t].kind != trakr_core::TrackKind::Sequencer {
                continue;
            }
            let mut song = (*self.song).clone();
            for (i, tr) in song.tracks.iter_mut().enumerate() {
                tr.solo = i == t;
            }
            song.loop_range = None;
            let data = self.render_song_copy(song, rate);
            if data.is_empty() {
                continue; // a silent track has no stem
            }
            let name: String = self.song.tracks[t]
                .name
                .chars()
                .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
                .collect();
            let s = Sample::new("stem", rate, 2, data);
            let path = dir.join(format!("{stem}-{:02}-{name}.{}", t + 1, format.extension()));
            match format.write_tagged(
                &path,
                &s,
                0,
                s.frames(),
                &self.song_tags(Some(&self.song.tracks[t].name)),
            ) {
                Ok(()) => written += 1,
                Err(e) => {
                    self.status = format!("stem export failed: {e}");
                    return written;
                }
            }
        }
        self.status = format!("exported {written} stem(s) next to {}", base.display());
        written
    }

    /// Render sequence slots `from..=to` (the loop range if set) to a WAV file.
    pub fn export_slots(&mut self, path: &Path, from: usize, to: usize, format: ExportFormat) {
        let rate = format.target_rate(RENDER_RATE);
        let path = &path.with_extension(format.extension());
        let n = self.song.sequence.len();
        if n == 0 || from > to || from >= n {
            self.status = "nothing to export".into();
            return;
        }
        let mut song = (*self.song).clone();
        song.sequence = song.sequence[from..=to.min(n - 1)].to_vec();
        song.loop_range = None;
        let data = self.render_song_copy(song, rate);
        if data.is_empty() {
            self.status = "nothing to render (that part is silent)".into();
            return;
        }
        let s = Sample::new("range", rate, 2, data);
        match format.write_tagged(path, &s, 0, s.frames(), &self.song_tags(None)) {
            Ok(()) => self.status = format!("exported slots {from:02X}-{to:02X} to {}", path.display()),
            Err(e) => self.status = format!("export failed: {e}"),
        }
    }

    /// Tags for an exported file taken from the song's name, author and comments. `part` names a stem or
    /// pattern and is appended to the title.
    pub fn song_tags(&self, part: Option<&str>) -> trakr_encode::Tags {
        let name = self.song.name.trim();
        let title = match (name.is_empty(), part) {
            (true, None) => String::new(),
            (true, Some(p)) => p.to_string(),
            (false, None) => name.to_string(),
            (false, Some(p)) => format!("{name} - {p}"),
        };
        trakr_encode::Tags {
            title,
            artist: self.song.author.trim().to_string(),
            album: String::new(),
            comment: self.song.comments.trim().chars().take(200).collect(),
            year: String::new(),
            genre: String::new(),
        }
    }

    /// Render one pattern on its own (as if it were the whole song) to an audio file.
    pub fn export_pattern(&mut self, path: &Path, pattern: usize, format: ExportFormat) {
        if pattern >= self.song.patterns.len() {
            self.status = "no such pattern".into();
            return;
        }
        let rate = format.target_rate(RENDER_RATE);
        let path = &path.with_extension(format.extension());
        let mut song = (*self.song).clone();
        song.sequence = vec![pattern];
        song.loop_range = None;
        let data = self.render_song_copy(song, rate);
        if data.is_empty() {
            self.status = "nothing to render (that pattern is silent)".into();
            return;
        }
        let s = Sample::new("pattern", rate, 2, data);
        let tags = self.song_tags(Some(&format!("pattern {pattern:02X}")));
        match format.write_tagged(path, &s, 0, s.frames(), &tags) {
            Ok(()) => self.status = format!("exported pattern {pattern:02X} to {}", path.display()),
            Err(e) => self.status = format!("export failed: {e}"),
        }
    }

    /// Render the whole song straight to an audio file at the format's sample rate.
    pub fn export_song(&mut self, path: &Path, format: ExportFormat) {
        let rate = format.target_rate(RENDER_RATE);
        let path = &path.with_extension(format.extension());
        let data = self.render_at(rate);
        if data.is_empty() {
            self.status = "nothing to render (song is silent)".into();
            return;
        }
        let s = Sample::new("render", rate, 2, data);
        match format.write_tagged(path, &s, 0, s.frames(), &self.song_tags(None)) {
            Ok(()) => self.status = format!("exported {}", path.display()),
            Err(e) => self.status = format!("export failed: {e}"),
        }
    }
}
