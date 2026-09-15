use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use libc::{c_void, mmap, munmap, pollfd, PROT_READ, MAP_SHARED, MAP_FAILED, POLLIN};
use anyhow::Result;

// Correct ioctl numbers from kernel headers (x86_64)
// NOTE: This module is x86_64-only. ioctl numbers and struct layouts are ABI-dependent.
// For ARM/aarch64 support, replace with the `v4l` or `v4l2-sys` crates.
const VIDIOC_G_FMT: u64 = 0xc0d05604;
const VIDIOC_REQBUFS: u64 = 0xc0145608;
const VIDIOC_QUERYBUF: u64 = 0xc0585609;
const VIDIOC_QBUF: u64 = 0xc058560f;
const VIDIOC_DQBUF: u64 = 0xc0585611;
const VIDIOC_STREAMON: u64 = 0x40045612;
const VIDIOC_STREAMOFF: u64 = 0x40045613;

const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const V4L2_MEMORY_MMAP: u32 = 1;

/// v4l2_fourcc('G','R','E','Y') — 8-bit greyscale, what Windows Hello IR
/// sensors expose. The capture path widens each byte to u16 directly, so any
/// other format (YUYV, MJPEG) would be fed to the model as noise.
const V4L2_PIX_FMT_GREY: u32 = 0x5945_5247;

/// Refuse implausible geometry from `G_FMT` before it sizes an allocation.
const MAX_DIMENSION: u32 = 8192;

// Kernel struct v4l2_format: type(4) + padding(4) + union raw_data[200] = 208 bytes
#[repr(C)]
struct v4l2_format {
    type_: u32,
    _pad: [u8; 4],
    raw: [u8; 200],
}

fn make_v4l2_format(type_: u32, width: u32, height: u32, pixelformat: u32) -> v4l2_format {
    let mut fmt = v4l2_format {
        type_,
        _pad: [0; 4],
        raw: [0; 200],
    };
    let pix: &mut v4l2_pix_format = unsafe { &mut *(fmt.raw.as_mut_ptr() as *mut v4l2_pix_format) };
    pix.width = width;
    pix.height = height;
    pix.pixelformat = pixelformat;
    fmt
}

#[repr(C)]
struct v4l2_pix_format {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[repr(C)]
struct v4l2_requestbuffers {
    count: u32,
    type_: u32,
    memory: u32,
    reserved: [u32; 2],
}

// Kernel struct v4l2_buffer: 88 bytes
#[repr(C)]
struct v4l2_buffer {
    index: u32,
    type_: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp: [i64; 2],      // struct timeval
    timecode: [u32; 4],       // struct v4l2_timecode: type, flags, frames, seconds, minutes, hours, userbits[4] → packed as 4 u32
    sequence: u32,
    memory: u32,
    m: u64,                   // union { offset, userptr, planes, fd }
    length: u32,
    reserved2: u32,
    reserved: u32,
}

pub struct IrFrame {
    pub data: Vec<u16>,
    pub width: u32,
    pub height: u32,
}

pub struct Camera {
    fd: OwnedFd,
    mmap_ptr: *mut c_void,
    length: usize,
    width: u32,
    height: u32,
    stream_on: bool,
}

unsafe impl Send for Camera {}

impl Camera {
    pub fn open(device_path: &str) -> Result<Self> {
        let fd = open(device_path, OFlag::O_RDWR, Mode::empty())
            .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", device_path, e))?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };

        // Query current format instead of setting it.
        // VIDIOC_S_FMT triggers sensor init on IR cameras (~2s delay).
        // The camera is already configured correctly, so G_FMT is instant.
        let mut fmt = make_v4l2_format(V4L2_BUF_TYPE_VIDEO_CAPTURE, 0, 0, 0);
        ioctl(fd.as_raw_fd(), VIDIOC_G_FMT, &mut fmt as *mut _ as *mut c_void)?;
        let pix: &v4l2_pix_format = unsafe { &*(fmt.raw.as_ptr() as *const v4l2_pix_format) };
        let width = pix.width;
        let height = pix.height;
        let pixelformat = pix.pixelformat;

        // Validate what the driver reports rather than assuming it. Pointed at
        // an ordinary RGB webcam this would otherwise silently reinterpret
        // YUYV or MJPEG bytes as greyscale and compare the noise against a
        // real template.
        if pixelformat != V4L2_PIX_FMT_GREY {
            let fourcc: String = pixelformat
                .to_le_bytes()
                .iter()
                .map(|&b| if b.is_ascii_graphic() { b as char } else { '?' })
                .collect();
            return Err(anyhow::anyhow!(
                "{} reports pixel format '{}' ({:#010x}); this tool requires raw 8-bit GREY \
                 from an IR sensor",
                device_path,
                fourcc,
                pixelformat
            ));
        }
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(anyhow::anyhow!(
                "{} reports implausible frame geometry {}x{}",
                device_path,
                width,
                height
            ));
        }

        let mut reqbuf = v4l2_requestbuffers {
            count: 1,
            type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
            memory: V4L2_MEMORY_MMAP,
            reserved: [0, 0],
        };
        ioctl(fd.as_raw_fd(), VIDIOC_REQBUFS, &mut reqbuf as *mut _ as *mut c_void)?;

        let mut buf = v4l2_buffer {
            index: 0,
            type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
            memory: V4L2_MEMORY_MMAP,
            ..unsafe { std::mem::zeroed() }
        };
        ioctl(fd.as_raw_fd(), VIDIOC_QUERYBUF, &mut buf as *mut _ as *mut c_void)?;

        let length = buf.length as usize;
        let offset = buf.m as libc::off_t;

        let mmap_ptr = unsafe {
            mmap(
                std::ptr::null_mut(),
                length,
                PROT_READ,
                MAP_SHARED,
                fd.as_raw_fd(),
                offset,
            )
        };
        if mmap_ptr == MAP_FAILED {
            return Err(anyhow::anyhow!("mmap failed"));
        }

        Ok(Self { fd, mmap_ptr, length, width, height, stream_on: false })
    }

    pub fn capture_frame(&mut self, timeout_ms: i32) -> Result<IrFrame> {
        if !self.stream_on {
            let buf = v4l2_buffer {
                index: 0,
                type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
                memory: V4L2_MEMORY_MMAP,
                ..unsafe { std::mem::zeroed() }
            };
            ioctl(self.fd.as_raw_fd(), VIDIOC_QBUF, &buf as *const _ as *mut c_void)?;
            let stream_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            ioctl(self.fd.as_raw_fd(), VIDIOC_STREAMON, &stream_type as *const _ as *mut c_void)?;
            self.stream_on = true;
        }

        // Use poll() to wait for data with the configured timeout
        let mut pfd = pollfd {
            fd: self.fd.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        };
        let poll_ret = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if poll_ret < 0 {
            return Err(anyhow::anyhow!("poll failed: {}", std::io::Error::last_os_error()));
        }
        if poll_ret == 0 {
            return Err(anyhow::anyhow!("Capture timed out after {}ms", timeout_ms));
        }

        let mut buf = v4l2_buffer {
            index: 0,
            type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
            memory: V4L2_MEMORY_MMAP,
            ..unsafe { std::mem::zeroed() }
        };
        if ioctl(self.fd.as_raw_fd(), VIDIOC_DQBUF, &mut buf as *mut _ as *mut c_void).is_ok() {
            // `bytesused` comes from the driver and is not trusted to fit the
            // mapping: this is the one place external data sizes an unsafe
            // read, and an oversized value would run off the end of the mmap.
            let bytes_used = (buf.bytesused as usize).min(self.length);
            if bytes_used < buf.bytesused as usize {
                tracing::warn!(
                    reported = buf.bytesused,
                    mapped = self.length,
                    "driver reported more bytes than the buffer holds; truncating"
                );
            }
            let data_slice =
                unsafe { std::slice::from_raw_parts(self.mmap_ptr as *const u8, bytes_used) };
            let data: Vec<u16> = data_slice.iter().map(|&b| (b as u16) * 257).collect();

            let _ = ioctl(self.fd.as_raw_fd(), VIDIOC_QBUF, &mut buf as *mut _ as *mut c_void);

            let expected = self.width as usize * self.height as usize;
            if data.len() < expected {
                return Err(anyhow::anyhow!(
                    "short frame: got {} bytes, expected {} for {}x{} GREY",
                    data.len(),
                    expected,
                    self.width,
                    self.height
                ));
            }

            Ok(IrFrame { data, width: self.width, height: self.height })
        } else {
            let err = std::io::Error::last_os_error();
            // Requeue so the next call has a buffer to fill; otherwise every
            // subsequent poll in a scan window just times out.
            let requeue = v4l2_buffer {
                index: 0,
                type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
                memory: V4L2_MEMORY_MMAP,
                ..unsafe { std::mem::zeroed() }
            };
            let _ = ioctl(
                self.fd.as_raw_fd(),
                VIDIOC_QBUF,
                &requeue as *const _ as *mut c_void,
            );
            Err(anyhow::anyhow!("Failed to capture frame: {}", err))
        }
    }

    fn stop_stream(&mut self) {
        if self.stream_on {
            let stream_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            let _ = ioctl(self.fd.as_raw_fd(), VIDIOC_STREAMOFF, &stream_type as *const _ as *mut c_void);
            self.stream_on = false;
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.stop_stream();
        unsafe { munmap(self.mmap_ptr, self.length) };
    }
}

pub fn capture_ir_frame(device_path: &str, timeout_ms: i32) -> Result<IrFrame> {
    let mut cam = Camera::open(device_path)?;
    let frame = cam.capture_frame(timeout_ms)?;
    cam.stop_stream();
    Ok(frame)
}

fn ioctl(fd: i32, request: u64, arg: *mut c_void) -> Result<i32> {
    let ret = unsafe { libc::ioctl(fd, request as libc::Ioctl, arg) };
    if ret < 0 {
        Err(anyhow::anyhow!("ioctl failed: {}", std::io::Error::last_os_error()))
    } else {
        Ok(ret)
    }
}

/// Does this V4L2 device name look like an IR sensor?
///
/// Matched on word boundaries rather than as a substring: plain `contains("ir")`
/// also fires on "Virtual Camera" (v4l2loopback) and "BRIO", either of which
/// would be picked as the authentication camera ahead of the real IR sensor.
pub fn name_suggests_ir(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == "ir" || word == "infrared")
}

/// Enumerate IR capture devices, best candidate first.
///
/// UVC cameras commonly expose several `/dev/videoN` nodes under one name —
/// typically a capture node followed by a metadata node that accepts no
/// frames. Each candidate is opened to confirm it really is a GREY capture
/// device before being offered.
pub fn enumerate_ir_cameras() -> Vec<(String, String)> {
    let base = std::path::Path::new("/sys/class/video4linux");
    let mut candidates: Vec<(String, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let Ok(name) = std::fs::read_to_string(entry.path().join("name")) else {
                continue;
            };
            let name = name.trim().to_string();
            if !name_suggests_ir(&name) {
                continue;
            }
            let Some(device_name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            candidates.push((format!("/dev/{}", device_name), name));
        }
    }
    // Sort numerically: /dev/video9 must order before /dev/video10.
    candidates.sort_by_key(|(path, _)| {
        (
            path.trim_start_matches("/dev/video")
                .parse::<u32>()
                .unwrap_or(u32::MAX),
            path.clone(),
        )
    });
    candidates
        .into_iter()
        .filter(|(path, _)| Camera::open(path).is_ok())
        .collect()
}

pub fn detect_ir_camera() -> Option<String> {
    enumerate_ir_cameras()
        .into_iter()
        .next()
        .map(|(path, _)| path)
}

/// Is `path` a real IR capture device on this machine?
///
/// Lets an unprivileged setting name *which* IR sensor to use without letting
/// it name an arbitrary video source: the device must live under `/dev`, carry
/// an IR-looking name in sysfs, and open as a GREY capture node. Selecting
/// among the sensors physically present is a preference; pointing the
/// authentication camera at some other stream is not.
pub fn is_ir_capture_device(path: &str) -> bool {
    let Some(node) = path.strip_prefix("/dev/") else {
        return false;
    };
    if node.is_empty() || node.contains('/') || node.contains("..") {
        return false;
    }

    let name_path = std::path::Path::new("/sys/class/video4linux")
        .join(node)
        .join("name");
    let Ok(name) = std::fs::read_to_string(name_path) else {
        return false;
    };
    if !name_suggests_ir(name.trim()) {
        return false;
    }

    // Camera::open enforces the GREY pixel format and sane geometry.
    Camera::open(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_ir_camera_names() {
        assert!(name_suggests_ir("Integrated IR Camera"));
        assert!(name_suggests_ir("Chicony USB2.0 Camera: Infrared"));
        assert!(name_suggests_ir("ir-camera"));
        assert!(name_suggests_ir("IR"));
    }

    #[test]
    fn does_not_match_ir_inside_unrelated_words() {
        // These are the cases plain substring matching got wrong.
        assert!(!name_suggests_ir("Virtual Camera"));
        assert!(!name_suggests_ir("Logitech BRIO"));
        assert!(!name_suggests_ir("Integrated Webcam"));
        assert!(!name_suggests_ir("Mirror Cam"));
    }
}
