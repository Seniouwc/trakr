use anyhow::Result;
use std::path::PathBuf;
use trakr_core::Song;

/// The audio output selection: owns the running stream and replaces it on request.
struct Audio {
    output: Option<trakr_engine::audio::AudioOutput>,
}

fn to_config(s: &trakr_editor::AudioSettings) -> trakr_engine::audio::AudioConfig {
    trakr_engine::audio::AudioConfig {
        host: s.host.clone(),
        output: s.output.clone(),
        input: s.input.clone(),
        sample_rate: s.sample_rate,
        buffer_frames: s.buffer_frames,
    }
}

fn to_info(s: &trakr_engine::audio::AudioStatus) -> trakr_editor::AudioInfo {
    trakr_editor::AudioInfo {
        host: s.host.clone(),
        device: s.device.clone(),
        sample_rate: s.sample_rate,
        buffer_frames: s.buffer_frames,
        channels: s.channels,
    }
}

impl trakr_editor::AudioControl for Audio {
    fn hosts(&self) -> Vec<String> {
        trakr_engine::audio::hosts()
    }
    fn outputs(&self, host: &str) -> Vec<String> {
        trakr_engine::audio::output_devices(host)
    }
    fn inputs(&self, host: &str) -> Vec<String> {
        trakr_engine::audio::input_devices(host)
    }
    fn rates(&self, host: &str, output: &str) -> Vec<u32> {
        trakr_engine::audio::output_rates(host, output)
    }
    fn restart(
        &mut self,
        settings: &trakr_editor::AudioSettings,
    ) -> Result<(trakr_engine::EngineHandle, trakr_editor::AudioInfo), String> {
        // Close the old stream first: many devices cannot be opened twice.
        self.output = None;
        match trakr_engine::audio::start_with(&to_config(settings)) {
            Ok((out, handle)) => {
                let info = to_info(&out.status);
                self.output = Some(out);
                Ok((handle, info))
            }
            Err(e) => {
                // Put the default output back so there is still sound.
                if let Ok((out, _)) = trakr_engine::audio::start() {
                    self.output = Some(out);
                }
                Err(format!("{e:#}"))
            }
        }
    }
}

fn main() -> Result<()> {
    let mut arg = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-h" | "--help" => {
                println!("usage: trakr [song.trkr | song.xrns | song.mod | song.s3m | song.xm]");
                return Ok(());
            }
            _ if a.starts_with('-') => anyhow::bail!("unknown option: {a}"),
            _ => arg = Some(a),
        }
    }
    let given = arg.is_some();
    let (song, path) = match arg {
        Some(p) => {
            let p = PathBuf::from(p);
            let (song, save_path) = trakr_editor::load_song(&p)?;
            (song, Some(save_path))
        }
        None => (Song::showcase(), None),
    };
    let (audio, handle) = match trakr_engine::audio::start() {
        Ok((a, h)) => (Some(a), Some(h)),
        Err(e) => {
            eprintln!("audio unavailable: {e:#}");
            (None, None)
        }
    };
    let info = audio.as_ref().map(|a| to_info(&a.status));
    let mut editor = trakr_editor::Editor::new(song, path, handle);
    editor.set_config_dir(trakr_editor::default_config_dir());
    editor.load_prefs();
    editor.set_recorder_factory(Box::new(|s| {
        Ok(Box::new(trakr_engine::audio::start_input_with(&to_config(s))?)
            as Box<dyn trakr_editor::InputRecorder>)
    }));
    editor.set_audio_control(Box::new(Audio { output: audio }), info);
    if editor.prefs.audio != trakr_editor::AudioSettings::default() {
        editor.audio_apply();
    }
    editor.set_plugin_host(Box::new(trakr_clap::ClapHost::new()));
    editor.set_midi_backend(Box::new(trakr_midi::MidirBackend));
    if !given && editor.prefs.reopen_last {
        if let Some(last) = editor.prefs.recent.first().cloned().filter(|p| p.is_file()) {
            editor.open(&last);
        }
    }
    if let Some(port) = editor.prefs.osc_port {
        let any = editor.prefs.osc_any_interface;
        editor.osc_start(port, any);
    }
    // Open the first MIDI keyboard automatically; the GUI menu can switch or close it.
    if !editor.midi_ports().is_empty() {
        editor.midi_connect(0);
    }
    trakr_gui::run(editor)
}
