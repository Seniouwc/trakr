//! Sample panel: instrument and sample properties, editing operations, recording and a zoomable waveform.

use crate::dialogs::FileMode;
use crate::envelopes::EnvKind;
use crate::{GuiApp, SampleTab};
use eframe::egui::{self, Color32, PointerButton, Pos2, Rect, Sense, Stroke, Vec2};
use trakr_core::{LoopMode, Sample};

/// Copy of the selected sample's properties, so the UI can borrow the editor mutably while drawing.
struct Info {
    frames: usize,
    channels: usize,
    rate: u32,
    base: u32,
    fine: i32,
    vol: f32,
    pan: f32,
    mode: LoopMode,
    loop_start: usize,
    loop_end: usize,
    keys: (u32, u32),
    vel: (u32, u32),
    name: String,
    peak: f32,
}

impl Info {
    fn of(s: &Sample) -> Self {
        Self {
            frames: s.frames(),
            channels: s.channels as usize,
            rate: s.sample_rate,
            base: s.base_note as u32,
            fine: s.fine_tune as i32,
            vol: s.volume,
            pan: s.panning,
            mode: s.loop_mode,
            loop_start: s.loop_start,
            loop_end: s.loop_end,
            keys: (s.key_range.0 as u32, s.key_range.1 as u32),
            vel: (s.vel_range.0 as u32, s.vel_range.1 as u32),
            name: s.name.clone(),
            peak: s.peak(0, s.frames()),
        }
    }
}

fn secs(frames: usize, rate: u32) -> String {
    format!("{:.3}s", frames as f64 / rate.max(1) as f64)
}

impl GuiApp {
    pub fn sample_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("sample_scroll").auto_shrink([false, false]).show(ui, |ui| {
            self.instrument_row(ui);
            if self.editor.current_instrument().is_some() {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.sample_tab, SampleTab::Sample, "Sample");
                    ui.selectable_value(&mut self.sample_tab, SampleTab::Filter, "Filter");
                    let nl = self.editor.current_instrument().map_or(0, |i| i.lfos.len());
                    let label = if nl > 0 { format!("LFOs ({nl})") } else { "LFOs".to_string() };
                    ui.selectable_value(&mut self.sample_tab, SampleTab::Modulation, label);
                    let nfx = self.editor.current_instrument().map_or(0, |i| i.fx.len());
                    ui.selectable_value(
                        &mut self.sample_tab,
                        SampleTab::Fx,
                        if nfx > 0 { format!("FX ({nfx})") } else { "FX".to_string() },
                    );
                    let has_plugin = self.editor.current_instrument().is_some_and(|i| i.plugin.is_some());
                    ui.selectable_value(
                        &mut self.sample_tab,
                        SampleTab::Plugin,
                        if has_plugin { "Plugin *" } else { "Plugin" },
                    );
                    for k in [EnvKind::Volume, EnvKind::Pan, EnvKind::Pitch, EnvKind::Cutoff] {
                        let on = self.editor.current_instrument().is_some_and(|i| match k {
                            EnvKind::Volume => i.volume_env.enabled,
                            EnvKind::Pan => i.pan_env.enabled,
                            EnvKind::Pitch => i.pitch_env.enabled,
                            EnvKind::Cutoff => i.cutoff_env.enabled,
                        });
                        let label = format!("{} env{}", k.label(), if on { " *" } else { "" });
                        ui.selectable_value(&mut self.sample_tab, SampleTab::Env(k), label);
                    }
                });
                if self.sample_tab == SampleTab::Filter {
                    self.filter_ui(ui);
                    return;
                }
                if self.sample_tab == SampleTab::Fx {
                    self.instrument_fx_ui(ui);
                    return;
                }
                if self.sample_tab == SampleTab::Plugin {
                    self.plugin_instrument_ui(ui);
                    return;
                }
                if self.sample_tab == SampleTab::Modulation {
                    self.modulation_ui(ui);
                    return;
                }
                if let SampleTab::Env(k) = self.sample_tab {
                    self.envelope_editor(ui, k);
                    return;
                }
            }
            let Some(info) = self.editor.current_sample().map(Info::of) else {
                ui.weak("No sample here. Use Import/Add file, or record from your input with Rec (Ctrl-R).");
                return;
            };
            self.sample_list_row(ui);
            self.sample_props_rows(ui, &info);
            self.sample_edit_rows(ui, &info);
            self.sample_process_rows(ui, &info);
            self.waveform(ui, &info);
        });
    }

    fn instrument_row(&mut self, ui: &mut egui::Ui) {
        let Some(inst) = self.editor.current_instrument() else {
            ui.weak("Select or import an instrument to edit its samples.");
            return;
        };
        let mut name = inst.name.clone();
        let mut volume = inst.volume;
        ui.horizontal_wrapped(|ui| {
            ui.label("Instrument");
            if ui.text_edit_singleline(&mut name).changed() {
                self.editor.instrument_rename(name);
            }
            ui.label("Volume");
            if ui.add(egui::DragValue::new(&mut volume).range(0.0..=4.0).speed(0.01)).changed() {
                self.editor.instrument_set_volume(volume);
            }
            ui.menu_button("File", |ui| {
                if ui.button("Save instrument...").clicked() {
                    self.open_dialog(crate::dialogs::FileMode::SaveInstrument);
                    ui.close_menu();
                }
                if ui.button("Load instrument...").clicked() {
                    self.open_dialog(crate::dialogs::FileMode::LoadInstrument);
                    ui.close_menu();
                }
                if ui
                    .button("Save to instrument bank")
                    .on_hover_text("Keeps a copy in your settings folder")
                    .clicked()
                {
                    self.editor.instrument_save_to_bank();
                    ui.close_menu();
                }
                ui.separator();
                let bank = self.editor.instrument_bank();
                if bank.is_empty() {
                    ui.weak("The bank is empty");
                }
                for (name, path) in bank {
                    if ui.button(format!("Insert {name}")).clicked() {
                        self.editor.instrument_load_file(&path);
                        ui.close_menu();
                    }
                }
            });
            let mut nna = self.editor.current_instrument().map_or(trakr_core::Nna::Cut, |i| i.nna);
            let before = nna;
            egui::ComboBox::from_id_salt("nna")
                .selected_text(match nna {
                    trakr_core::Nna::Cut => "NNA: cut",
                    trakr_core::Nna::NoteOff => "NNA: note-off",
                    trakr_core::Nna::Continue => "NNA: continue",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut nna, trakr_core::Nna::Cut, "Cut: the old note stops at once");
                    ui.selectable_value(
                        &mut nna,
                        trakr_core::Nna::NoteOff,
                        "Note-off: the old note plays its release",
                    );
                    ui.selectable_value(
                        &mut nna,
                        trakr_core::Nna::Continue,
                        "Continue: the old note plays to its end",
                    );
                })
                .response
                .on_hover_text("What happens to a sounding note when a new one starts in the same column");
            if nna != before {
                self.editor.edit_instrument("nna", |i| i.nna = nna);
            }
            let midi = self.editor.current_instrument().and_then(|i| i.midi_out);
            let mut on = midi.is_some();
            if ui
                .checkbox(&mut on, "MIDI out")
                .on_hover_text("Send this instrument's notes to the MIDI output port (Play > MIDI output)")
                .changed()
            {
                self.editor.instrument_set_midi(on.then(trakr_core::MidiOut::default));
            }
            if let Some(m) = midi {
                let mut ch = m.channel as u32 + 1;
                ui.label("Ch");
                if ui.add(egui::DragValue::new(&mut ch).range(1..=16)).changed() {
                    self.editor.instrument_set_midi(Some(trakr_core::MidiOut { channel: ch as u8 - 1, ..m }));
                }
                let mut prog_on = m.program.is_some();
                let mut prog = m.program.unwrap_or(0) as u32;
                let a = ui.checkbox(&mut prog_on, "Prog").changed();
                let b = prog_on && ui.add(egui::DragValue::new(&mut prog).range(0..=127)).changed();
                if a || b {
                    self.editor.instrument_set_midi(Some(trakr_core::MidiOut {
                        program: prog_on.then_some(prog as u8),
                        ..m
                    }));
                }
            }
            let mut interp = self
                .editor
                .current_instrument()
                .map_or(trakr_core::Interpolation::Cubic, |i| i.interpolation);
            let before_i = interp;
            egui::ComboBox::from_id_salt("interp")
                .selected_text(match interp {
                    trakr_core::Interpolation::Nearest => "Interpolation: none",
                    trakr_core::Interpolation::Linear => "Interpolation: linear",
                    trakr_core::Interpolation::Cubic => "Interpolation: cubic",
                    trakr_core::Interpolation::Sinc => "Interpolation: sinc",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut interp,
                        trakr_core::Interpolation::Nearest,
                        "None: the nearest frame, gritty like old samplers",
                    );
                    ui.selectable_value(&mut interp, trakr_core::Interpolation::Linear, "Linear: cheapest");
                    ui.selectable_value(
                        &mut interp,
                        trakr_core::Interpolation::Cubic,
                        "Cubic: smooth (default)",
                    );
                    ui.selectable_value(
                        &mut interp,
                        trakr_core::Interpolation::Sinc,
                        "Sinc: cleanest, more CPU",
                    );
                })
                .response
                .on_hover_text("How the sample is read between frames when pitched");
            if interp != before_i {
                self.editor.edit_instrument("interp", |i| i.interpolation = interp);
            }
            ui.separator();
            if ui.button("Play").on_hover_text("Audition the instrument at the sample's base note").clicked()
            {
                let note = self.editor.current_sample().map_or(48, |s| s.base_note);
                self.editor.preview_note(note);
            }
            ui.separator();
            if ui
                .button("Export sample...")
                .on_hover_text("Write the selection (or whole sample) as WAV, FLAC, Ogg, MP3, AAC or Opus")
                .clicked()
            {
                self.open_dialog(FileMode::ExportSample);
            }
        });
    }

    fn sample_list_row(&mut self, ui: &mut egui::Ui) {
        let names: Vec<String> = self
            .editor
            .current_instrument()
            .map(|i| i.samples.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        let current = self.editor.sample_index();
        ui.horizontal_wrapped(|ui| {
            ui.label("Samples");
            for (i, n) in names.iter().enumerate() {
                if ui.selectable_label(i == current, format!("{:02} {n}", i + 1)).clicked() {
                    self.editor.select_sample(i);
                    self.sample_sel = None;
                    self.wave_view = None;
                }
            }
            if ui.small_button("+ file").on_hover_text("Add a sample file to this instrument").clicked() {
                self.open_dialog(FileMode::AddSample);
            }
            if names.len() > 1 && ui.small_button("Remove").clicked() {
                self.editor.sample_remove(current);
                self.sample_sel = None;
            }
        });
    }

    fn sample_props_rows(&mut self, ui: &mut egui::Ui, i: &Info) {
        let idx = self.editor.sample_index();
        let (mut base, mut fine, mut vol, mut pan) = (i.base, i.fine, i.vol, i.pan);
        let (mut mode, mut ls, mut le) = (i.mode, i.loop_start as u64, i.loop_end as u64);
        let mut keys = i.keys;
        let mut vel = i.vel;
        ui.horizontal_wrapped(|ui| {
            ui.monospace(format!(
                "{}  {} Hz  {} ch  {} frames  {}  peak {:.1} dB",
                i.name,
                i.rate,
                i.channels,
                i.frames,
                secs(i.frames, i.rate),
                20.0 * i.peak.max(1e-6).log10()
            ));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Base note");
            let c = ui.add(
                egui::DragValue::new(&mut base)
                    .range(0..=119)
                    .custom_formatter(|n, _| trakr_core::note::note_name(n as u8)),
            );
            if c.changed() {
                self.editor.edit_sample_coalesced("base", idx, |s| s.base_note = base as u8);
            }
            ui.label("Fine");
            if ui.add(egui::DragValue::new(&mut fine).range(-128..=127)).changed() {
                self.editor.edit_sample_coalesced("fine", idx, |s| s.fine_tune = fine as i8);
            }
            ui.label("Vol");
            if ui.add(egui::DragValue::new(&mut vol).range(0.0..=4.0).speed(0.01)).changed() {
                self.editor.edit_sample_coalesced("svol", idx, |s| s.volume = vol);
            }
            ui.label("Pan");
            if ui.add(egui::DragValue::new(&mut pan).range(-1.0..=1.0).speed(0.01)).changed() {
                self.editor.edit_sample_coalesced("span", idx, |s| s.panning = pan);
            }
            ui.label("Keys");
            let fmt = |n: f64, _: std::ops::RangeInclusive<usize>| trakr_core::note::note_name(n as u8);
            let lo = ui.add(egui::DragValue::new(&mut keys.0).range(0..=119).custom_formatter(fmt));
            ui.label("to");
            let hi = ui.add(egui::DragValue::new(&mut keys.1).range(0..=119).custom_formatter(fmt));
            if lo.changed() || hi.changed() {
                self.editor
                    .edit_sample_coalesced("keys", idx, |s| s.key_range = (keys.0 as u8, keys.1 as u8));
            }
            ui.label("Velocity");
            let vlo = ui.add(egui::DragValue::new(&mut vel.0).range(0..=127));
            ui.label("to");
            let vhi = ui.add(egui::DragValue::new(&mut vel.1).range(0..=127));
            if vlo.changed() || vhi.changed() {
                self.editor.edit_sample_coalesced("vel", idx, |s| s.vel_range = (vel.0 as u8, vel.1 as u8));
            }
        });
        ui.horizontal_wrapped(|ui| {
            let before = mode;
            egui::ComboBox::from_id_salt("loopmode").selected_text(format!("Loop: {mode:?}")).show_ui(
                ui,
                |ui| {
                    for m in [LoopMode::Off, LoopMode::Forward, LoopMode::Reverse, LoopMode::PingPong] {
                        ui.selectable_value(&mut mode, m, format!("{m:?}"));
                    }
                },
            );
            if mode != before {
                self.editor.edit_sample(idx, |s| s.loop_mode = mode);
            }
            let mut one_shot = self.editor.current_sample().is_some_and(|s| s.one_shot);
            if ui
                .checkbox(&mut one_shot, "One-shot")
                .on_hover_text("Play to the end whatever happens: note-off does not stop it (drum hits)")
                .changed()
            {
                self.editor.edit_sample(idx, |s| s.one_shot = one_shot);
            }
            let mut release = self.editor.current_sample().is_some_and(|s| s.loop_release);
            if ui
                .checkbox(&mut release, "Loop release")
                .on_hover_text("Loop until note-off, then play on to the end of the sample (even with a volume envelope)")
                .changed()
            {
                self.editor.edit_sample(idx, |s| s.loop_release = release);
            }
            ui.label("Start");
            if ui.add(egui::DragValue::new(&mut ls).range(0..=i.frames as u64).speed(16.0)).changed() {
                self.editor
                    .edit_sample_coalesced("ls", idx, |s| s.loop_start = (ls as usize).min(s.loop_end));
            }
            ui.label("End");
            if ui.add(egui::DragValue::new(&mut le).range(0..=i.frames as u64).speed(16.0)).changed() {
                self.editor
                    .edit_sample_coalesced("le", idx, |s| s.loop_end = (le as usize).max(s.loop_start));
            }
            if let Some((a, b)) = self.sample_sel {
                if ui.button("Loop = selection").clicked() {
                    self.editor.edit_sample(idx, |s| {
                        s.loop_start = a;
                        s.loop_end = b;
                        if s.loop_mode == LoopMode::Off {
                            s.loop_mode = LoopMode::Forward;
                        }
                    });
                }
            }
        });
    }

    fn sample_edit_rows(&mut self, ui: &mut egui::Ui, i: &Info) {
        let idx = self.editor.sample_index();
        let whole = (0, i.frames);
        let sel = self.sample_sel.filter(|(a, b)| b > a);
        let range = sel.unwrap_or(whole);
        let need_sel = |app: &mut GuiApp, what: &str| {
            app.editor.status = format!("{what}: drag on the waveform to select a range first");
        };
        ui.horizontal_wrapped(|ui| {
            ui.label("Edit");
            if ui.button("Cut").clicked() {
                match sel {
                    Some((a, b)) => {
                        self.editor.sample_cut(a, b);
                        self.sample_sel = None;
                    }
                    None => need_sel(self, "Cut"),
                }
            }
            if ui.button("Copy").clicked() {
                match sel {
                    Some((a, b)) => self.editor.sample_copy(a, b),
                    None => need_sel(self, "Copy"),
                }
            }
            if ui.button("Paste").on_hover_text("Insert the clipboard at the selection start").clicked() {
                self.editor.sample_paste(range.0);
            }
            if ui.button("Delete").clicked() {
                match sel {
                    Some((a, b)) => {
                        self.editor.sample_delete(a, b);
                        self.sample_sel = None;
                    }
                    None => need_sel(self, "Delete"),
                }
            }
            if ui.button("Crop").on_hover_text("Keep only the selection").clicked() {
                match sel {
                    Some((a, b)) => {
                        self.editor.sample_crop(a, b);
                        self.sample_sel = None;
                        self.wave_view = None;
                    }
                    None => need_sel(self, "Crop"),
                }
            }
            ui.separator();
            ui.label(if sel.is_some() { "on selection:" } else { "on whole sample:" });
            let (a, b) = range;
            if ui.button("Silence").clicked() {
                self.editor.edit_sample(idx, |s| s.silence(a, b));
            }
            if ui.button("Fade in").clicked() {
                self.editor.edit_sample(idx, |s| s.fade_in_range(a, b));
            }
            if ui.button("Fade out").clicked() {
                self.editor.edit_sample(idx, |s| s.fade_out_range(a, b));
            }
            if ui.button("Reverse").clicked() {
                self.editor.edit_sample(idx, |s| s.reverse_range(a, b));
            }
            if ui.button("Invert").clicked() {
                self.editor.edit_sample(idx, |s| s.invert(a, b));
            }
            if ui.button("Normalize").clicked() {
                self.editor.edit_sample(idx, |s| s.normalize_range(a, b));
            }
            ui.add(egui::DragValue::new(&mut self.gain_db).range(-48.0..=48.0).speed(0.1).suffix(" dB"));
            if ui.button("Gain").clicked() {
                let f = 10f32.powf(self.gain_db / 20.0);
                self.editor.edit_sample(idx, |s| s.gain(a, b, f));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Whole sample");
            if ui.button("Remove DC").clicked() {
                self.editor.edit_sample(idx, |s| s.remove_dc());
            }
            if ui.button("To mono").clicked() {
                self.editor.edit_sample(idx, |s| s.to_mono());
            }
            if ui.button("To stereo").clicked() {
                self.editor.edit_sample(idx, |s| s.to_stereo());
            }
            ui.separator();
            ui.add(egui::DragValue::new(&mut self.pitch_st).range(-48.0..=48.0).speed(0.1).suffix(" st"));
            if ui
                .button("Pitch shift")
                .on_hover_text("Tape-style: changes length and pitch together")
                .clicked()
            {
                let st = self.pitch_st;
                self.editor.edit_sample(idx, |s| s.pitch_shift(st));
                self.sample_sel = None;
            }
            if ui
                .button("Pitch (keep length)")
                .on_hover_text("Shifts the pitch and stretches the sample back to its length (WSOLA)")
                .clicked()
            {
                let st = self.pitch_st;
                self.editor.edit_sample(idx, |s| s.pitch_shift_keep_length(st));
                self.sample_sel = None;
            }
            if ui
                .button("Pitch (keep formants)")
                .on_hover_text("Like keep length, then restores the original spectral envelope so voices keep their vowel colour")
                .clicked()
            {
                let st = self.pitch_st;
                self.editor.edit_sample(idx, |s| s.pitch_shift_formant(st));
                self.sample_sel = None;
            }
            ui.add(
                egui::DragValue::new(&mut self.resample_hz).range(4000..=192_000).speed(100.0).suffix(" Hz"),
            );
            if ui
                .button("Resample")
                .on_hover_text("Convert to this rate without changing the sound")
                .clicked()
            {
                let hz = self.resample_hz;
                self.editor.edit_sample(idx, |s| s.resample_to(hz));
            }
            ui.separator();
            ui.label("Slices");
            let n = self.editor.current_sample().map_or(0, |s| s.slices.len());
            ui.add(egui::DragValue::new(&mut self.slice_parts).range(2..=64).suffix(" parts"));
            if ui.button("Even").on_hover_text("Place evenly spaced slice markers").clicked() {
                let p = self.slice_parts as usize;
                self.editor.edit_sample(idx, |s| s.slice_evenly(p));
            }
            if ui.button("Add at selection").on_hover_text("Or Shift+click on the waveform").clicked() {
                if let Some((a, _)) = sel {
                    self.editor.edit_sample(idx, |s| s.add_slice(a));
                }
            }
            if ui.button("Clear").clicked() {
                self.editor.edit_sample(idx, |s| s.slices.clear());
            }
            if ui
                .add_enabled(n > 0, egui::Button::new(format!("Split ({} markers)", n)))
                .on_hover_text("One sample per slice, keys from C-4 up")
                .clicked()
            {
                self.editor.sample_split_slices();
                self.sample_sel = None;
                self.wave_view = None;
            }
        });
    }

    fn sample_process_rows(&mut self, ui: &mut egui::Ui, i: &Info) {
        let idx = self.editor.sample_index();
        ui.horizontal_wrapped(|ui| {
            ui.label("Process");
            let rate = i.rate.max(1) as f32;
            ui.add(egui::DragValue::new(&mut self.xfade_ms).range(1.0..=2000.0).speed(1.0).suffix(" ms"));
            if ui
                .button("Loop crossfade")
                .on_hover_text(
                    "Blend the end of the loop into the audio before its start so it wraps without a click",
                )
                .clicked()
            {
                let n = (self.xfade_ms / 1000.0 * rate) as usize;
                let mut ok = true;
                self.editor.edit_sample(idx, |s| ok = s.loop_crossfade(n));
                if !ok {
                    self.editor.status =
                        "loop crossfade needs a loop with some audio before its start".into();
                }
            }
            ui.separator();
            ui.add(egui::DragValue::new(&mut self.stretch).range(0.25..=4.0).speed(0.01).prefix("x"));
            if ui
                .button("Time-stretch")
                .on_hover_text("Longer (above 1) or shorter without changing the pitch")
                .clicked()
            {
                let f = self.stretch as f64;
                self.editor.edit_sample(idx, |s| s.time_stretch(f));
                self.sample_sel = None;
            }
            ui.separator();
            ui.add(egui::Slider::new(&mut self.slice_sens, 0.0..=1.0).text("sensitivity"));
            if ui
                .button("Slice at transients")
                .on_hover_text("Detect drum hits and notes and place markers")
                .clicked()
            {
                let sens = self.slice_sens;
                let mut n = 0;
                self.editor.edit_sample(idx, |s| n = s.auto_slice(sens));
                self.editor.status = format!("{n} transient marker(s) placed");
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Finish");
            ui.add(egui::DragValue::new(&mut self.dither_bits).range(2..=24).suffix(" bit"));
            if ui.button("Reduce with dither").on_hover_text("Round to this bit depth with shaped dither").clicked() {
                let bits = self.dither_bits;
                self.editor.edit_sample(idx, |s| s.reduce_bit_depth(bits, true));
            }
            ui.separator();
            let mut lines = self.editor.current_sample().map_or(0, |s| s.beat_sync_lines);
            ui.label("Beat sync");
            if ui
                .add(egui::DragValue::new(&mut lines).range(0..=512).suffix(" lines"))
                .on_hover_text("0 = off. Otherwise the sample is played so it lasts exactly this many pattern lines at the song tempo (pitch follows the speed)")
                .changed()
            {
                self.editor.edit_sample_coalesced("beat_sync", idx, |s| s.beat_sync_lines = lines);
            }
            ui.separator();
            ui.checkbox(&mut self.show_spectrum, "Spectrum");
        });
        if self.show_spectrum {
            self.spectrum_view(ui);
        }
    }

    /// Frequency spectrum of the selection (or the start of the sample), log frequency axis.
    fn spectrum_view(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.editor.current_sample() else { return };
        let ch = s.channels.max(1) as usize;
        let (a, b) = self.sample_sel.filter(|(a, b)| b > a).unwrap_or((0, s.frames()));
        let n = (b - a).min(8192);
        let mono: Vec<f32> =
            (a..a + n).map(|f| (0..ch).map(|c| s.data[f * ch + c]).sum::<f32>() / ch as f32).collect();
        let size = 4096.min(mono.len().max(2).next_power_of_two());
        let mags = trakr_core::fft::magnitude_spectrum(&mono, size);
        let rate = s.sample_rate as f32;
        let (resp, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 110.0), Sense::hover());
        let r = resp.rect;
        painter.rect_filled(r, 2.0, Color32::from_gray(14));
        let (fmin, fmax) = (20.0f32, (rate / 2.0).max(40.0));
        let x_of = |f: f32| r.left() + (f / fmin).ln() / (fmax / fmin).ln() * r.width();
        for g in [100.0, 1000.0, 10_000.0] {
            if g < fmax {
                let x = x_of(g);
                painter.line_segment(
                    [Pos2::new(x, r.top()), Pos2::new(x, r.bottom())],
                    Stroke::new(1.0_f32, Color32::from_gray(40)),
                );
                painter.text(
                    Pos2::new(x + 2.0, r.bottom() - 12.0),
                    egui::Align2::LEFT_TOP,
                    format!("{g} Hz"),
                    egui::FontId::monospace(9.0),
                    Color32::from_gray(110),
                );
            }
        }
        let y_of = |m: f32| {
            let db = 20.0 * m.max(1e-6).log10(); // -120..0
            r.bottom() - ((db + 100.0) / 100.0).clamp(0.0, 1.0) * (r.height() - 4.0)
        };
        let mut pts = Vec::new();
        for (k, m) in mags.iter().enumerate().skip(1) {
            let f = k as f32 * rate / size as f32;
            if f >= fmin {
                pts.push(Pos2::new(x_of(f), y_of(*m)));
            }
        }
        painter.add(egui::Shape::line(pts, Stroke::new(1.0_f32, Color32::from_rgb(120, 180, 255))));
    }

    /// Zoomable waveform. Drag selects, Shift+click adds a slice marker, wheel zooms, Shift+wheel or
    /// middle-drag pans.
    fn waveform(&mut self, ui: &mut egui::Ui, info: &Info) {
        let frames = info.frames.max(1);
        let (mut vs, mut vl) = self.wave_view.unwrap_or((0, frames));
        vl = vl.clamp(16.min(frames), frames);
        vs = vs.min(frames - vl);

        let size = Vec2::new(ui.available_width(), ui.available_height().max(120.0));
        let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let r = resp.rect;
        painter.rect_filled(r, 2.0, Color32::from_gray(14));
        let frame_at = |x: f32, vs: usize, vl: usize| {
            (vs as f64 + ((x - r.left()) / r.width()).clamp(0.0, 1.0) as f64 * vl as f64) as usize
        };

        // Wheel: zoom around the pointer; Shift+wheel pans.
        if resp.hovered() {
            let (d, shift, ptr) =
                ui.input(|i| (i.smooth_scroll_delta, i.modifiers.shift, i.pointer.hover_pos()));
            if shift || d.x != 0.0 {
                let pan = ((d.x + if shift { d.y } else { 0.0 }) * vl as f32 / r.width()) as i64;
                vs = (vs as i64 - pan).clamp(0, (frames - vl) as i64) as usize;
            } else if d.y != 0.0 {
                let anchor = ptr.map_or(vs + vl / 2, |p| frame_at(p.x, vs, vl));
                let factor = (1.0 - d.y as f64 * 0.003).clamp(0.5, 2.0);
                let new_len = ((vl as f64 * factor) as usize).clamp(16.min(frames), frames);
                let rel = (anchor.saturating_sub(vs)) as f64 / vl as f64;
                vs = (anchor as f64 - rel * new_len as f64).max(0.0) as usize;
                vl = new_len;
                vs = vs.min(frames - vl);
            }
        }
        if resp.dragged_by(PointerButton::Middle) {
            let pan = (resp.drag_delta().x * vl as f32 / r.width()) as i64;
            vs = (vs as i64 - pan).clamp(0, (frames - vl) as i64) as usize;
        }
        self.wave_view = if vl == frames { None } else { Some((vs, vl)) };
        let to_x = |f: f64| r.left() + ((f - vs as f64) / vl as f64) as f32 * r.width();
        let to_frame = |x: f32| frame_at(x, vs, vl);

        // Waveform per channel lane.
        let ch = info.channels.max(1);
        let lane_h = r.height() / ch as f32;
        let Some(s) = self.editor.current_sample() else { return };
        for c in 0..ch {
            let mid = r.top() + lane_h * (c as f32 + 0.5);
            let half = lane_h * 0.5 - 2.0;
            painter.line_segment(
                [Pos2::new(r.left(), mid), Pos2::new(r.right(), mid)],
                Stroke::new(1.0_f32, Color32::from_gray(40)),
            );
            let green = Color32::from_rgb(90, 200, 120);
            if vl * 2 <= r.width() as usize {
                // Zoomed far in: connect the actual samples.
                let pts: Vec<Pos2> = (vs..(vs + vl + 1).min(frames))
                    .map(|f| Pos2::new(to_x(f as f64), mid - s.data[f * ch + c] * half))
                    .collect();
                painter.add(egui::Shape::line(pts.clone(), Stroke::new(1.0_f32, green)));
                if vl * 8 <= r.width() as usize {
                    for p in pts {
                        painter.circle_filled(p, 2.0, green);
                    }
                }
            } else {
                let w = r.width().max(1.0) as usize;
                for x in 0..w {
                    let a = vs + x * vl / w;
                    let b = (vs + ((x + 1) * vl / w).max(x * vl / w + 1)).min(frames);
                    let (mut lo, mut hi) = (0.0f32, 0.0f32);
                    for f in a..b {
                        let v = s.data[f * ch + c];
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                    let px = r.left() + x as f32;
                    painter.line_segment(
                        [Pos2::new(px, mid - hi * half), Pos2::new(px, mid - lo * half - 0.5)],
                        Stroke::new(1.0_f32, green),
                    );
                }
            }
        }

        // Markers.
        let vline = |f: usize, color: Color32| {
            let x = to_x(f as f64);
            if x >= r.left() && x <= r.right() {
                painter.line_segment(
                    [Pos2::new(x, r.top()), Pos2::new(x, r.bottom())],
                    Stroke::new(1.0_f32, color),
                );
            }
        };
        if info.mode != LoopMode::Off {
            vline(info.loop_start, Color32::from_rgb(230, 170, 40));
            vline(info.loop_end, Color32::from_rgb(230, 170, 40));
        }
        for &p in &s.slices {
            vline(p, Color32::from_rgb(230, 80, 200));
        }
        if let Some((a, b)) = self.sample_sel {
            let sel = Rect::from_min_max(
                Pos2::new(to_x(a as f64).max(r.left()), r.top()),
                Pos2::new(to_x(b as f64).min(r.right()), r.bottom()),
            );
            painter.rect_filled(sel, 0.0, Color32::from_rgba_unmultiplied(80, 140, 255, 50));
        }

        // Selecting and slicing.
        let shift = ui.input(|i| i.modifiers.shift);
        if self.sample_draw {
            // Draw mode: dragging writes the waveform under the pointer (the zoom decides the precision).
            if let (Some(p), true) = (resp.interact_pointer_pos(), resp.dragged_by(PointerButton::Primary)) {
                let f = to_frame(p.x).min(frames - 1);
                let v = (1.0 - 2.0 * ((p.y - r.top()) / r.height())).clamp(-1.0, 1.0);
                let (f0, v0) = self.draw_last.unwrap_or((f, v));
                let idx = self.editor.sample_index();
                self.editor.edit_sample_coalesced("draw", idx, |s| s.draw(f0, f, v0, v, None));
                self.draw_last = Some((f, v));
            }
            if resp.drag_stopped() {
                self.draw_last = None;
            }
        } else if let Some(p) = resp.interact_pointer_pos() {
            if resp.drag_started_by(PointerButton::Primary) {
                self.sample_drag = Some(to_frame(p.x));
            }
            if resp.dragged_by(PointerButton::Primary) {
                if let Some(a) = self.sample_drag {
                    let f = to_frame(p.x);
                    self.sample_sel = Some((a.min(f), a.max(f).min(frames)));
                }
            }
            if resp.clicked() {
                if shift {
                    let f = to_frame(p.x);
                    let idx = self.editor.sample_index();
                    self.editor.edit_sample(idx, |s| s.add_slice(f));
                } else {
                    self.sample_sel = None;
                }
            }
        }
        let text = match self.sample_sel {
            Some((a, b)) => format!(
                "selection {a}..{b}  ({} frames, {})   view {}..{}",
                b - a,
                secs(b - a, info.rate),
                vs,
                vs + vl
            ),
            None => format!(
                "view {}..{} of {}   (wheel zoom, Shift+wheel pan, drag select, Shift+click slice)",
                vs,
                vs + vl,
                frames
            ),
        };
        ui.horizontal(|ui| {
            ui.weak(text);
            if ui.small_button("Fit").clicked() {
                self.wave_view = None;
            }
            ui.checkbox(&mut self.sample_draw, "Draw")
                .on_hover_text("Drag on the waveform to draw sample values by hand (zoom in first)");
            if let Some((a, b)) = self.sample_sel {
                if ui.small_button("Zoom to selection").clicked() {
                    self.wave_view = Some((a, (b - a).max(16)));
                }
            }
        });
    }
}

impl GuiApp {
    /// Per-instrument resonant filter; the cutoff envelope tab modulates it over the note.
    fn filter_ui(&mut self, ui: &mut egui::Ui) {
        let Some(f) = self.editor.current_instrument().map(|i| i.filter) else { return };
        let (mut mode, mut cutoff, mut q) = (f.mode, f.cutoff, f.resonance);
        ui.horizontal_wrapped(|ui| {
            ui.label("Filter");
            egui::ComboBox::from_id_salt("inst_filter_mode").selected_text(format!("{mode:?}")).show_ui(
                ui,
                |ui| {
                    for m in [
                        trakr_core::FilterMode::Off,
                        trakr_core::FilterMode::LowPass,
                        trakr_core::FilterMode::HighPass,
                        trakr_core::FilterMode::BandPass,
                        trakr_core::FilterMode::Notch,
                    ] {
                        ui.selectable_value(&mut mode, m, format!("{m:?}"));
                    }
                },
            );
            let a = ui.add(
                egui::Slider::new(&mut cutoff, 20.0..=20_000.0)
                    .logarithmic(true)
                    .text("Cutoff")
                    .suffix(" Hz"),
            );
            let b = ui.add(egui::Slider::new(&mut q, 0.1..=10.0).logarithmic(true).text("Resonance (Q)"));
            if mode != f.mode || a.changed() || b.changed() {
                self.editor.edit_instrument("inst_filter", |i| {
                    i.filter = trakr_core::InstrumentFilter { mode, cutoff, resonance: q };
                });
            }
        });
        ui.weak("The filter runs on every note of this instrument. Use the Cutoff env tab to sweep it over the life of each note.");
    }
}
