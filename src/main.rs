#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::collections::BTreeSet;
use directories::ProjectDirs;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use egui::{Color32, FontId, ViewportBuilder, Visuals, Panel, MenuBar};
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};

static RT: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime")
});

#[derive(Serialize)]
struct RunRequest {
    code: String,
}

#[derive(Deserialize)]
struct RunResponse {
    stdout: Option<String>,
    stderr: Option<String>,
}

const API_BASE: &str = "https://novacibes-python-running-api.hf.space";

fn python_syntax() -> Syntax {
    Syntax {
        language: "Python",
        case_sensitive: true,
        comment: "#",
        comment_multiline: ["\"\"\"", "\"\"\""],
        keywords: BTreeSet::from([
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
            "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in",
            "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
        ]),
        types: BTreeSet::from([
            "bool", "int", "float", "complex", "list", "tuple", "range", "str", "bytes", "bytearray", "memoryview", "set", "frozenset", "dict",
        ]),
        special: BTreeSet::from([
            "self", "None", "True", "False", "__init__", "__name__", "__main__",
        ]),
        hyperlinks: BTreeSet::new(),
        quotes: BTreeSet::from(['"', '\'']),
    }
}

fn monokai_theme() -> ColorTheme {
    ColorTheme {
        name: "Monokai",
        dark: true,
        bg: "#272822",
        cursor: "#f8f8f0",
        selection: "#49483e",
        comments: "#75715e",
        functions: "#a6e22e",
        keywords: "#f92672",
        literals: "#ae81ff",
        numerics: "#ae81ff",
        punctuation: "#f8f8f2",
        strs: "#e6db74",
        types: "#66d9ef",
        special: "#fd971f",
    }
}

struct EditorTab {
    path: Option<PathBuf>,
    code: String,
    modified: bool,
}

impl EditorTab {
    fn new_empty() -> Self {
        Self { path: None, code: String::new(), modified: false }
    }
    fn title(&self) -> String {
        let name = self.path.as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "Untitled".into());
        if self.modified { format!("{} *", name) } else { name }
    }
}

struct NovaCibesEditor {
    open_files: Vec<EditorTab>,
    active_tab: usize,
    output_text: String,
    status_message: String,
    running: bool,
    api_token: Option<String>,
    token_prompt_open: bool,
    temp_token: String,
    run_all_tabs: bool,
    rx: Option<mpsc::UnboundedReceiver<String>>,
    python_syntax: Syntax,
    monokai: ColorTheme,
}

impl NovaCibesEditor {
    fn new() -> Self {
        let (token, prompt) = Self::load_token();
        Self {
            open_files: vec![EditorTab::new_empty()],
            active_tab: 0,
            output_text: String::new(),
            status_message: "Idle".into(),
            running: false,
            api_token: token,
            token_prompt_open: prompt,
            temp_token: String::new(),
            run_all_tabs: false,
            rx: None,
            python_syntax: python_syntax(),
            monokai: monokai_theme(),
        }
    }

    fn token_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "novacibes", "editor")
            .map(|d| d.config_dir().join("token.txt"))
    }

    fn load_token() -> (Option<String>, bool) {
        if let Some(path) = Self::token_path() {
            if let Ok(t) = std::fs::read_to_string(&path) {
                let t = t.trim().to_string();
                if !t.is_empty() { return (Some(t), false); }
            }
        }
        (None, true)
    }

    fn save_token(token: &str) {
        if let Some(path) = Self::token_path() {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let _ = std::fs::write(&path, token);
        }
    }

    fn run_code(&mut self) {
        if self.api_token.is_none() {
            self.output_text = "Error: No API token. Enter it in the prompt.\n".into();
            return;
        }
        if self.running { return; }

        let token = self.api_token.clone().unwrap();
        let jobs: Vec<(String, String)> = if self.run_all_tabs {
            self.open_files.iter().map(|t| (t.title(), t.code.clone())).collect()
        } else {
            let t = &self.open_files[self.active_tab];
            vec![(t.title(), t.code.clone())]
        };

        self.running = true;
        self.status_message = if jobs.len() == 1 {
            format!("Running {}...", jobs[0].0)
        } else {
            format!("Running {} files...", jobs.len())
        };

        let (tx, rx) = mpsc::unbounded_channel();
        self.rx = Some(rx);

        RT.spawn(async move {
            let client = reqwest::Client::new();
            let mut results = Vec::new();
            for (title, code) in &jobs {
                let req = RunRequest { code: code.clone() };
                let resp = client.post(format!("{}/run", API_BASE))
                    .header("Authorization", format!("Bearer {}", token))
                    .header("Content-Type", "application/json")
                    .json(&req)
                    .send()
                    .await;

                let block = match resp {
                    Ok(r) => {
                        let status = r.status();
                        let body = r.text().await.unwrap_or_default();
                        if status.is_success() {
                            if let Ok(parsed) = serde_json::from_str::<RunResponse>(&body) {
                                let mut b = String::new();
                                if let Some(out) = parsed.stdout { b.push_str(&out); }
                                if let Some(err) = parsed.stderr { b.push_str(&err); }
                                if b.is_empty() { " (no output)".into() } else { b }
                            } else { body }
                        } else {
                            format!("HTTP {}: {}", status.as_u16(), body)
                        }
                    },
                    Err(e) => format!("Request failed: {}", e),
                };
                if jobs.len() > 1 {
                    results.push(format!("===== {} =====\n{}", title, block));
                } else { results.push(block); }
            }
            let _ = tx.send(results.join("\n"));
        });
    }

    fn save_active(&mut self) {
        if self.active_tab >= self.open_files.len() { return; }
        if let Some(path) = self.open_files[self.active_tab].path.clone() {
            let tab = &mut self.open_files[self.active_tab];
            if std::fs::write(&path, &tab.code).is_ok() {
                tab.modified = false;
                self.status_message = format!("Saved {}", tab.title());
            } else {
                self.status_message = "Error saving".into();
            }
        } else {
            self.save_active_as();
        }
    }

    fn save_active_as(&mut self) {
        if self.active_tab >= self.open_files.len() { return; }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Python", &["py"])
            .save_file()
        {
            let tab = &mut self.open_files[self.active_tab];
            if std::fs::write(&path, &tab.code).is_ok() {
                tab.path = Some(path);
                tab.modified = false;
                self.status_message = format!("Saved {}", tab.title());
            } else {
                self.status_message = "Save failed".into();
            }
        }
    }

    fn close_tab(&mut self, idx: usize) {
        if self.open_files.len() <= 1 {
            self.open_files = vec![EditorTab::new_empty()];
            self.active_tab = 0;
            return;
        }
        self.open_files.remove(idx);
        if self.active_tab >= self.open_files.len() {
            self.active_tab = self.open_files.len() - 1;
        }
    }
}

impl eframe::App for NovaCibesEditor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.set_visuals(Visuals::dark());

        if self.token_prompt_open {
            egui::Window::new("Enter Hugging Face API Token")
                .collapsible(false).resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    ui.label("Paste your personal access token:");
                    ui.text_edit_singleline(&mut self.temp_token);
                    if ui.button("OK").clicked() && !self.temp_token.trim().is_empty() {
                        let token = self.temp_token.trim().to_string();
                        self.api_token = Some(token.clone());
                        Self::save_token(&token);
                        self.temp_token.clear();
                        self.token_prompt_open = false;
                    }
                });
        }

        Panel::top("menu_bar").show_inside(ui, |ui| {
            MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Tab").clicked() {
                        self.open_files.push(EditorTab::new_empty());
                        self.active_tab = self.open_files.len() - 1;
                        ui.close();
                    }
                    if ui.button("Open…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("Python", &["py"]).pick_file() {
                            if let Ok(code) = std::fs::read_to_string(&path) {
                                let cur = &self.open_files[self.active_tab];
                                if !cur.modified && cur.code.is_empty() && cur.path.is_none() {
                                    let tab = &mut self.open_files[self.active_tab];
                                    tab.code = code; tab.path = Some(path); tab.modified = false;
                                } else {
                                    self.open_files.push(EditorTab { path: Some(path), code, modified: false });
                                    self.active_tab = self.open_files.len() - 1;
                                }
                            }
                        }
                        ui.close();
                    }
                    if ui.button("Save").clicked() { self.save_active(); ui.close(); }
                    if ui.button("Save As…").clicked() { self.save_active_as(); ui.close(); }
                    if ui.button("Close Tab").clicked() { self.close_tab(self.active_tab); ui.close(); }
                });
            });
        });

        Panel::top("tab_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let mut close_idx = None;
                for (i, tab) in self.open_files.iter().enumerate() {
                    let fill = if i == self.active_tab { Color32::from_rgb(60,60,60) } else { Color32::from_rgb(40,40,40) };
                    if ui.add(egui::Button::new(tab.title()).fill(fill)).clicked() { self.active_tab = i; }
                    if ui.small_button("x").clicked() { close_idx = Some(i); }
                }
                if ui.small_button("+").clicked() {
                    self.open_files.push(EditorTab::new_empty());
                    self.active_tab = self.open_files.len() - 1;
                }
                if let Some(i) = close_idx { self.close_tab(i); }
            });
        });

        Panel::right("output_panel").resizable(true).default_size(300.0).show_inside(ui, |ui| {
            ui.heading("Output");
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false;2]).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.output_text.as_str())
                    .font(FontId::monospace(13.0))
                    .interactive(false)
                    .desired_width(f32::INFINITY));
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            if self.active_tab < self.open_files.len() {
                let tab = &mut self.open_files[self.active_tab];
                let mut editor = CodeEditor::default()
                    .id_source(format!("tab_{}", self.active_tab))
                    .with_rows(25)
                    .with_fontsize(14.0)
                    .with_theme(self.monokai.clone())
                    .with_syntax(self.python_syntax.clone());

                let response = editor.show(ui, &mut tab.code);
                if response.response.changed() {
                    tab.modified = true;
                }
            }
        });

        Panel::bottom("bottom_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let can_run = !self.running && self.api_token.is_some();
                if ui.add_enabled(can_run, egui::Button::new("▶ Run")).clicked() {
                    self.run_code();
                }
                ui.checkbox(&mut self.run_all_tabs, "Run all open files");
                if self.running { ui.add(egui::Spinner::new()); }
                ui.label(&self.status_message);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear Output").clicked() { self.output_text.clear(); }
                });
            });
        });

        if let Some(rx) = &mut self.rx {
            if let Ok(result) = rx.try_recv() {
                self.output_text = result;
                self.running = false;
                self.status_message = "Idle".into();
                self.rx = None;
            }
        }
        if self.running { ctx.request_repaint(); }
    }
}

fn main() {
    let _ = &*RT;
    let opts = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([900.0, 600.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native("NovaCibes Editor", opts, Box::new(|_cc| Ok(Box::new(NovaCibesEditor::new())))).unwrap();
}
