//! Lua script console: write or load a script, run it, read its output. Scripts also run straight
//! from the Scripts menu.

use crate::GuiApp;
use eframe::egui;

impl GuiApp {
    pub fn scripts_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Script console...").clicked() {
            self.show_console = true;
            ui.close_menu();
        }
        ui.separator();
        ui.menu_button("Examples", |ui| {
            for (name, src) in trakr_editor::SCRIPT_EXAMPLES {
                if ui
                    .button(*name)
                    .on_hover_text("Run now (one undo step); open the console to read or change it")
                    .clicked()
                {
                    self.editor.run_script(src);
                    ui.close_menu();
                }
            }
        });
        let user = self.editor.user_scripts();
        ui.separator();
        if user.is_empty() {
            match self.editor.scripts_dir() {
                Some(d) => ui.weak(format!("Put .lua files in {} to list them here", d.display())),
                None => ui.weak("No scripts folder"),
            };
        }
        for (name, path) in user {
            if ui.button(&name).clicked() {
                self.editor.run_script_file(&path);
                ui.close_menu();
            }
        }
    }

    pub fn console_window(&mut self, ctx: &egui::Context) {
        if !self.show_console {
            return;
        }
        let mut open = true;
        egui::Window::new("Script console").open(&mut open).default_size([560.0, 460.0]).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Run").on_hover_text("Runs against a copy of the song; one undo step").clicked()
                {
                    let src = self.script_text.clone();
                    self.editor.run_script(&src);
                }
                egui::ComboBox::from_id_salt("script_examples").selected_text("Load example").show_ui(
                    ui,
                    |ui| {
                        for (name, src) in trakr_editor::SCRIPT_EXAMPLES {
                            if ui.selectable_label(false, *name).clicked() {
                                self.script_text = (*src).to_string();
                            }
                        }
                    },
                );
                if ui.button("Clear").clicked() {
                    self.script_text.clear();
                }
                ui.separator();
                ui.label("Save as");
                ui.add(egui::TextEdit::singleline(&mut self.script_name).desired_width(110.0));
                if ui.button("Save").on_hover_text("Writes <name>.lua to the scripts folder").clicked() {
                    self.save_user_script();
                }
            });
            ui.add(
                egui::TextEdit::multiline(&mut self.script_text)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .desired_rows(14)
                    .desired_width(f32::INFINITY)
                    .hint_text("-- Lua. Try: trakr.status('hello ' .. trakr.song().name)"),
            );
            if let Some(e) = &self.editor.script_error {
                ui.colored_label(egui::Color32::LIGHT_RED, e);
            }
            if !self.editor.script_output.is_empty() {
                egui::ScrollArea::vertical().max_height(120.0).id_salt("script_out").show(ui, |ui| {
                    for l in &self.editor.script_output {
                        ui.monospace(l);
                    }
                });
            }
            ui.collapsing("Reference", |ui| {
                egui::ScrollArea::vertical().max_height(220.0).id_salt("script_api").show(ui, |ui| {
                    ui.monospace(trakr_editor::SCRIPT_API);
                });
            });
        });
        self.show_console = open;
    }

    fn save_user_script(&mut self) {
        let name: String = self
            .script_name
            .trim()
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
            .collect();
        let Some(dir) = self.editor.scripts_dir() else {
            self.editor.status = "no scripts folder available".into();
            return;
        };
        if name.is_empty() {
            self.editor.status = "give the script a name first".into();
            return;
        }
        let path = dir.join(format!("{name}.lua"));
        let result = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &self.script_text));
        self.editor.status = match result {
            Ok(()) => format!("saved {}", path.display()),
            Err(e) => format!("cannot save script: {e}"),
        };
    }
}
