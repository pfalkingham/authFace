//! Diagnostic: capture consecutive frames from the IR camera and report
//! per-frame statistics, plus PNG dumps, to see what the sensor actually
//! delivers frame to frame.
use face_auth_core::capture::Camera;
use face_auth_core::detector::raw_frame_has_content;

fn main() -> anyhow::Result<()> {
    let device = std::env::args().nth(1).unwrap_or_else(|| "/dev/video2".into());
    let count: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let outdir = std::env::args().nth(3).unwrap_or_else(|| "/tmp/frames".into());
    std::fs::create_dir_all(&outdir)?;

    let mut cam = Camera::open(&device)?;
    println!("{:>3}  {:>9} {:>12} {:>6} {:>6}  {:>7}", "n", "mean", "variance", "min", "max", "content");

    for n in 0..count {
        let frame = cam.capture_frame(2000)?;
        let len = frame.data.len() as f64;
        let sum: f64 = frame.data.iter().map(|&v| v as f64).sum();
        let mean = sum / len;
        let var: f64 = frame.data.iter().map(|&v| { let d = v as f64 - mean; d * d }).sum::<f64>() / len;
        let min = frame.data.iter().copied().min().unwrap_or(0);
        let max = frame.data.iter().copied().max().unwrap_or(0);
        println!("{n:>3}  {mean:>9.1} {var:>12.0} {min:>6} {max:>6}  {:>7}",
                 raw_frame_has_content(&frame));

        let img: image::ImageBuffer<image::Luma<u8>, Vec<u8>> =
            image::ImageBuffer::from_fn(frame.width, frame.height, |x, y| {
                image::Luma([(frame.data[(y * frame.width + x) as usize] >> 8) as u8])
            });
        img.save(format!("{outdir}/frame{n:02}.png"))?;
    }
    println!("\nPNGs written to {outdir}/");
    Ok(())
}
