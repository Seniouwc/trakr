//! Keeps the agent guide honest: the Python helper and its tests run against the real binary.

use std::process::Command;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_python_helper_builds_projects_the_binary_accepts() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not found: skipping");
        return;
    }
    let out = Command::new("python3")
        .arg(root().join("docs/agents/test_trakr_song.py"))
        .env("TRAKR", env!("CARGO_BIN_EXE_trakr"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_worked_example_renders_sound() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("trakr-agents-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let song = dir.join("e.trkr");
    let out = Command::new("python3")
        .arg(root().join("docs/agents/example_beat.py"))
        .args([env!("CARGO_BIN_EXE_trakr"), song.to_str().unwrap()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{text}{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("render:") && !text.contains("SILENT") && !text.contains("clips"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_cli_reports_and_rejects() {
    let bin = env!("CARGO_BIN_EXE_trakr");
    let dir = std::env::temp_dir().join(format!("trakr-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let full = dir.join("full.trkr");
    assert!(Command::new(bin).args(["--example-full", full.to_str().unwrap()]).status().unwrap().success());
    let out = Command::new(bin).args(["--check", full.to_str().unwrap()]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && text.contains("19 EDO") && text.contains("render:"), "{text}");
    let flac = dir.join("full.flac");
    let out = Command::new(bin)
        .args([
            "--render",
            flac.to_str().unwrap(),
            full.to_str().unwrap(),
            "--stems",
            dir.join("stems").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(flac.is_file() && dir.join("stems/00_Lead.flac").is_file());
    let bad = dir.join("bad.trkr");
    std::fs::write(&bad, b"not a zip").unwrap();
    assert!(!Command::new(bin).args(["--check", bad.to_str().unwrap()]).status().unwrap().success());
    let out = Command::new(bin).arg("--devices").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("native.reverb"));
    let _ = std::fs::remove_dir_all(&dir);
}
