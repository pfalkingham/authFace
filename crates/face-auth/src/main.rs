//! PAM authentication helper, invoked via `pam_exec.so`.
//!
//! Exit 0 authenticates the user; any other status falls through to the next
//! module in the stack (normally a password prompt). Everything this process
//! reads from its environment is attacker-influenced except `PAM_USER`, which
//! `pam_exec` sets from the PAM handle itself.

use face_auth_core::{user, FaceAuth, FaceAuthConfig};
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::Instant;
use tracing_subscriber::{fmt, EnvFilter};

/// Status values consumed by the GNOME Shell scan indicator.
const STATUS_SCANNING: &str = "scanning";
const STATUS_OK: &str = "ok";
const STATUS_FAIL: &str = "fail";

/// Publish scan state to `/run/user/<uid>/face-auth-status` for the lock-screen
/// indicator extension.
///
/// This process runs as root while the target directory belongs to the user,
/// so the write must not follow a symlink: without `O_NOFOLLOW` a user could
/// point `face-auth-status` at `/etc/shadow` and have root truncate it on
/// their next unlock attempt. `O_NOFOLLOW` fails rather than following.
///
/// The file stays root-owned; the extension can still unlink it, because
/// removing a directory entry needs write permission on the directory (which
/// the user owns), not on the file.
fn write_status(info: &user::UserInfo, status: &str) {
    let dir = PathBuf::from(format!("/run/user/{}", info.uid));
    if !dir.is_dir() {
        return;
    }
    let path = dir.join("face-auth-status");

    let result = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .and_then(|mut f| f.write_all(status.as_bytes()));

    if let Err(e) = result {
        tracing::debug!("could not write scan status to {}: {e}", path.display());
    }
}

/// Refuse to authenticate a session that is not physically at this machine.
///
/// The camera is attached to the console. Without this, a remote `sudo` over
/// SSH triggers the local IR sensor, and whoever happens to be sitting at the
/// desk authenticates the remote attacker.
fn reject_remote_session() -> Result<(), String> {
    let Ok(rhost) = env::var("PAM_RHOST") else {
        return Ok(());
    };
    let rhost = rhost.trim();
    let local = rhost.is_empty()
        || rhost == "localhost"
        || rhost == "localhost.localdomain"
        || rhost == "::1"
        || rhost.starts_with("127.");
    if local {
        Ok(())
    } else {
        Err(format!("remote session from {rhost}"))
    }
}

/// A routine authentication outcome: no match, or a session this tool declines
/// to handle. Logged below the default level, because `pam_exec` relays our
/// stderr to the terminal and this would otherwise print on every failed sudo.
fn fail_auth(msg: &str) -> ! {
    tracing::info!("{msg}");
    std::process::exit(1)
}

/// A misconfiguration: PAM did not supply a user, the config is broken, a model
/// is missing, the camera is unusable. These are rare, actionable, and useless
/// if silent — an admin has to be able to see them without setting RUST_LOG.
fn fail_setup(msg: &str) -> ! {
    tracing::error!("{msg}");
    std::process::exit(1)
}

/// Interactive verification against a stored template. Prints a human-readable
/// result and exits 0 on a match, 1 otherwise.
fn run_verify(name: &str) -> ! {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("--verify reads root-owned templates; re-run with sudo or pkexec");
        std::process::exit(2);
    }
    let info = match user::lookup(name) {
        Ok(info) => info,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let config = match FaceAuthConfig::load_for_auth(&info.name) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            std::process::exit(2);
        }
    };
    let window = config.scan_duration_ms();
    let interval = config.scan_interval_ms();
    let mut auth = match FaceAuth::new(config) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("init error: {e}");
            std::process::exit(2);
        }
    };
    match auth.authenticate_scan(&info.name, window, interval) {
        Ok(true) => {
            println!("match");
            std::process::exit(0);
        }
        Ok(false) => {
            println!("no match");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}

fn main() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("face_auth_core=error,face_auth=error"));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let t0 = Instant::now();

    // `--verify <user>` is the settings GUI's "Test Authentication" path. It
    // reads the same root-owned templates the PAM path does, so it requires
    // root; it grants nothing a root caller did not already have.
    let argv: Vec<String> = env::args().skip(1).collect();
    if !argv.is_empty() {
        match argv.as_slice() {
            [flag, name] if flag == "--verify" => run_verify(name),
            _ => {
                eprintln!("usage: face-auth            (PAM mode, reads PAM_USER)");
                eprintln!("       face-auth --verify USER  (test a stored face, requires root)");
                std::process::exit(2);
            }
        }
    }

    // PAM_USER is the only authoritative identity here. USER, LOGNAME and
    // `id -un` are environment strings or the *invoking* account, not the
    // account being authenticated, and trusting them lets the wrong template
    // decide the answer.
    let username = match env::var("PAM_USER") {
        Ok(u) if !u.is_empty() => u,
        _ => fail_setup("PAM_USER is not set; refusing to guess which account to authenticate"),
    };

    // Resolve through NSS, which also rejects anything that is not a real,
    // well-formed account name before it becomes a path component.
    let info = match user::lookup(&username) {
        Ok(info) => info,
        Err(e) => fail_setup(&format!("cannot authenticate '{username}': {e}")),
    };

    if let Err(reason) = reject_remote_session() {
        fail_auth(&format!("refusing face authentication for {reason}"));
    }

    // System config only, plus a strictly-narrowing overlay from the user.
    let config = match FaceAuthConfig::load_for_auth(&info.name) {
        Ok(c) => c,
        Err(e) => fail_setup(&format!("config error: {e}")),
    };

    let scan_duration = config.scan_duration_ms();
    let scan_interval = config.scan_interval_ms();

    let mut auth = match FaceAuth::new(config) {
        Ok(a) => a,
        Err(e) => fail_setup(&format!("init error: {e}")),
    };

    tracing::debug!(
        user = %info.name,
        window_ms = scan_duration,
        interval_ms = scan_interval,
        setup = ?t0.elapsed(),
        "starting scan"
    );

    write_status(&info, STATUS_SCANNING);

    let result = auth.authenticate_scan(&info.name, scan_duration, scan_interval);
    tracing::debug!(total = ?t0.elapsed(), "scan finished");

    match result {
        Ok(true) => {
            write_status(&info, STATUS_OK);
            std::process::exit(0);
        }
        Ok(false) => {
            write_status(&info, STATUS_FAIL);
            fail_auth(&format!("face not recognised for '{}'", info.name));
        }
        Err(e) => {
            write_status(&info, STATUS_FAIL);
            fail_setup(&format!("face authentication error: {e}"));
        }
    }
}
