use anyhow::Result;
use std::path::PathBuf;
use trakr_core::Song;

fn main() -> Result<()> {
    let mut tui = false;
    let mut arg = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--tui" => tui = true,
            "-h" | "--help" => {
                println!("usage: trakr [--tui] [song.trkr | song.xrns]\n  --tui   run in the terminal instead of the native window");
                return Ok(());
            }
            _ if a.starts_with('-') => anyhow::bail!("unknown option: {a}"),
            _ => arg = Some(a),
        }
    }
    let (song, path) = match arg {
        Some(p) => {
            let p = PathBuf::from(p);
            let (song, save_path) = trakr_editor::load_song(&p)?;
            (song, Some(save_path))
        }
        None => (Song::demo(), None),
    };

    let (audio, handle) = match trakr_engine::audio::start() {
        Ok((a, h)) => (Some(a), Some(h)),
        Err(e) => {
            eprintln!("audio unavailable: {e:#}");
            (None, None)
        }
    };
    let result = if tui { trakr_tui::run(song, path, handle) } else { trakr_gui::run(song, path, handle) };
    drop(audio);
    result
}
