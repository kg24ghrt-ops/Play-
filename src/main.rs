#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::collections::BTreeSet;
use directories::ProjectDirs;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
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

const MONOKAI: ColorTheme = ColorTheme {
    name: "Monokai",
    dark: true,
    bg: "#272822",
    cursor: "#f8f8f2",
    selection: "#49483e",
    comments: "#75715e",
    functions: "#a6e22e",
    keywords: "#f92672",
    literals: "#ae81ff",
    numerics: "#ae81ff",
    punctuation: "#f8f8f2",
    special: "#66d9ef",
    strs: "#e6db74",
    types: "#66d9ef",
};

fn python_syntax() -> Syntax {
    Syntax {
        language: "Python",
        case_sensitive: true,
        comment: "#",
        comment_multiline: ["\"\"\"", "\"\"\""],
        hyperlinks: BTreeSet::from(["http", "https"]),
        keywords: BTreeSet::from([
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while", "with", "yield",
        ]),
        types: BTreeSet::from([
            "bool", "dict", "float", "int", "list", "set", "str", "tuple", "object",
        ]),
        special: BTreeSet::from(["self", "cls", "print", "len", "range", "enumerate"]),
        quotes: BTreeSet::from(['\'', '"']),
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
        let payload_code = if self.run_all_tabs {
            self.open_files.iter()
                .map(|t| format!("# File: {}\n{}\n", t.title(), t.code))
                .collect::<Vec<_>>()
                .join("\n# ---\n\n")
        } else {
            self.open_files[self.active_tab].code.clone()
        };

        self.running = true;
        self.status_message = "Running...".into();

        let (tx, rx) = mpsc::unbounded_channel();
        self.rx = Some(rx);

        RT.spawn(async move {
            let client = reqwest::Client::new();
            let req = RunRequest { code: payload_code };
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
                            let out = parsed.stdout.unwrap_or_default();
                            let err = parsed.stderr.unwrap_or_default();
                            format!("{}{}", out, err)
                        } else { body }
                    } else {
                        format!("HTTP {}: {}", status.as_u16(), body)
                    }
                },
                Err(e) => format!("Request failed: {}", e),
            };
            let _ = tx.send(block);
        });
    }

    fn save_active(&mut self) {
        if self.active_tab >= self.open_files.len() { return; }
        let tab = &mut self.open_files[self.active_tab];
        if let Some(path) = &tab.path {
            if std::fs::write(path, &tab.code).is_ok() {
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

    fn close_active_tab(&mut self) {
        if self.open_files.len() <= 1 {
            self.open_files = vec![EditorTab::new_empty()];
            self.active_tab = 0;
            return;
        }
        self.open_files.remove(self.active_tab);
        if self.active_tab >= self.open_files.len() {
            self.active_tab = self.active_tab.saturating_sub(1);
        }
    }
}

impl eframe::App for NovaCibesEditor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().set_visuals(egui::Visuals::dark());

        if self.token_prompt_open {
            egui::Window::new("Enter Hugging Face API Token")
                .collapsible(false).resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.label("Paste your personal access token:");
                    ui.text_edit_singleline(&mut self.temp_token);
                    if ui.button("OK").clicked() && !self.temp_token.trim().is_empty() {
                        self.api_token = Some(self.temp_token.trim().to_string());
                        Self::save_token(self.temp_token.trim());
                        self.temp_token.clear();
                        self.token_prompt_open = false;
                    }
                });
        }

        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
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
                    if ui.button("Close Tab").clicked() { self.close_active_tab(); ui.close(); }
                });
            });
        });

        egui::Panel::top("tab_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let mut close_idx = None;
                for (i, tab) in self.open_files.iter().enumerate() {
                    let fill = if i == self.active_tab { egui::Color32::from_rgb(60,60,60) } else { egui::Color32::from_rgb(40,40,40) };
                    if ui.add(egui::Button::new(tab.title()).fill(fill)).clicked() { self.active_tab = i; }
                    if ui.small_button("x").clicked() { close_idx = Some(i); }
                }
                if ui.small_button("+").clicked() {
                    self.open_files.push(EditorTab::new_empty());
                    self.active_tab = self.open_files.len() - 1;
                }
                if let Some(i) = close_idx {
                    self.active_tab = i;
                    self.close_active_tab();
                }
            });
        });

        egui::Panel::bottom("bottom_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let can_run = !self.running && self.api_token.is_some();
                if ui.add_enabled(can_run, egui::Button::new("▶ Run")).clicked() {
                    self.run_code();
                }
                ui.checkbox(&mut self.run_all_tabs, "Run all open files");
                if self.running { ui.add(egui::Spinner::new()); }
                ui.label(&self.status_message);
            });
        });

        egui::Panel::right("output_panel").resizable(true).default_size(300.0).show_inside(ui, |ui| {
            ui.heading("Output");
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false;2]).show(ui, |ui| {
                let mut out = self.output_text.clone();
                ui.add(egui::TextEdit::multiline(&mut out)
                    .font(egui::FontId::monospace(13.0))
                    .interactive(false)
                    .desired_width(f32::INFINITY));
            });
            if ui.button("Clear Output").clicked() { self.output_text.clear(); }
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            if self.active_tab < self.open_files.len() {
                let syntax = python_syntax();
                let tab = &mut self.open_files[self.active_tab];
                let mut code = tab.code.clone();

                CodeEditor::default()
                    .id_source(format!("tab_{}", self.active_tab))
                    .with_theme(MONOKAI)
                    .with_syntax(syntax)
                    .with_fontsize(14.0)
                    .show(ui, &mut code);

                if code != tab.code {
                    tab.code = code;
                    tab.modified = true;
                }
            }
        });

        if let Some(rx) = &mut self.rx {
            if let Ok(result) = rx.try_recv() {
                self.output_text = result;
                self.running = false;
                self.status_message = "Idle".into();
                self.rx = None;
            }
        }
        if self.running { ui.ctx().request_repaint(); }
    }
}

fn main() {
    let _ = &*RT;
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 600.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native("NovaCibes Editor", opts, Box::new(|_cc| Ok(Box::new(NovaCibesEditor::new())))).unwrap();
}
