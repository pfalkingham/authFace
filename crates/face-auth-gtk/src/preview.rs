use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use face_auth_core::capture::{Camera, IrFrame};
use face_auth_core::detector::{raw_frame_has_content, FaceDetector};
use face_auth_core::inference::FaceEncoder;
use face_auth_core::preprocess::{histogram_equalize, preprocess_ir_frame};

#[derive(Clone)]
pub struct PreviewFrame {
    pub data: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub face_detected: bool,
}

pub struct CaptureController {
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    sender: mpsc::Sender<PreviewFrame>,
    pub receiver: mpsc::Receiver<PreviewFrame>,
    device: String,
    model_path: String,
    detector_model: String,
    detector_threshold: f32,
    capture_timeout: i32,
}

impl CaptureController {
    pub fn new(
        model_path: &str,
        detector_model: &str,
        detector_threshold: f32,
        device: &str,
        capture_timeout: i32,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            thread: None,
            sender: tx,
            receiver: rx,
            device: device.to_string(),
            model_path: model_path.to_string(),
            detector_model: detector_model.to_string(),
            detector_threshold,
            capture_timeout,
        }
    }

    pub fn set_device(&mut self, device: &str) {
        self.device = device.to_string();
        self.restart();
    }

    pub fn start(&mut self) {
        self.shutdown.store(false, Ordering::Relaxed);
        let shutdown = self.shutdown.clone();
        let tx = self.sender.clone();
        let device = self.device.clone();
        let model_path = self.model_path.clone();
        let detector_model = self.detector_model.clone();
        let detector_threshold = self.detector_threshold;
        let capture_timeout = self.capture_timeout;

        self.thread = Some(thread::spawn(move || {
            let mut encoder = match FaceEncoder::new(&model_path) {
                Ok(e) => e,
                Err(_) => return,
            };
            let mut detector = match FaceDetector::new(&detector_model, detector_threshold) {
                Ok(d) => d,
                Err(_) => return,
            };

            // Keep a single camera stream open for the life of the preview instead of
            // reopening (and thus restarting VIDIOC_STREAMON) on every frame — some IR
            // sensors (e.g. Logitech BRIO) interleave illuminated/dark frames, and a fresh
            // stream tends to land on the same phase every time, showing an all-black feed.
            let mut cam = match Camera::open(&device) {
                Ok(c) => c,
                Err(_) => return,
            };

            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                let frame = match cam.capture_frame(capture_timeout) {
                    Ok(f) => f,
                    Err(_) => {
                        // Try to recover from a transient capture error by reopening the device.
                        thread::sleep(Duration::from_millis(100));
                        cam = match Camera::open(&device) {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                        continue;
                    }
                };

                if !raw_frame_has_content(&frame) {
                    send_preview(&tx, &frame, false);
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }

                let mut frame = frame;
                histogram_equalize(&mut frame);

                let face_detected = match detector.detect(&frame) {
                    Ok(true) => {
                        match preprocess_ir_frame(&frame) {
                            Ok(input) => match encoder.encode(input.view()) {
                                Ok(_) => true,
                                Err(_) => false,
                            },
                            Err(_) => false,
                        }
                    }
                    _ => false,
                };

                send_preview(&tx, &frame, face_detected);
                thread::sleep(Duration::from_millis(50));
            }
        }));
    }

    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    fn restart(&mut self) {
        self.stop();
        self.start();
    }
}

impl Drop for CaptureController {
    fn drop(&mut self) {
        self.stop();
    }
}

fn send_preview(tx: &mpsc::Sender<PreviewFrame>, frame: &IrFrame, face_detected: bool) {
    let data = frame_to_rgba(frame);
    let _ = tx.send(PreviewFrame {
        data,
        width: frame.width as i32,
        height: frame.height as i32,
        face_detected,
    });
}

fn frame_to_rgba(frame: &IrFrame) -> Vec<u8> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let pixel = frame.data.get(idx).copied().unwrap_or(0);
            let v = (pixel >> 8) as u8;
            rgba.push(v);
            rgba.push(v);
            rgba.push(v);
            rgba.push(255);
        }
    }
    rgba
}
