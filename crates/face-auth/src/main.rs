use face_auth_core::{FaceAuth, FaceAuthConfig};
use tracing_subscriber::{EnvFilter, fmt};
use std::env;
use std::time::Instant;

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
    
    let config = match FaceAuthConfig::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Config error: {}", e);
            std::process::exit(1);
        }
    };
    
    eprintln!("TIMING config_load: {:?}", t1.elapsed());
    let t2 = Instant::now();
    
    let mut auth = match FaceAuth::new(config) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Init error: {}", e);
            std::process::exit(1);
        }
    };
    
    eprintln!("TIMING model_load: {:?}", t2.elapsed());
    let t3 = Instant::now();
    
    match auth.authenticate(&user) {
        Ok(true) => {
            eprintln!("TIMING authenticate: {:?}", t3.elapsed());
            eprintln!("TIMING total: {:?}", t0.elapsed());
            std::process::exit(0);
        }
        Ok(false) => {
            eprintln!("TIMING authenticate: {:?}", t3.elapsed());
            eprintln!("TIMING total: {:?}", t0.elapsed());
            eprintln!("Face verification failed for user '{}'", user);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("TIMING authenticate: {:?}", t3.elapsed());
            eprintln!("TIMING total: {:?}", t0.elapsed());
            eprintln!("Auth error: {}", e);
            std::process::exit(1);
        }
    }
}
