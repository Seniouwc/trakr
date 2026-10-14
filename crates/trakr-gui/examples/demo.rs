//! Run the GUI on the demo song without an audio device: `cargo run -p trakr-gui --example demo`.
fn main() -> anyhow::Result<()> {
    trakr_gui::run(trakr_editor::Editor::new(trakr_core::Song::demo(), None, None))
}
