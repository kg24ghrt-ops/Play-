use fltk::{
    app, dialog,
    enums::{Color, Font, FrameType, Shortcut},
    frame::Frame,
    group::Tile,
    menu::{self, MenuFlag},
    prelude::*,
    text::{TextBuffer, TextDisplay, TextEditor, WrapMode},
    window::Window,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};

// ---------- Configuration ----------
#[derive(Serialize, Deserialize, Clone)]
struct Config {
    token: String,
}

impl Default for Config {
    fn default() -> Self {
        Self { token: String::new() }
    }
}

fn config_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("novacibes");
    fs::create_dir_all(&path).ok();
    path.push("settings.json");
    path
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

fn run_code(token: &str, code: &str, cancel: Arc<AtomicBool>) -> (String, String) {
    let resp = ureq::post("https://novacibes-python-running-api.hf.space/run")
        .set("Authorization", &format!("Bearer {}", token))
        .set("Content-Type", "application/json")
        .send_json(ureq::json!({ "code": code }));

    match resp {
        Ok(response) => {
            let mut reader = response.into_reader();
            let mut buf = [0u8; 2048];
            let mut output = Vec::new();
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return ("".to_string(), "Execution cancelled by user".to_string());
                }
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => output.extend_from_slice(&buf[..n]),
                    Err(_) => {
                        return ("".to_string(), "Error reading response".to_string());
                    }
                }
            }
            match serde_json::from_slice::<serde_json::Value>(&output) {
                Ok(val) => {
                    let stdout = val["stdout"].as_str().unwrap_or("").to_string();
                    let stderr = val["stderr"].as_str().unwrap_or("").to_string();
                    (stdout, stderr)
                }
                Err(_) => ("".to_string(), "Invalid JSON response".to_string()),
            }
        }
        Err(ureq::Error::Status(code, response)) => {
            let err_body = response.into_string().unwrap_or_default();
            ("".to_string(), format!("HTTP {}: {}", code, err_body))
        }
        Err(e) => ("".to_string(), format!("Network error: {}", e)),
    }
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
    let running = Arc::new(AtomicBool::new(false));
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let output_buffer: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

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
            dialog::message_default("NovaCibes Python Runner v0.2\nRemote Python execution client.");
        },
    );

    // --- Main layout: resizable editor + console ---
    let mut main_tile = Tile::new(10, 40, 780, 520, "");

    // ---- Editor area (with line numbers) ----
    let mut editor_group = fltk::group::Group::new(0, 0, 780, 300, "");
    editor_group.set_frame(FrameType::FlatBox);
    editor_group.set_color(Color::White);

    // Line numbers widget (read-only TextDisplay)
    let mut line_numbers = TextDisplay::new(0, 0, 40, 300, "");
    line_numbers.set_color(Color::from_rgb(240, 240, 240));
    line_numbers.set_text_font(Font::Courier);
    line_numbers.set_text_size(14);
    line_numbers.set_readonly(true);
    line_numbers.set_wrap_mode(WrapMode::None);
    line_numbers.set_scrollbar_align(fltk::enums::Align::Empty); // no scrollbar

    // Code editor (TextEditor)
    let mut editor = TextEditor::new(40, 0, 740, 300, "");
    editor.set_text_font(Font::Courier);
    editor.set_text_size(14);
    editor.set_wrap_mode(WrapMode::None);
    editor.buffer().unwrap().set_tab_distance(4);

    // Sync scroll and update line numbers
    editor.set_trigger(fltk::enums::CallbackTrigger::Changed);
    let mut editor_buf = editor.buffer().unwrap();
    let mut line_numbers = line_numbers.clone(); // clone for move
    editor_buf.add_modify_callback(move |_, _, _, _, _| {
        update_line_numbers(&mut line_numbers, &editor);
    });
    // Sync vertical scroll
    editor.set_scroll_callback({
        let line_numbers = line_numbers.clone();
        move |ed| {
            let (x, y) = ed.scroll_position();
            line_numbers.set_scroll(x, y);
        }
    });

    editor_group.end();

    // ---- Console area (output) ----
    let mut console = TextDisplay::new(0, 300, 780, 220, "");
    console.set_text_font(Font::Courier);
    console.set_text_size(13);
    console.set_readonly(true);
    console.set_wrap_mode(WrapMode::AtColumn);
    console.set_wrap_at_column(80);
    console.buffer().unwrap().set_text("Output will appear here...\n");

    main_tile.end();

    // --- Toolbar buttons ---
    let mut run_btn = fltk::button::Button::new(10, 570, 80, 25, "▶ Run");
    let mut stop_btn = fltk::button::Button::new(95, 570, 80, 25, "■ Stop");
    let mut clear_btn = fltk::button::Button::new(180, 570, 80, 25, "🗐 Clear");
    stop_btn.deactivate();
    run_btn.deactivate(); // enable after health check

    wind.end();
    wind.show();

    // --- Health check on startup ---
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
    let connected = connected.clone();
    if !token_for_check.is_empty() {
        let token = token_for_check.clone();
        thread::spawn(move || {
            let ok = check_health(&token);
            connected.store(ok, Ordering::Relaxed);
            app::awake();
        });
    }

    // --- Callbacks for buttons ---
    let editor = editor.clone(); // share editor handle
    let console = console.clone();
    let config = config.clone();
    let running = running.clone();
    let cancel_flag = cancel_flag.clone();
    let output_buffer = output_buffer.clone();

    run_btn.set_callback(move |_| {
        if running.load(Ordering::Relaxed) {
            return; // already running
        }
        let code = editor.buffer().unwrap().text();
        if code.trim().is_empty() {
            return;
        }
        // Check internet
        let token = config.lock().unwrap().token.clone();
        if !check_health(&token) {
            console.buffer().unwrap().set_text("Error: No internet or invalid token\n");
            return;
        }
        // Start run
        running.store(true, Ordering::Relaxed);
        cancel_flag.store(false, Ordering::Relaxed);
        stop_btn.activate();
        run_btn.deactivate();
        console.buffer().unwrap().set_text("Running...\n");

        let token = token.clone();
        let code = code.clone();
        let output_buffer = output_buffer.clone();
        let running = running.clone();
        let cancel_flag = cancel_flag.clone();
        let mut stop_btn = stop_btn.clone();
        let mut run_btn = run_btn.clone();
        let mut console = console.clone();

        thread::spawn(move || {
            let (stdout, stderr) = run_code(&token, &code, cancel_flag.clone());
            let mut out = output_buffer.lock().unwrap();
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
