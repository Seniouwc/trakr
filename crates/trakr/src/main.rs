use anyhow::Result;
use std::path::PathBuf;
use trakr_core::Song;

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
    let mut editor = trakr_editor::Editor::new(song, path, handle);
    editor.set_config_dir(trakr_editor::default_config_dir());
    editor.load_prefs();
    editor.set_recorder_factory(Box::new(|| {
        Ok(Box::new(trakr_engine::audio::start_input()?) as Box<dyn trakr_editor::InputRecorder>)
    }));
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
    let result = trakr_gui::run(editor);
    drop(audio);
    result
}
