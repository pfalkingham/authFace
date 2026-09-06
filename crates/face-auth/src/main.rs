use face_auth_core::{FaceAuth, FaceAuthConfig};
use tracing_subscriber::{EnvFilter, fmt};
use std::env;
use std::path::PathBuf;
use std::time::Instant;

/// Resolve the user's runtime directory: `/run/user/<uid>/`.
fn runtime_dir_for(user: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("getent")
        .arg("passwd")
        .arg(user)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8(output.stdout).ok()?;
    let parts: Vec<&str> = line.trim().split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let uid: u32 = parts[2].parse().ok()?;
    let dir = PathBuf::from(format!("/run/user/{}", uid));
    dir.is_dir().then_some(dir)
}

/// Write a scan indicator status consumed by the GNOME Shell extension:
/// `/run/user/<uid>/face-auth-status` containing `scanning`, `ok`, or `fail`.
fn write_status(user: &str, status: &str) {
    let Some(runtime_dir) = runtime_dir_for(user) else {
        return;
    };
    let path = runtime_dir.join("face-auth-status");
    let _ = std::fs::write(&path, status.as_bytes());
}

fn main() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("face_auth_core=error"));
    fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    
    let t0 = Instant::now();
    
    let user = env::var("PAM_USER")
        .or_else(|_| env::var("USER"))
        .or_else(|_| env::var("LOGNAME"))
        .unwrap_or_else(|_| {
            std::process::Command::new("id")
                .arg("-un")
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| {
                    eprintln!("Could not determine user");
                    std::process::exit(1);
                })
        });
    
    eprintln!("TIMING user_resolve: {:?}", t0.elapsed());
    let t1 = Instant::now();
    
    let config = match FaceAuthConfig::load_for_user(Some(&user)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {}", e);
            std::process::exit(1);
        }
    };
    
    eprintln!("TIMING config_load: {:?}", t1.elapsed());
    let t2 = Instant::now();

    let scan_duration = config.scan_duration_ms();
    let scan_interval = config.scan_interval_ms();

    let mut auth = match FaceAuth::new(config) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Init error: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("TIMING model_load: {:?}", t2.elapsed());
    let t3 = Instant::now();

    eprintln!(
        "SCAN: window={}ms interval={}ms user='{}'",
        scan_duration, scan_interval, user
    );

    write_status(&user, "scanning");

    match auth.authenticate_scan(&user, scan_duration, scan_interval) {
        Ok(true) => {
            eprintln!("TIMING authenticate: {:?}", t3.elapsed());
            eprintln!("TIMING total: {:?}", t0.elapsed());
            write_status(&user, "ok");
            std::process::exit(0);
        }
        Ok(false) => {
            eprintln!("TIMING authenticate: {:?}", t3.elapsed());
            eprintln!("TIMING total: {:?}", t0.elapsed());
            eprintln!("Face verification failed for user '{}'", user);
            write_status(&user, "fail");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("TIMING authenticate: {:?}", t3.elapsed());
            eprintln!("TIMING total: {:?}", t0.elapsed());
            eprintln!("Auth error: {}", e);
            write_status(&user, "fail");
            std::process::exit(1);
        }
    }
}
