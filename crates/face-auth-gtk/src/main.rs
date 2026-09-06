use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use libadwaita::prelude::*;

mod preview;
use preview::CaptureController;

const APP_ID: &str = "com.github.pfalkingham.face-auth-gtk";

struct GuiState {
    controller: RefCell<CaptureController>,
    picture: gtk4::Picture,
    status_label: gtk4::Label,
    device_row: libadwaita::ComboRow,
    camera_paths: Vec<String>,
    threshold_adj: gtk4::Adjustment,
    threshold_label: gtk4::Label,
    config_path: PathBuf,
    toast_overlay: libadwaita::ToastOverlay,
}

fn main() -> glib::ExitCode {
    let app = libadwaita::Application::builder()
        .application_id(APP_ID)
        .build();

    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &libadwaita::Application) {
    let config = load_config();
    let config_path = config_path();

    let picture = gtk4::Picture::builder()
        .halign(gtk4::Align::Fill)
        .valign(gtk4::Align::Fill)
        .hexpand(true)
        .vexpand(true)
        .build();

    let status_label = gtk4::Label::builder()
        .label("No camera feed")
        .css_classes(vec!["title-4".to_string()])
        .halign(gtk4::Align::Center)
        .build();

    let preview_overlay = gtk4::Overlay::builder()
        .child(&picture)
        .build();
    preview_overlay.add_overlay(&status_label);
    status_label.set_valign(gtk4::Align::End);
    status_label.set_margin_bottom(12);

    let preview_frame = gtk4::Frame::builder()
        .child(&preview_overlay)
        .hexpand(true)
        .vexpand(true)
        .build();

    let (camera_display, camera_paths) = enumerate_cameras();
    let camera_refs: Vec<&str> = camera_display.iter().map(|s| s.as_str()).collect();
    let camera_store = gtk4::StringList::new(&camera_refs);
    let current_device = config.device();
    let device_index = camera_paths.iter().position(|d| *d == current_device).unwrap_or(0);

    let device_row = libadwaita::ComboRow::builder()
        .title("Camera")
        .subtitle("IR camera device")
        .model(&camera_store)
        .selected(device_index as u32)
        .build();

    let threshold_adj = gtk4::Adjustment::builder()
        .lower(0.1)
        .upper(0.95)
        .step_increment(0.01)
        .page_increment(0.1)
        .value(config.threshold() as f64)
        .build();

    let threshold_label = gtk4::Label::builder()
        .label(format!("{:.2}", config.threshold()))
        .width_chars(4)
        .build();

    let threshold_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .build();
    let scale = gtk4::Scale::builder()
        .adjustment(&threshold_adj)
        .hexpand(true)
        .draw_value(false)
        .build();
    threshold_box.append(&scale);
    threshold_box.append(&threshold_label);

    let threshold_row = libadwaita::ActionRow::builder()
        .title("Similarity Threshold")
        .subtitle("Higher = stricter match")
        .activatable_widget(&scale)
        .build();
    threshold_row.add_suffix(&threshold_box);

    let enroll_button = gtk4::Button::builder()
        .label("Enroll Face")
        .css_classes(vec!["suggested-action".to_string()])
        .build();

    let improve_button = gtk4::Button::builder()
        .label("Improve Matching")
        .tooltip_text("Append new embeddings to improve recognition")
        .build();

    let test_button = gtk4::Button::builder()
        .label("Test Authentication")
        .build();

    let button_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk4::Align::Center)
        .build();
    button_box.append(&enroll_button);
    button_box.append(&improve_button);
    button_box.append(&test_button);

    let prefs_group = libadwaita::PreferencesGroup::new();
    prefs_group.add(&device_row);
    prefs_group.add(&threshold_row);

    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(24)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    content.append(&preview_frame);
    content.append(&prefs_group);
    content.append(&button_box);

    let toast_overlay = libadwaita::ToastOverlay::new();
    toast_overlay.set_child(Some(&content));

    let clamp = libadwaita::Clamp::builder()
        .child(&toast_overlay)
        .maximum_size(600)
        .build();

    let toolbar_view = libadwaita::ToolbarView::builder()
        .content(&clamp)
        .build();

    let header = libadwaita::HeaderBar::builder()
        .title_widget(&gtk4::Label::new(Some("Face Authentication")))
        .build();
    toolbar_view.add_top_bar(&header);

    let window = libadwaita::Window::builder()
        .application(app)
        .default_width(640)
        .default_height(740)
        .content(&toolbar_view)
        .build();

    let controller = CaptureController::new(
        &config.model_path(),
        &config.detector_model_path(),
        config.detector_threshold(),
        &config.device(),
        config.capture_timeout_ms(),
    );

    let state = Rc::new(GuiState {
        controller: RefCell::new(controller),
        picture,
        status_label,
        device_row,
        camera_paths,
        threshold_adj,
        threshold_label,
        config_path,
        toast_overlay,
    });

    setup_callbacks(&state, &enroll_button, &improve_button, &test_button);
    state.controller.borrow_mut().start();

    let state_clone = state.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(33), move || {
        update_preview(&state_clone);
        glib::ControlFlow::Continue
    });

    window.present();
}

fn setup_callbacks(state: &Rc<GuiState>, enroll_button: &gtk4::Button, improve_button: &gtk4::Button, test_button: &gtk4::Button) {
    let s = state.clone();
    state.device_row.connect_selected_notify(move |row| {
        let idx = row.selected() as usize;
        if let Some(device) = s.camera_paths.get(idx) {
            s.controller.borrow_mut().set_device(device);
            save_device(&s.config_path, device);
        }
    });

    let s = state.clone();
    state.threshold_adj.connect_value_changed(move |adj| {
        let val = adj.value() as f32;
        s.threshold_label.set_text(&format!("{:.2}", val));
        save_threshold(&s.config_path, val);
    });

    let s = state.clone();
    enroll_button.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        btn.set_label("Enrolling...");
        let s = s.clone();
        let btn = btn.clone();
        glib::MainContext::default().spawn_local(async move {
            s.controller.borrow_mut().stop();
            let result = run_enroll();
            match result {
                Ok(n) => show_toast(&s, &format!("Enrolled {} frames", n)),
                Err(e) => show_toast(&s, &format!("Enrollment failed: {}", e)),
            }
            s.controller.borrow_mut().start();
            btn.set_sensitive(true);
            btn.set_label("Enroll Face");
        });
    });

    let s = state.clone();
    improve_button.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        btn.set_label("Improving...");
        let s = s.clone();
        let btn = btn.clone();
        glib::MainContext::default().spawn_local(async move {
            s.controller.borrow_mut().stop();
            let result = run_improve();
            match result {
                Ok(n) => show_toast(&s, &format!("Added {} new embeddings", n)),
                Err(e) => show_toast(&s, &format!("Improve failed: {}", e)),
            }
            s.controller.borrow_mut().start();
            btn.set_sensitive(true);
            btn.set_label("Improve Matching");
        });
    });

    let s = state.clone();
    test_button.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        btn.set_label("Testing...");
        let s = s.clone();
        let btn = btn.clone();
        glib::MainContext::default().spawn_local(async move {
            s.controller.borrow_mut().stop();
            let result = run_test_auth();
            match result {
                Ok(msg) => show_toast(&s, &msg),
                Err(e) => show_toast(&s, &e),
            }
            s.controller.borrow_mut().start();
            btn.set_sensitive(true);
            btn.set_label("Test Authentication");
        });
    });
}

fn update_preview(state: &GuiState) {
    if let Ok(frame) = state.controller.borrow_mut().receiver.try_recv() {
        if frame.width > 0 && frame.height > 0 && !frame.data.is_empty() {
            let texture = gdk::MemoryTexture::new(
                frame.width,
                frame.height,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from(&frame.data),
                frame.width as usize * 4,
            );
            state.picture.set_paintable(Some(&texture));

            if frame.face_detected {
                state.status_label.set_markup(
                    "<span foreground='green' weight='bold'>✓ Face detected</span>",
                );
            } else {
                state.status_label.set_markup(
                    "<span foreground='red'>No face</span>",
                );
            }
        } else {
            state.status_label.set_text("No camera feed");
        }
    }
}

fn show_toast(state: &GuiState, msg: &str) {
    let toast = libadwaita::Toast::new(msg);
    toast.set_timeout(3);
    state.toast_overlay.add_toast(toast);
}

fn run_enroll() -> Result<usize, String> {
    let config = face_auth_core::FaceAuthConfig::load().map_err(|e| e.to_string())?;
    let _auth = face_auth_core::FaceAuth::new(config).map_err(|e| e.to_string())?;
    let user = whoami();
    let frames = 5;
    // We need a different approach — enroll uses Camera::open internally
    // For simplicity, shell out to face-enroll CLI
    let output = std::process::Command::new("face-enroll")
        .arg("--user")
        .arg(&user)
        .arg("--frames")
        .arg(frames.to_string())
        .output()
        .map_err(|e| format!("Failed to run face-enroll: {}", e))?;
    if output.status.success() {
        Ok(frames)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Enrollment failed: {}", stderr.trim()))
    }
}

fn run_improve() -> Result<usize, String> {
    let user = whoami();
    let frames = 5;
    let output = std::process::Command::new("face-enroll")
        .arg("--user")
        .arg(&user)
        .arg("--frames")
        .arg(frames.to_string())
        .arg("--improve")
        .output()
        .map_err(|e| format!("Failed to run face-enroll: {}", e))?;
    if output.status.success() {
        Ok(frames)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Improve failed: {}", stderr.trim()))
    }
}

fn run_test_auth() -> Result<String, String> {
    let config = face_auth_core::FaceAuthConfig::load().map_err(|e| e.to_string())?;
    let mut auth = face_auth_core::FaceAuth::new(config).map_err(|e| e.to_string())?;
    let user = whoami();
    match auth.authenticate_once(&user) {
        Ok(true) => Ok("Face matched ✓".to_string()),
        Ok(false) => Err("No match — face not recognized".to_string()),
        Err(e) => Err(format!("Test failed: {}", e)),
    }
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_string())
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("face-auth.toml")
}

fn load_config() -> face_auth_core::FaceAuthConfig {
    face_auth_core::FaceAuthConfig::load().unwrap_or_default()
}

fn save_device(path: &PathBuf, device: &str) {
    let content = if path.exists() {
        std::fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    };
    // Simple line-based replacement — just adds/replaces the device key
    let lines: Vec<String> = content
        .lines()
        .filter(|l| !l.trim().starts_with("device"))
        .map(|l| l.to_string())
        .collect();
    let mut out = lines.join("\n");
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("device = {:?}\n", device));
    let _ = std::fs::write(path, out);
}

fn save_threshold(path: &PathBuf, threshold: f32) {
    let content = if path.exists() {
        std::fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    };
    let lines: Vec<String> = content
        .lines()
        .filter(|l| !l.trim().starts_with("threshold"))
        .map(|l| l.to_string())
        .collect();
    let mut out = lines.join("\n");
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("threshold = {:.2}\n", threshold));
    let _ = std::fs::write(path, out);
}

/// Returns (display_names, device_paths) — parallel vectors.
/// Only IR cameras are shown; non-IR cameras are excluded since
/// the capture pipeline expects raw 8-bit greyscale (IR sensor format).
fn enumerate_cameras() -> (Vec<String>, Vec<String>) {
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") {
        for entry in entries.flatten() {
            let name_path = entry.path().join("name");
            if let Ok(name) = std::fs::read_to_string(&name_path) {
                let name = name.trim().to_string();
                if let Some(dev_name) = entry.file_name().to_str() {
                    let dev_path = format!("/dev/{}", dev_name);
                    if name.to_lowercase().contains("ir") || name.to_lowercase().contains("infrared") {
                        candidates.push((dev_path, name));
                    }
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));

    let mut display = Vec::new();
    let mut paths = Vec::new();
    for (dev_path, name) in candidates {
        if face_auth_core::capture::Camera::open(&dev_path).is_ok() {
            display.push(format!("{} ({})", dev_path, name));
            paths.push(dev_path);
        }
    }

    if paths.is_empty() {
        display.push("/dev/video0".to_string());
        paths.push("/dev/video0".to_string());
    }
    (display, paths)
}
