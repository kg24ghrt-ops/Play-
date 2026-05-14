use fltk::{
    app, dialog,
    enums::{Color, FrameType, MenuBarType, Shortcut},
    frame::Frame,
    menu,
    prelude::*,
    window::Window,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
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
    let mut menu_bar = menu::MenuBar::default()
        .with_type(MenuBarType::Normal)
        .with_pos(0, 0);
    menu_bar.add(
        "&File/&Settings\t",
        Shortcut::None,
        menu::MenuFlag::Normal,
        {
            let config = config.clone();
            let connected = connected.clone();
            move |_| {
                // password now requires a default value (empty string)
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
        menu::MenuFlag::Normal,
        |_| {
            dialog::message_default("NovaCibes Python Runner v0.1\nThin client for remote Python execution.");
        },
    );
    menu_bar.end();

    // --- Status bar ---
    let mut status_frame = Frame::default()
        .with_size(800 - 20, 30)
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
