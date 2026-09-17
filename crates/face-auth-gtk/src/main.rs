use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use libadwaita::prelude::*;

mod preview;
use preview::{CaptureController, PreviewEvent};

const APP_ID: &str = "com.github.pfalkingham.face-auth-gtk";
const ENROLL_FRAMES: usize = 5;

// Absolute paths: pkexec resolves a bare name against the *caller's* PATH, and
// these always install to /usr/local/bin even when the GUI itself went to
// ~/.local/bin on an immutable system.
const FACE_ENROLL_BIN: &str = "/usr/local/bin/face-enroll";
const FACE_AUTH_BIN: &str = "/usr/local/bin/face-auth";

struct GuiState {
    controller: RefCell<CaptureController>,
    picture: gtk4::Picture,
    status_label: gtk4::Label,
    camera_paths: Vec<String>,
    threshold_label: gtk4::Label,
    config_path: PathBuf,
    toast_overlay: libadwaita::ToastOverlay,
    username: String,
}

fn main() -> glib::ExitCode {
    let app = libadwaita::Application::builder()
        .application_id(APP_ID)
        .build();

    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &libadwaita::Application) {
    let config = face_auth_core::FaceAuthConfig::load().unwrap_or_default();
    let config_path = config_path();

    // The authentication path will not accept a threshold looser than the
    // system one, so the slider starts there rather than letting someone pick
    // a value that silently does nothing at the login screen.
    let system_floor = system_threshold_floor();

    // The account to enrol and test. Resolved from whoever invoked the app, not
    // the process UID: started with `sudo face-auth-gtk` the latter is root, and
    // the app would enrol root's face while PAM authenticates the desktop user.
    let username = match face_auth_core::user::invoking() {
        Ok(info) => info.name,
        Err(e) => {
            eprintln!("cannot determine which user to enrol: {e}");
            String::new()
        }
    };

    let picture = gtk4::Picture::builder()
        .halign(gtk4::Align::Fill)
        .valign(gtk4::Align::Fill)
        .hexpand(true)
        .vexpand(true)
        .build();

    let status_label = gtk4::Label::builder()
        .label("Starting camera…")
        .css_classes(vec!["title-4".to_string()])
        .halign(gtk4::Align::Center)
        .wrap(true)
        .build();

    let preview_overlay = gtk4::Overlay::builder().child(&picture).build();
    preview_overlay.add_overlay(&status_label);
    status_label.set_valign(gtk4::Align::End);
    status_label.set_margin_bottom(12);

    let preview_frame = gtk4::Frame::builder()
        .child(&preview_overlay)
        .hexpand(true)
        .vexpand(true)
        .build();

    let cameras = face_auth_core::capture::enumerate_ir_cameras();
    let (camera_display, camera_paths): (Vec<String>, Vec<String>) = if cameras.is_empty() {
        (vec!["No IR camera found".to_string()], vec![config.device()])
    } else {
        cameras
            .iter()
            .map(|(path, name)| (format!("{path} ({name})"), path.clone()))
            .unzip()
    };
    let camera_refs: Vec<&str> = camera_display.iter().map(|s| s.as_str()).collect();
    let camera_store = gtk4::StringList::new(&camera_refs);
    let current_device = config.device();
    let device_index = camera_paths
        .iter()
        .position(|d| *d == current_device)
        .unwrap_or(0);

    let device_row = libadwaita::ComboRow::builder()
        .title("Camera")
        .subtitle("IR camera used for face unlock")
        .model(&camera_store)
        .selected(device_index as u32)
        .build();

    let initial_threshold = config.threshold().max(system_floor);
    let threshold_adj = gtk4::Adjustment::builder()
        .lower(system_floor as f64)
        .upper(0.95)
        .step_increment(0.01)
        .page_increment(0.1)
        .value(initial_threshold as f64)
        .build();

    let threshold_label = gtk4::Label::builder()
        .label(format!("{initial_threshold:.2}"))
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
        .subtitle(format!(
            "Higher = stricter. System minimum is {system_floor:.2}; edit {} as root to go lower.",
            face_auth_core::config::SYSTEM_CONFIG_PATH
        ))
        .activatable_widget(&scale)
        .build();
    threshold_row.add_suffix(&threshold_box);

    let enroll_button = gtk4::Button::builder()
        .label("Enroll Face")
        .css_classes(vec!["suggested-action".to_string()])
        .tooltip_text("Replace your stored face with a fresh capture")
        .build();

    let improve_button = gtk4::Button::builder()
        .label("Improve Matching")
        .tooltip_text("Append new captures to improve recognition")
        .build();

    let test_button = gtk4::Button::builder()
        .label("Test Authentication")
        .tooltip_text("Check your stored face against the camera")
        .build();

    let button_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk4::Align::Center)
        .build();
    button_box.append(&enroll_button);
    button_box.append(&improve_button);
    button_box.append(&test_button);

    // Say out loud whose face this will enrol. Getting this wrong is silent and
    // costly: you enrol one account and authenticate as another, and both the
    // Enrol and Test buttons happily agree with each other.
    let account_row = libadwaita::ActionRow::builder()
        .title("Account")
        .subtitle(if username.is_empty() {
            "Unknown — enrolment and testing are disabled".to_string()
        } else if username == "root" {
            format!("{username} — this is probably not what you want; \
                     close this and run face-auth-gtk without sudo")
        } else {
            format!("{username} — Enroll, Improve and Test all apply to this account")
        })
        .build();

    let prefs_group = libadwaita::PreferencesGroup::new();
    prefs_group.add(&account_row);
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

    let toolbar_view = libadwaita::ToolbarView::builder().content(&clamp).build();

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
        &config.detector_model_path(),
        config.detector_threshold(),
        &config.device(),
    );

    let state = Rc::new(GuiState {
        controller: RefCell::new(controller),
        picture,
        status_label,
        camera_paths,
        threshold_label,
        config_path,
        toast_overlay,
        username,
    });

    setup_callbacks(
        &state,
        &device_row,
        &threshold_adj,
        &enroll_button,
        &improve_button,
        &test_button,
    );
    state.controller.borrow_mut().start();

    let state_clone = state.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(33), move || {
        update_preview(&state_clone);
        glib::ControlFlow::Continue
    });

    window.present();
}

fn setup_callbacks(
    state: &Rc<GuiState>,
    device_row: &libadwaita::ComboRow,
    threshold_adj: &gtk4::Adjustment,
    enroll_button: &gtk4::Button,
    improve_button: &gtk4::Button,
    test_button: &gtk4::Button,
) {
    let s = state.clone();
    device_row.connect_selected_notify(move |row| {
        let idx = row.selected() as usize;
        if let Some(device) = s.camera_paths.get(idx).cloned() {
            s.controller.borrow_mut().set_device(&device);
            if let Err(e) = save_setting(&s.config_path, "device", &format!("{device:?}")) {
                show_toast(&s, &format!("Could not save camera: {e}"));
            }
        }
    });

    let s = state.clone();
    threshold_adj.connect_value_changed(move |adj| {
        let val = adj.value() as f32;
        s.threshold_label.set_text(&format!("{val:.2}"));
        if let Err(e) = save_setting(&s.config_path, "threshold", &format!("{val:.2}")) {
            show_toast(&s, &format!("Could not save threshold: {e}"));
        }
    });

    connect_privileged_action(
        state,
        enroll_button,
        "Enroll Face",
        "Enrolling…",
        PrivilegedAction::Enroll,
    );
    connect_privileged_action(
        state,
        improve_button,
        "Improve Matching",
        "Improving…",
        PrivilegedAction::Improve,
    );
    connect_privileged_action(
        state,
        test_button,
        "Test Authentication",
        "Testing…",
        PrivilegedAction::Test,
    );
}

#[derive(Clone, Copy)]
enum PrivilegedAction {
    Enroll,
    Improve,
    Test,
}

impl PrivilegedAction {
    /// Build the `pkexec` command line. Enrolment writes, and verification
    /// reads, a root-owned template store, so all three need privilege.
    fn command(self, user: &str) -> Vec<String> {
        match self {
            PrivilegedAction::Enroll => vec![
                FACE_ENROLL_BIN.into(),
                "--user".into(),
                user.into(),
                "--frames".into(),
                ENROLL_FRAMES.to_string(),
            ],
            PrivilegedAction::Improve => vec![
                FACE_ENROLL_BIN.into(),
                "--user".into(),
                user.into(),
                "--frames".into(),
                ENROLL_FRAMES.to_string(),
                "--improve".into(),
            ],
            PrivilegedAction::Test => vec![FACE_AUTH_BIN.into(), "--verify".into(), user.into()],
        }
    }

    fn describe(self, status: std::process::ExitStatus, stderr: &str) -> String {
        let code = status.code().unwrap_or(-1);
        match (self, code) {
            (PrivilegedAction::Test, 0) => "Face matched".to_string(),
            (PrivilegedAction::Test, 1) => "No match — face not recognised".to_string(),
            (PrivilegedAction::Enroll, 0) => format!("Enrolled {ENROLL_FRAMES} frames"),
            (PrivilegedAction::Improve, 0) => "Added new captures".to_string(),
            // 126 and 127 are pkexec's "declined" and "not found".
            (_, 126) => "Cancelled — administrator authentication required".to_string(),
            (_, 127) => "pkexec or the helper binary was not found".to_string(),
            _ => {
                let detail = stderr.lines().last().unwrap_or("").trim();
                if detail.is_empty() {
                    format!("Failed (exit {code})")
                } else {
                    detail.to_string()
                }
            }
        }
    }
}

/// Wire a button to a `pkexec` helper invocation.
///
/// The work runs on its own thread: `Command::output()` on the main loop froze
/// the window for as long as the camera took, which for a five-frame enrolment
/// is several seconds.
fn connect_privileged_action(
    state: &Rc<GuiState>,
    button: &gtk4::Button,
    idle_label: &'static str,
    busy_label: &'static str,
    action: PrivilegedAction,
) {
    let s = state.clone();
    button.connect_clicked(move |btn| {
        if s.username.is_empty() {
            show_toast(&s, "Cannot determine the current user");
            return;
        }
        btn.set_sensitive(false);
        btn.set_label(busy_label);

        // Release the camera so the helper can open it.
        s.controller.borrow_mut().stop();
        s.status_label.set_text("Camera in use by helper…");

        let args = action.command(&s.username);
        let (tx, rx) = async_channel::bounded(1);

        std::thread::spawn(move || {
            let result = std::process::Command::new("pkexec").args(&args).output();
            let _ = tx.send_blocking(result);
        });

        let s = s.clone();
        let btn = btn.clone();
        glib::spawn_future_local(async move {
            let message = match rx.recv().await {
                Ok(Ok(output)) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    action.describe(output.status, &stderr)
                }
                Ok(Err(e)) => format!("Could not run pkexec: {e}"),
                Err(_) => "Helper produced no result".to_string(),
            };
            show_toast(&s, &message);
            s.controller.borrow_mut().start();
            btn.set_sensitive(true);
            btn.set_label(idle_label);
        });
    });
}

fn update_preview(state: &GuiState) {
    // Drain: the capture thread may have produced several frames since the
    // last tick, and only the newest is worth painting.
    let mut latest = None;
    let mut unavailable = None;
    while let Ok(event) = state.controller.borrow_mut().receiver.try_recv() {
        match event {
            PreviewEvent::Frame(f) => latest = Some(f),
            PreviewEvent::Unavailable(msg) => unavailable = Some(msg),
        }
    }

    if let Some(frame) = latest {
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
                state
                    .status_label
                    .set_markup("<span foreground='#81c784' weight='bold'>Face detected</span>");
            } else {
                state
                    .status_label
                    .set_markup("<span foreground='#e57373'>No face</span>");
            }
        }
    } else if let Some(msg) = unavailable {
        // Say why, rather than the old generic "No camera feed".
        state.status_label.set_markup(&format!(
            "<span foreground='#e57373'>{}</span>",
            glib::markup_escape_text(&msg)
        ));
    }
}

fn show_toast(state: &GuiState, msg: &str) {
    let toast = libadwaita::Toast::new(msg);
    toast.set_timeout(4);
    state.toast_overlay.add_toast(toast);
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("face-auth.toml")
}

/// Lowest threshold the authentication path will accept from a user setting.
fn system_threshold_floor() -> f32 {
    std::fs::read_to_string(face_auth_core::config::SYSTEM_CONFIG_PATH)
        .ok()
        .and_then(|s| toml::from_str::<face_auth_core::FaceAuthConfig>(&s).ok())
        .map(|c| c.threshold())
        .unwrap_or(0.6)
}

/// Replace or insert a single top-level key, preserving everything else.
///
/// Written to a temporary file and renamed: writing in place left the config
/// truncated if the process died mid-write.
fn save_setting(path: &Path, key: &str, value: &str) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();

    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| !is_assignment_of(l, key))
        .map(str::to_string)
        .collect();
    lines.push(format!("{key} = {value}"));

    let mut body = lines.join("\n");
    body.push('\n');

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// Does this line assign exactly `key`? Guards against `threshold` also
/// matching `detector_threshold`, and against commented-out examples.
fn is_assignment_of(line: &str, key: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return false;
    }
    match trimmed.split_once('=') {
        Some((lhs, _)) => lhs.trim() == key,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_the_exact_key() {
        assert!(is_assignment_of("threshold = 0.6", "threshold"));
        assert!(is_assignment_of("  threshold=0.6", "threshold"));
        assert!(!is_assignment_of("detector_threshold = 0.5", "threshold"));
        assert!(!is_assignment_of("# threshold = 0.6", "threshold"));
        assert!(!is_assignment_of("threshold_extra = 1", "threshold"));
        assert!(!is_assignment_of("some comment", "threshold"));
    }

    #[test]
    fn save_setting_preserves_other_keys_and_comments() {
        let dir = std::env::temp_dir().join(format!("face-auth-gui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("face-auth.toml");
        std::fs::write(&path, "# my config\ndevice = \"/dev/video3\"\nthreshold = 0.6\n").unwrap();

        save_setting(&path, "threshold", "0.80").unwrap();
        let out = std::fs::read_to_string(&path).unwrap();

        assert!(out.contains("# my config"));
        assert!(out.contains("device = \"/dev/video3\""));
        assert!(out.contains("threshold = 0.80"));
        assert_eq!(out.matches("threshold =").count(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
