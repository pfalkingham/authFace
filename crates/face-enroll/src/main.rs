use clap::Parser;
use face_auth_core::{user, EnrollProgress, FaceAuth, FaceAuthConfig};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser, Debug)]
#[command(
    name = "face-enroll",
    about = "Enrol a face for IR authentication",
    after_help = "Templates live under a root-owned directory, so this must be run with sudo."
)]
struct Args {
    #[arg(short, long, help = "Username to enrol")]
    user: String,

    #[arg(short, long, help = "Number of frames to capture", default_value = "5")]
    frames: usize,

    #[arg(long, help = "Interval between frames (ms)", default_value = "400")]
    interval: u64,

    #[arg(long, help = "Camera device path (overrides config)")]
    device: Option<String>,

    #[arg(long, help = "Similarity threshold (overrides config)")]
    threshold: Option<f32>,

    #[arg(long, help = "Model path (overrides config)")]
    model: Option<String>,

    #[arg(long, help = "Embeddings directory (overrides config)")]
    embeddings_dir: Option<String>,

    #[arg(long, help = "Append to existing embeddings instead of replacing")]
    improve: bool,

    #[arg(short, long, help = "Verbose output")]
    verbose: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let filter = if args.verbose {
        EnvFilter::new("face_auth_core=debug,face_enroll=debug")
    } else {
        EnvFilter::new("face_auth_core=warn,face_enroll=info")
    };
    fmt().with_env_filter(filter).with_writer(std::io::stderr).init();

    if args.frames == 0 {
        anyhow::bail!("--frames must be at least 1");
    }

    // Resolve to the canonical account name. `getent passwd 0` succeeds and
    // would otherwise enrol into a directory literally named "0", which the
    // authentication path (looking up "root") never reads — a silent no-op.
    let info = user::lookup(&args.user)?;

    // The system template directory is root-owned 0700 so that no unprivileged
    // process can plant a face for an account. Writing there needs root; say so
    // clearly rather than failing later on EACCES. An explicit --embeddings-dir
    // is the caller's own business, so it is left alone.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 && args.embeddings_dir.is_none() {
        anyhow::bail!(
            "enrolment writes to a root-owned template store; re-run with:\n\
             \n    sudo face-enroll --user {}{}\n\
             \nOr pass --embeddings-dir to write somewhere you own.",
            info.name,
            if args.improve { " --improve" } else { "" }
        );
    }

    let mut config = FaceAuthConfig::load()?;

    if let Some(device) = args.device {
        config.device = Some(device);
    }
    if let Some(model) = args.model {
        config.model_path = Some(model);
    }
    if let Some(dir) = args.embeddings_dir {
        config.embeddings_dir = Some(dir);
    }
    if let Some(threshold) = args.threshold {
        config.threshold = Some(threshold);
    }
    config.validate()?;

    println!("Enrolling '{}'", info.name);
    println!("  camera:     {}", config.device());
    println!("  model:      {}", config.model_path());
    println!("  templates:  {}", config.embeddings_dir().display());
    println!();

    let mut auth = FaceAuth::new(config)?;

    let mut progress = |p: EnrollProgress| match p {
        EnrollProgress::Capturing { captured, wanted, attempt } => {
            println!("Capturing frame {}/{} (attempt {})...", captured + 1, wanted, attempt);
        }
        EnrollProgress::NoContent => println!("  nothing in frame, retrying..."),
        EnrollProgress::NoFace => println!("  no face detected, retrying..."),
        EnrollProgress::Captured { captured, wanted } => {
            println!("  captured {captured}/{wanted}");
        }
    };

    if args.improve {
        let (added, total) =
            auth.enroll_append(&info.name, args.frames, args.interval, &mut progress)?;
        println!("\nAdded {} embeddings for '{}' ({} total)", added, info.name, total);
    } else {
        let saved = auth.enroll(&info.name, args.frames, args.interval, &mut progress)?;
        println!("\nSaved {} embeddings for '{}'", saved, info.name);
    }

    Ok(())
}
