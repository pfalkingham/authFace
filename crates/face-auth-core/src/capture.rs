use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use libc::{c_void, mmap, munmap, PROT_READ, MAP_SHARED, MAP_FAILED};
use anyhow::Result;
use std::time::Instant;

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
        let t0 = Instant::now();
        let fd = open(device_path, OFlag::O_RDWR, Mode::empty())
            .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", device_path, e))?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        eprintln!("TIMING_CAP open: {:?}", t0.elapsed());

        // Query current format instead of setting it.
        // VIDIOC_S_FMT triggers sensor init on IR cameras (~2s delay).
        // The camera is already configured correctly, so G_FMT is instant.
        let mut fmt = make_v4l2_format(V4L2_BUF_TYPE_VIDEO_CAPTURE, 0, 0, 0);
        ioctl(fd.as_raw_fd(), VIDIOC_G_FMT, &mut fmt as *mut _ as *mut c_void)?;
        let pix: &v4l2_pix_format = unsafe { &*(fmt.raw.as_ptr() as *const v4l2_pix_format) };
        let width = pix.width;
        let height = pix.height;

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

    pub fn capture_frame(&mut self, _timeout_ms: i32) -> Result<IrFrame> {
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

        let mut frame_data = None;
        for _ in 0..3 {
            let mut buf = v4l2_buffer {
                index: 0,
                type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
                memory: V4L2_MEMORY_MMAP,
                ..unsafe { std::mem::zeroed() }
            };
            if ioctl(self.fd.as_raw_fd(), VIDIOC_DQBUF, &mut buf as *mut _ as *mut c_void).is_ok() {
                let bytes_used = buf.bytesused as usize;
                let data_slice = unsafe { std::slice::from_raw_parts(self.mmap_ptr as *const u8, bytes_used) };
                frame_data = Some(data_slice.iter().map(|&b| (b as u16) * 257).collect());

                let _ = ioctl(self.fd.as_raw_fd(), VIDIOC_QBUF, &mut buf as *mut _ as *mut c_void);
                break;
            }
        }

        if let Some(data) = frame_data {
            Ok(IrFrame { data, width: self.width, height: self.height })
        } else {
            Err(anyhow::anyhow!("Failed to capture frame (timeout)"))
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

pub fn detect_ir_camera() -> Option<String> {
    let base = std::path::Path::new("/sys/class/video4linux");
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let name_path = entry.path().join("name");
            if let Ok(name) = std::fs::read_to_string(&name_path) {
                if name.to_lowercase().contains("ir") || name.to_lowercase().contains("infrared") {
                    if let Some(device_name) = entry.file_name().to_str() {
                        return Some(format!("/dev/{}", device_name));
                    }
                }
            }
        }
    }
    None
}
