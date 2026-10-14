//! Run the GUI on the demo song without an audio device: `cargo run -p trakr-gui --example demo`.
//! `TRAKR_GUI_DOCK=mixer|devices|automation|hidden` picks the bottom panel (handy for screenshots).
fn main() -> anyhow::Result<()> {
    use trakr_core::AutoTarget;
    let mut ed = trakr_editor::Editor::new(trakr_core::Song::demo(), None, None);
    if std::env::var("TRAKR_DEMO_FX").is_ok() {
        ed.device_add(0, "native.filter");
        ed.device_add(0, "native.delay");
        ed.device_add(0, "native.reverb");
        let target = AutoTarget::Device { device: 0, param: 1 };
        ed.automation_edit("demo", 0, target, |a| {
            a.set_point(0.0, 0.2);
            a.set_point(16.0, 0.9);
            a.set_point(40.0, 0.4);
        });
    }
    trakr_gui::run(ed)
}
