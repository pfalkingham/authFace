use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use libc::{c_void, mmap, munmap, PROT_READ, PROT_WRITE, MAP_SHARED, MAP_FAILED};
use anyhow::Result;

// Correct ioctl numbers from kernel headers (x86_64)
const VIDIOC_S_FMT: u64 = 0xc0d05605;
const VIDIOC_G_FMT: u64 = 0xc0d05604;
const VIDIOC_REQBUFS: u64 = 0xc0145608;
const VIDIOC_QUERYBUF: u64 = 0xc0585609;
const VIDIOC_QBUF: u64 = 0xc058560f;
const VIDIOC_DQBUF: u64 = 0xc0585611;
const VIDIOC_STREAMON: u64 = 0x40045612;
const VIDIOC_STREAMOFF: u64 = 0x40045613;

const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const V4L2_MEMORY_MMAP: u32 = 1;
const V4L2_PIX_FMT_GREY: u32 = 0x59455247; // 'GREY'

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

pub fn capture_ir_frame(device_path: &str) -> Result<IrFrame> {
    let fd = open(device_path, OFlag::O_RDWR, Mode::empty())
        .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", device_path, e))?;
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut fmt = make_v4l2_format(
        V4L2_BUF_TYPE_VIDEO_CAPTURE,
        640, 400, V4L2_PIX_FMT_GREY,
    );
    ioctl(fd.as_raw_fd(), VIDIOC_S_FMT, &mut fmt as *mut _ as *mut c_void)?;

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
    // m union: offset is at bits 0-31, userptr/planes at bits 0-63
    let offset = buf.m as libc::off_t;

    let mmap_ptr = unsafe {
        mmap(
            std::ptr::null_mut(),
            length,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            fd.as_raw_fd(),
            offset,
        )
    };

    if mmap_ptr == MAP_FAILED {
        return Err(anyhow::anyhow!("mmap failed"));
    }

    let mut buf = v4l2_buffer {
        index: 0,
        type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
        memory: V4L2_MEMORY_MMAP,
        ..unsafe { std::mem::zeroed() }
    };
    ioctl(fd.as_raw_fd(), VIDIOC_QBUF, &mut buf as *mut _ as *mut c_void)?;
    let stream_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    ioctl(fd.as_raw_fd(), VIDIOC_STREAMON, &stream_type as *const _ as *mut c_void)?;

    let mut frame_data = None;

    for _ in 0..3 {
        let mut buf = v4l2_buffer {
            index: 0,
            type_: V4L2_BUF_TYPE_VIDEO_CAPTURE,
            memory: V4L2_MEMORY_MMAP,
            ..unsafe { std::mem::zeroed() }
        };
        if ioctl(fd.as_raw_fd(), VIDIOC_DQBUF, &mut buf as *mut _ as *mut c_void).is_ok() {
            let bytes_used = buf.bytesused as usize;
            let data_slice = unsafe { std::slice::from_raw_parts(mmap_ptr as *const u8, bytes_used) };
            frame_data = Some(data_slice.iter().map(|&b| (b as u16) * 257).collect());

            let _ = ioctl(fd.as_raw_fd(), VIDIOC_QBUF, &mut buf as *mut _ as *mut c_void);
            break;
        }
    }

    let stream_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    let _ = ioctl(fd.as_raw_fd(), VIDIOC_STREAMOFF, &stream_type as *const _ as *mut c_void);
    unsafe { munmap(mmap_ptr, length) };

    if let Some(data) = frame_data {
        let mut fmt = make_v4l2_format(V4L2_BUF_TYPE_VIDEO_CAPTURE, 0, 0, 0);
        let _ = ioctl(fd.as_raw_fd(), VIDIOC_G_FMT, &mut fmt as *mut _ as *mut c_void);
        let pix: &v4l2_pix_format = unsafe { &*(fmt.raw.as_ptr() as *const v4l2_pix_format) };

        Ok(IrFrame {
            data,
            width: pix.width,
            height: pix.height,
        })
    } else {
        Err(anyhow::anyhow!("Failed to capture frame (timeout)"))
    }
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
