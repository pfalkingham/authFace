use crate::capture::IrFrame;
use image::{DynamicImage, ImageBuffer, Luma};
use tract_onnx::prelude::tract_ndarray::Array3;

const ENCODER_SIZE: usize = 112;
const HISTOGRAM_BINS: usize = 65536;

/// Wrap a captured frame as a 16-bit greyscale image.
///
/// Shared by the detector and encoder paths so the geometry check lives in
/// one place: `ImageBuffer::from_fn` indexes `width * height` pixels, and a
/// frame shorter than that would otherwise be silently zero-padded into a
/// picture that is part real and part black.
pub fn frame_to_luma16(frame: &IrFrame) -> anyhow::Result<ImageBuffer<Luma<u16>, Vec<u16>>> {
    let width = frame.width;
    let height = frame.height;
    anyhow::ensure!(width > 0 && height > 0, "frame has zero extent");

    let expected = width as usize * height as usize;
    anyhow::ensure!(
        frame.data.len() >= expected,
        "frame holds {} samples, expected {} for {}x{}",
        frame.data.len(),
        expected,
        width,
        height
    );

    Ok(ImageBuffer::from_fn(width, height, |x, y| {
        Luma([frame.data[(y * width + x) as usize]])
    }))
}

/// Resize and normalise a frame into the encoder's `[3, 112, 112]` input.
///
/// The arithmetic here defines what an enrolled embedding means; changing it
/// invalidates every template already on disk.
pub fn preprocess_ir_frame(frame: &IrFrame) -> anyhow::Result<Array3<f32>> {
    let img_buffer = frame_to_luma16(frame)?;

    let dynamic_img = DynamicImage::ImageLuma16(img_buffer);
    let resized = dynamic_img.resize_exact(
        ENCODER_SIZE as u32,
        ENCODER_SIZE as u32,
        image::imageops::FilterType::Lanczos3,
    );
    let gray_img = resized.to_luma16();

    let mut array = Array3::<f32>::zeros((3, ENCODER_SIZE, ENCODER_SIZE));

    for y in 0..ENCODER_SIZE {
        for x in 0..ENCODER_SIZE {
            let pixel = gray_img.get_pixel(x as u32, y as u32).0[0] as f32 / 65535.0;
            let normalized = (pixel - 0.5) / 0.5;
            for c in 0..3usize {
                array[[c, y, x]] = normalized;
            }
        }
    }

    Ok(array)
}

/// Flatten the frame's intensity distribution so the encoder sees consistent
/// contrast regardless of how brightly the IR illuminator lit the scene.
///
/// The lookup tables are heap-allocated: as stack arrays they were 512 KB per
/// call, which is most of a spawned thread's default stack.
pub fn histogram_equalize(frame: &mut IrFrame) {
    if frame.data.is_empty() {
        return;
    }

    let mut hist = vec![0u32; HISTOGRAM_BINS];
    for &val in &frame.data {
        hist[val as usize] += 1;
    }

    let total = frame.data.len() as f32;
    let mut cdf = vec![0f32; HISTOGRAM_BINS];
    let mut sum = 0f32;
    for i in 0..HISTOGRAM_BINS {
        sum += hist[i] as f32;
        cdf[i] = sum / total;
    }

    for val in &mut frame.data {
        *val = (cdf[*val as usize] * 65535.0) as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(data: Vec<u16>, width: u32, height: u32) -> IrFrame {
        IrFrame { data, width, height }
    }

    #[test]
    fn rejects_frame_shorter_than_its_geometry() {
        // Previously padded with zeros, producing a half-black image that the
        // encoder would happily turn into an embedding.
        let f = frame(vec![0; 10], 32, 32);
        assert!(frame_to_luma16(&f).is_err());
        assert!(preprocess_ir_frame(&f).is_err());
    }

    #[test]
    fn rejects_zero_extent_frame() {
        assert!(frame_to_luma16(&frame(vec![], 0, 0)).is_err());
    }

    #[test]
    fn accepts_exactly_sized_frame() {
        let f = frame(vec![1234; 32 * 32], 32, 32);
        let img = frame_to_luma16(&f).unwrap();
        assert_eq!(img.dimensions(), (32, 32));
    }

    #[test]
    fn accepts_frame_with_trailing_padding() {
        // Some drivers report bytesused beyond the visible image.
        let f = frame(vec![7; 32 * 32 + 64], 32, 32);
        assert!(frame_to_luma16(&f).is_ok());
    }

    #[test]
    fn preprocess_produces_encoder_shaped_input() {
        let f = frame((0..64 * 64).map(|i| (i % 65536) as u16).collect(), 64, 64);
        let arr = preprocess_ir_frame(&f).unwrap();
        assert_eq!(arr.shape(), &[3, ENCODER_SIZE, ENCODER_SIZE]);
        assert!(arr.iter().all(|v| v.is_finite() && (-1.0..=1.0).contains(v)));
    }

    #[test]
    fn equalization_spreads_a_narrow_range() {
        let mut f = frame(vec![100, 100, 200, 200, 300, 300, 400, 400], 4, 2);
        histogram_equalize(&mut f);
        // The brightest input bin maps to full scale.
        assert_eq!(*f.data.iter().max().unwrap(), 65535);
        assert!(f.data.iter().any(|&v| v < 65535));
    }

    #[test]
    fn equalization_handles_empty_frame() {
        let mut f = frame(vec![], 0, 0);
        histogram_equalize(&mut f);
        assert!(f.data.is_empty());
    }
}
