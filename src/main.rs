use eframe::{egui, NativeOptions};
use egui::ViewportBuilder;

struct App {
    // Future fields: editor, API client, settings, etc.
}

impl Default for App {
    fn default() -> Self {
        Self {}
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Set dark background for the whole viewport
        ctx.set_visuals(egui::Visuals::dark());
        // Override background color to rgb(30,30,30)
        let mut style = (*ctx.style()).clone();
        style.visuals.window_fill = egui::Color32::from_rgb(30, 30, 30);
        style.visuals.panel_fill = egui::Color32::from_rgb(30, 30, 30);
        ctx.set_style(style);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Editor will go here");
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([900.0, 600.0])
            .with_title("NovaCibes Editor")
            .with_resizable(true)
            .with_vsync(true),
        ..Default::default()
    };

    eframe::run_native(
        "NovaCibes Editor",
        options,
        Box::new(|_cc| Box::new(App::default())),
    )
}.lock().unwrap();
            out.clear();
            if !stdout.is_empty() {
                out.push_str("--- stdout ---\n");
                out.push_str(&stdout);
                if !stdout.ends_with('\n') {
                    out.push('\n');
                }
            }
            if !stderr.is_empty() {
                out.push_str("--- stderr ---\n");
                out.push_str(&stderr);
                if !stderr.ends_with('\n') {
                    out.push('\n');
                }
            }
            if stdout.is_empty() && stderr.is_empty() {
                out.push_str("(no output)\n");
            }
            // Schedule UI update
            app::awake();
        });
    });

    stop_btn.set_callback(move |_| {
        cancel_flag.store(true, Ordering::Relaxed);
        // UI update will happen when the thread finishes and calls awake
    });

    clear_btn.set_callback(move |_| {
        console.buffer().unwrap().set_text("");
    });

    // --- Main event loop ---
    while app.wait() {
        // Update status bar and buttons based on connection and running state
        let is_connected = connected.load(Ordering::Relaxed);
        let is_running = running.load(Ordering::Relaxed);
        if is_connected && !is_running {
            run_btn.activate();
        } else {
            run_btn.deactivate();
        }
        if !is_running {
            stop_btn.deactivate();
        }
        // Check if output_buffer has been updated and append to console
        {
            let mut out = output_buffer.lock().unwrap();
            if !out.is_empty() {
                console.buffer().unwrap().set_text(out.as_str());
                out.clear();
                running.store(false, Ordering::Relaxed);
            }
        }
        wind.redraw();
    }
}

// ---------- Helper: update line numbers ----------
fn update_line_numbers(line_numbers: &mut TextDisplay, editor: &TextEditor) {
    let text = editor.buffer().unwrap().text();
    let line_count = text.lines().count();
    let mut nums = String::new();
    for i in 1..=line_count {
        nums.push_str(&format!("{}\n", i));
    }
    // Only update if different to reduce flicker
    if nums != line_numbers.buffer().unwrap().text() {
        line_numbers.buffer().unwrap().set_text(&nums);
    }
                }    path
}

fn load_config() -> Config {
    let path = config_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Config::default()
        }
    } else {
        Config::default()
    }
}

fn save_config(config: &Config) {
    let path = config_path();
    if let Ok(json) = serde_json::to_string_pretty(config) {
        fs::write(path, json).ok();
    }
}

// ---------- Network ----------
fn check_health(token: &str) -> bool {
    ureq::get("https://novacibes-python-running-api.hf.space/health")
        .set("Authorization", &format!("Bearer {}", token))
        .timeout(std::time::Duration::from_secs(3))
        .call()
        .is_ok()
}

// ---------- Main UI ----------
fn main() {
    let app = app::App::default();
    let mut wind = Window::default()
        .with_size(800, 600)
        .with_label("NovaCibes Python Runner");
    wind.make_resizable(true);

    // Shared state
    let config: Arc<Mutex<Config>> = Arc::new(Mutex::new(load_config()));
    let connected = Arc::new(AtomicBool::new(false));

    // --- Menu bar ---
    let mut menu_bar = menu::MenuBar::new(0, 0, 800, 30, "");
    menu_bar.add(
        "&File/&Settings\t",
        Shortcut::None,
        MenuFlag::Normal,
        {
            let config = config.clone();
            let connected = connected.clone();
            move |_| {
                let token = dialog::password(400, 200, "Enter your Hugging Face token", "")
                    .unwrap_or_default();
                if token.is_empty() {
                    return;
                }
                {
                    let mut cfg = config.lock().unwrap();
                    cfg.token = token.clone();
                    save_config(&cfg);
                }
                let token = token.clone();
                let connected = connected.clone();
                thread::spawn(move || {
                    let ok = check_health(&token);
                    connected.store(ok, Ordering::Relaxed);
                    app::awake();
                });
            }
        },
    );
    menu_bar.add(
        "&Help/&About",
        Shortcut::None,
        MenuFlag::Normal,
        |_| {
            dialog::message_default("NovaCibes Python Runner v0.1\nThin client for remote Python execution.");
        },
    );

    // --- Status bar ---
    let mut status_frame = Frame::default()
        .with_size(780, 30)
        .with_pos(10, 40)
        .with_label("Status: checking...");
    status_frame.set_color(Color::from_rgb(240, 240, 240));
    status_frame.set_frame(FrameType::FlatBox);

    // Run button (disabled initially)
    let mut run_btn = fltk::button::Button::new(10, 80, 80, 30, "▶ Run");
    run_btn.deactivate();

    wind.end();
    wind.show();

    // --- Initial token / health check ---
    let initial_token = {
        let cfg = config.lock().unwrap().clone();
        cfg.token
    };

    if initial_token.is_empty() {
        let token = dialog::password(400, 200, "Welcome!\nEnter your Hugging Face token", "")
            .unwrap_or_default();
        if token.is_empty() {
            dialog::alert(400, 200, "No token provided. You can add it later via File > Settings.");
        } else {
            let mut cfg = config.lock().unwrap();
            cfg.token = token.clone();
            save_config(&cfg);
        }
    }

    let token_for_check = {
        config.lock().unwrap().token.clone()
    };
    if !token_for_check.is_empty() {
        let connected = connected.clone();
        thread::spawn(move || {
            let ok = check_health(&token_for_check);
            connected.store(ok, Ordering::Relaxed);
            app::awake();
        });
    } else {
        connected.store(false, Ordering::Relaxed);
        status_frame.set_label("Status: no token set. Go to File > Settings.");
    }

    // Main loop
    while app.wait() {
        let is_connected = connected.load(Ordering::Relaxed);
        if is_connected {
            status_frame.set_label("Status: connected to NovaCibes API");
            status_frame.set_color(Color::from_rgb(200, 255, 200));
            run_btn.activate();
        } else if !config.lock().unwrap().token.is_empty() {
            status_frame.set_label("Status: no internet or invalid token");
            status_frame.set_color(Color::from_rgb(255, 200, 200));
            run_btn.deactivate();
        }
        wind.redraw();
    }
}
