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

const USAGE: &str = "usage: trakr [song.trkr | song.xrns | song.mod | song.s3m | song.xm | song.it]
       trakr --check song.trkr                     load, validate and summarize (no GUI, no audio device)
       trakr --render out.<ext> song.trkr [--rate N] [--seconds N] [--quality 0-10] [--bits 16|24|32] [--stems DIR]
                                                   render offline; ext = wav flac ogg mp3 aac opus (default 44100 Hz, 600 s max);
                                                   --stems DIR also writes one file per sequencer track (others muted)
       trakr --devices | --effects                 list native devices (ids, parameters in order, ranges) or effect commands
       trakr --example-full out.trkr               a project that uses every feature (exact JSON spelling of every field and enum)
       trakr --example out.trkr                    write a small valid project to start from";

/// Headless commands used by scripts and agents. Returns `Ok(true)` when one ran.
fn headless(args: &[String]) -> Result<bool> {
    if args.iter().any(|a| a == "--devices") {
        for d in trakr_core::DEVICES {
            println!("{}  ({})", d.id, d.name);
            for (i, p) in d.params.iter().enumerate() {
                println!("    [{i}] {}: {} .. {} {} (default {})", p.name, p.min, p.max, p.unit, p.default);
            }
        }
        return Ok(true);
    }
    if args.iter().any(|a| a == "--effects") {
        for (id, name, help) in trakr_core::effects::EFFECTS {
            println!("{id}  {name}: {help}");
        }
        return Ok(true);
    }
    let mut it = args.iter();
    let mut mode = None;
    let (mut out, mut input) = (None, None);
    let (mut rate, mut seconds) = (44_100u32, 600.0f32);
    let (mut quality, mut bits, mut stems) = (8u8, 16u8, None::<String>);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--check" => mode = Some("check"),
            "--render" => {
                mode = Some("render");
                out = it.next().cloned();
            }
            "--example-full" => {
                mode = Some("example-full");
                out = it.next().cloned();
            }
            "--example" => {
                mode = Some("example");
                out = it.next().cloned();
            }
            "--rate" => {
                rate = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("--rate needs a number"))?
            }
            "--quality" => {
                quality = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("--quality needs 0..10"))?
            }
            "--bits" => {
                bits = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("--bits needs 16, 24 or 32"))?
            }
            "--stems" => stems = it.next().cloned(),
            "--seconds" => {
                seconds = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("--seconds needs a number"))?
            }
            _ if a.starts_with('-') => anyhow::bail!("unknown option: {a}"),
            _ => input = Some(a.clone()),
        }
    }
    let Some(mode) = mode else { return Ok(false) };
    if mode == "example" || mode == "example-full" {
        let out = out.ok_or_else(|| anyhow::anyhow!("{mode} needs an output path"))?;
        let song = if mode == "example" { Song::demo() } else { Song::kitchen_sink() };
        trakr_format::save_path(&song, std::path::Path::new(&out))?;
        println!("wrote {out}");
        return Ok(true);
    }
    let input = input.ok_or_else(|| anyhow::anyhow!("no song given"))?;
    let (song, warnings) = trakr_editor::load_song_report(std::path::Path::new(&input))?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let song = std::sync::Arc::new(song);
    let used: usize =
        song.patterns.iter().map(|p| p.tracks.iter().map(|t| t.lines.len()).sum::<usize>()).sum();
    println!(
        "ok: \"{}\", {} bpm, lpb {}, {} tracks, {} patterns, sequence of {}, {} instruments, {} non-empty lines{}",
        song.name,
        song.bpm,
        song.lpb,
        song.tracks.len(),
        song.patterns.len(),
        song.sequence.len(),
        song.instruments.len(),
        used,
        if song.edo != 12 { format!(", {} EDO", song.edo) } else { String::new() }
    );
    let pcm = trakr_engine::render_song(song.clone(), rate, seconds);
    report("render", &pcm, rate);
    if mode == "render" {
        let out = out.ok_or_else(|| anyhow::anyhow!("--render needs an output path"))?;
        let path = std::path::Path::new(&out);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("wav");
        let codec = trakr_encode::Codec::from_extension(ext)
            .ok_or_else(|| anyhow::anyhow!("unknown output type .{ext} (wav flac ogg mp3 aac opus)"))?;
        let format =
            trakr_encode::ExportFormat { codec, quality: quality.min(10), bits, ..Default::default() };
        let tags = trakr_encode::Tags {
            title: song.name.clone(),
            artist: song.author.clone(),
            comment: song.comments.clone(),
            ..Default::default()
        };
        let write = |path: &std::path::Path, pcm: Vec<f32>| -> Result<()> {
            let frames = pcm.len() / 2;
            let s = trakr_core::Sample::new("render", rate, 2, pcm);
            format.write_tagged(path, &s, 0, frames, &tags)?;
            Ok(())
        };
        write(path, pcm)?;
        println!("wrote {}", path.with_extension(format.extension()).display());
        if let Some(dir) = stems {
            std::fs::create_dir_all(&dir)?;
            for (i, t) in song.tracks.iter().enumerate() {
                if t.kind != trakr_core::TrackKind::Sequencer {
                    continue;
                }
                let mut solo = (*song).clone();
                solo.tracks.iter_mut().for_each(|t| t.solo = false);
                solo.tracks[i].solo = true;
                let pcm = trakr_engine::render_song(std::sync::Arc::new(solo), rate, seconds);
                report(&format!("stem {i} ({})", t.name), &pcm, rate);
                let name: String =
                    t.name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
                write(
                    &std::path::Path::new(&dir).join(format!("{i:02}_{name}.{}", format.extension())),
                    pcm,
                )?;
            }
        }
    }
    Ok(true)
}

fn report(what: &str, pcm: &[f32], rate: u32) {
    let frames = pcm.len() / 2;
    let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let rms = (pcm.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / pcm.len().max(1) as f64).sqrt();
    println!(
        "{what}: {:.2} s, peak {peak:.3}, rms {rms:.4}{}",
        frames as f32 / rate as f32,
        if frames == 0 { " (SILENT: no sound produced)" } else { "" }
    );
    if peak > 1.0 {
        println!("warning: {what} peak above 1.0 clips; lower track or master volume");
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    if headless(&args)? {
        return Ok(());
    }
    let mut arg = None;
    for a in args {
        match a.as_str() {
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
    if let Some(target) = editor.prefs.osc_feedback.clone() {
        editor.osc_feedback_start(&target);
    }
    // Open the first MIDI keyboard automatically; the GUI menu can switch or close it.
    if !editor.midi_ports().is_empty() {
        editor.midi_connect(0);
    }
    trakr_gui::run(editor)
}
