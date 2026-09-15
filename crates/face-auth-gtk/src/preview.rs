use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use face_auth_core::capture::{Camera, IrFrame};
use face_auth_core::detector::{raw_frame_has_content, FaceDetector};
use face_auth_core::preprocess::histogram_equalize;

/// Cap how long a single capture may block, so switching camera or pressing
/// Enrol does not wait out the full authentication timeout.
const PREVIEW_CAPTURE_TIMEOUT_MS: i32 = 400;
/// Pause before retrying after the camera goes away (unplugged, or in use).
const RETRY_DELAY: Duration = Duration::from_millis(500);
/// Run face detection at most this often. The preview itself streams at the
/// camera's own rate; inference every frame would peg a core for no visible
/// benefit, since the badge only has to keep up with a person moving.
const DETECT_INTERVAL: Duration = Duration::from_millis(150);

pub struct PreviewFrame {
    pub data: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub face_detected: bool,
}

pub enum PreviewEvent {
    Frame(PreviewFrame),
    /// Camera unavailable — carries a message worth showing the user.
    Unavailable(String),
}

pub struct CaptureController {
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    sender: SyncSender<PreviewEvent>,
    pub receiver: mpsc::Receiver<PreviewEvent>,
    device: String,
    detector_model: String,
    detector_threshold: f32,
}

impl CaptureController {
    pub fn new(detector_model: &str, detector_threshold: f32, device: &str) -> Self {
        // Depth 1 with try_send: the UI redraws at its own pace, and a queue
        // that grows faster than it drains only buys latency and memory.
        let (tx, rx) = mpsc::sync_channel(1);
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            thread: None,
            sender: tx,
            receiver: rx,
            device: device.to_string(),
            detector_model: detector_model.to_string(),
            detector_threshold,
        }
    }

    pub fn set_device(&mut self, device: &str) {
        self.device = device.to_string();
        self.restart();
    }

    pub fn start(&mut self) {
        if self.thread.is_some() {
            return;
        }
        self.shutdown.store(false, Ordering::Relaxed);
        let shutdown = self.shutdown.clone();
        let tx = self.sender.clone();
        let device = self.device.clone();
        let detector_model = self.detector_model.clone();
        let detector_threshold = self.detector_threshold;

        self.thread = Some(thread::spawn(move || {
            capture_loop(shutdown, tx, device, detector_model, detector_threshold);
        }));
    }

    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        // Discard anything queued so a restart does not show a stale frame.
        while self.receiver.try_recv().is_ok() {}
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

fn capture_loop(
    shutdown: Arc<AtomicBool>,
    tx: SyncSender<PreviewEvent>,
    device: String,
    detector_model: String,
    detector_threshold: f32,
) {
    let mut detector = match FaceDetector::new(&detector_model, detector_threshold) {
        Ok(d) => d,
        Err(e) => {
            let _ = tx.try_send(PreviewEvent::Unavailable(format!(
                "face detector unavailable: {e}"
            )));
            return;
        }
    };

    // The camera is opened once and held. Reopening per frame meant a full
    // open/G_FMT/REQBUFS/QUERYBUF/mmap/STREAMON cycle around twenty times a
    // second, which is most of what made the preview feel sluggish.
    let mut camera: Option<Camera> = None;
    let mut last_detect = Instant::now() - DETECT_INTERVAL;
    let mut face_detected = false;

    while !shutdown.load(Ordering::Relaxed) {
        if camera.is_none() {
            match Camera::open(&device) {
                Ok(c) => camera = Some(c),
                Err(e) => {
                    let _ = tx.try_send(PreviewEvent::Unavailable(format!("{e}")));
                    sleep_interruptible(&shutdown, RETRY_DELAY);
                    continue;
                }
            }
        }

        let captured = camera
            .as_mut()
            .expect("camera opened just above")
            .capture_frame(PREVIEW_CAPTURE_TIMEOUT_MS);

        let frame = match captured {
            Ok(f) => f,
            Err(e) => {
                // Drop the handle so the next iteration reopens the device.
                camera = None;
                face_detected = false;
                let _ = tx.try_send(PreviewEvent::Unavailable(format!("{e}")));
                sleep_interruptible(&shutdown, RETRY_DELAY);
                continue;
            }
        };

        if !raw_frame_has_content(&frame) {
            face_detected = false;
            send(&tx, &frame, false);
            continue;
        }

        let mut frame = frame;
        histogram_equalize(&mut frame);

        if last_detect.elapsed() >= DETECT_INTERVAL {
            // Detection alone answers "is there a face". The old code also ran
            // the full 512-d encoder here and threw the embedding away.
            face_detected = detector.detect(&frame).unwrap_or(false);
            last_detect = Instant::now();
        }

        send(&tx, &frame, face_detected);
    }
}

/// Sleep in slices so shutdown is noticed promptly.
fn sleep_interruptible(shutdown: &AtomicBool, total: Duration) {
    let step = Duration::from_millis(50);
    let mut left = total;
    while !left.is_zero() && !shutdown.load(Ordering::Relaxed) {
        let nap = step.min(left);
        thread::sleep(nap);
        left -= nap;
    }
}

fn send(tx: &SyncSender<PreviewEvent>, frame: &IrFrame, face_detected: bool) {
    let event = PreviewEvent::Frame(PreviewFrame {
        data: frame_to_rgba(frame),
        width: frame.width as i32,
        height: frame.height as i32,
        face_detected,
    });
    // Drop this frame rather than block if the UI has not taken the last one.
    match tx.try_send(event) {
        Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
    }
}

fn frame_to_rgba(frame: &IrFrame) -> Vec<u8> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let v = frame.data.get(y * w + x).map_or(0, |&p| (p >> 8) as u8);
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    rgba
}
