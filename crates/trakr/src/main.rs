use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use trakr_core::Song;

fn load(path: &Path) -> Result<Song> {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("xrns") => {
            trakr_xrns::import_path(path).with_context(|| format!("importing {}", path.display()))
        }
        Some("trkr") => trakr_format::load_path(path).with_context(|| format!("loading {}", path.display())),
        _ => anyhow::bail!("unknown file type: {} (expected .trkr or .xrns)", path.display()),
    }
}

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
            let song = load(&p)?;
            // Saving an imported .xrns writes a sibling .trkr instead of clobbering the original.
            let save_path = if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("trkr")) {
                p
            } else {
                p.with_extension("trkr")
            };
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
