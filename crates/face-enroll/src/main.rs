use clap::Parser;
use face_auth_core::{FaceAuth, FaceAuthConfig};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(name = "face-enroll", about = "Enroll face for IR authentication")]
struct Args {
    #[arg(short, long, help = "Username to enroll")]
    user: String,
    
    #[arg(short, long, help = "Number of frames to capture", default_value = "5")]
    frames: usize,
    
    #[arg(long, help = "Interval between frames (ms)", default_value = "400")]
    interval: u64,
    
    #[arg(long, help = "Camera device path (overrides config)")]
    device: Option<String>,
    
    #[arg(long, help = "Similarity threshold", default_value = "0.6")]
    threshold: f32,
    
    #[arg(long, help = "Model path (overrides config)")]
    model: Option<String>,
    
    #[arg(long, help = "Embeddings directory (overrides config)")]
    embeddings_dir: Option<String>,
    
    #[arg(short, long, help = "Verbose output")]
    verbose: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    
    let filter = if args.verbose {
        EnvFilter::new("face_auth_core=debug")
    } else {
        EnvFilter::new("face_auth_core=info")
    };
    fmt().with_env_filter(filter).init();
    
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
    config.threshold = Some(args.threshold);
    
    println!("Using device: {}", config.device());
    println!("Using model: {}", config.model_path());
    println!("Threshold: {}", config.threshold());
    println!("Embeddings dir: {}", config.embeddings_dir().display());
    
    let mut auth = FaceAuth::new(config)?;
    auth.enroll(&args.user, args.frames, args.interval)?;
    
    Ok(())
}