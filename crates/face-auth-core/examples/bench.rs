//! Diagnostic: where does unlock latency go?
//!
//! Times model loading (paid once per PAM invocation, since `face-auth` is a
//! fresh process every unlock) and then each stage of the per-attempt loop.
use face_auth_core::capture::Camera;
use face_auth_core::detector::{assess_frame, FaceDetector, FrameQuality};
use face_auth_core::inference::FaceEncoder;
use face_auth_core::preprocess::{histogram_equalize, preprocess_ir_frame};
use face_auth_core::FaceAuthConfig;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let iterations: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let config = FaceAuthConfig::load()?;
    let device = config.device();
    println!("device: {device}\n");

    let t = Instant::now();
    let mut encoder = FaceEncoder::new(&config.model_path())?;
    let encoder_load = t.elapsed();

    let t = Instant::now();
    let mut detector = FaceDetector::new(&config.detector_model_path(), config.detector_threshold())?;
    let detector_load = t.elapsed();

    let t = Instant::now();
    let mut cam = Camera::open(&device)?;
    let camera_open = t.elapsed();

    println!("one-off startup cost (paid on every unlock):");
    println!("  encoder model load  {encoder_load:>9.1?}");
    println!("  detector model load {detector_load:>9.1?}");
    println!("  camera open         {camera_open:>9.1?}");
    println!(
        "  TOTAL               {:>9.1?}\n",
        encoder_load + detector_load + camera_open
    );

    println!("per-attempt breakdown:");
    println!(
        "{:>3} {:>10} {:>10} {:>10} {:>10} {:>10}  result",
        "n", "capture", "equalize", "detect", "preprocess", "encode"
    );

    let mut totals = [std::time::Duration::ZERO; 5];
    let mut usable = 0usize;

    for n in 0..iterations {
        let t = Instant::now();
        let frame = cam.capture_illuminated_frame(config.capture_timeout_ms())?;
        let t_capture = t.elapsed();

        let quality = assess_frame(&frame);
        if quality != FrameQuality::Ok {
            println!("{n:>3} {t_capture:>10.1?} {:>10} {:>10} {:>10} {:>10}  {quality}",
                     "-", "-", "-", "-");
            continue;
        }

        let mut frame = frame;
        let t = Instant::now();
        histogram_equalize(&mut frame);
        let t_eq = t.elapsed();

        let t = Instant::now();
        let found = detector.detect(&frame)?;
        let t_detect = t.elapsed();

        if !found {
            println!("{n:>3} {t_capture:>10.1?} {t_eq:>10.1?} {t_detect:>10.1?} {:>10} {:>10}  no face",
                     "-", "-");
            continue;
        }

        let t = Instant::now();
        let input = preprocess_ir_frame(&frame)?;
        let t_pre = t.elapsed();

        let t = Instant::now();
        let _embedding = encoder.encode(input.view())?;
        let t_enc = t.elapsed();

        println!("{n:>3} {t_capture:>10.1?} {t_eq:>10.1?} {t_detect:>10.1?} {t_pre:>10.1?} {t_enc:>10.1?}  face");

        totals[0] += t_capture;
        totals[1] += t_eq;
        totals[2] += t_detect;
        totals[3] += t_pre;
        totals[4] += t_enc;
        usable += 1;
    }

    if usable > 0 {
        let n = usable as u32;
        let per = |d: std::time::Duration| d / n;
        let sum: std::time::Duration = totals.iter().sum();
        println!("\nmean over {usable} successful attempts:");
        println!("  capture     {:>9.1?}", per(totals[0]));
        println!("  equalize    {:>9.1?}", per(totals[1]));
        println!("  detect      {:>9.1?}", per(totals[2]));
        println!("  preprocess  {:>9.1?}", per(totals[3]));
        println!("  encode      {:>9.1?}", per(totals[4]));
        println!("  TOTAL       {:>9.1?}  (+ scan_interval_ms between attempts)", per(sum));
    }

    Ok(())
}
