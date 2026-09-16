use crate::capture::IrFrame;
use image::{DynamicImage, ImageBuffer, Luma};
use tract_onnx::prelude::tract_ndarray::Array3;

const ENCODER_SIZE: usize = 112;

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

/// Default CLAHE parameters. `CLIP_LIMIT` follows OpenCV's convention: the
/// per-bin ceiling is `clip * tile_pixels / 256`, with the clipped mass
/// redistributed. Howdy uses 2.0; 3.0 measured better on this sensor's frames
/// without visibly amplifying noise.
pub const CLAHE_CLIP_LIMIT: f32 = 3.0;
pub const CLAHE_TILES: u32 = 8;

/// Contrast-limited adaptive histogram equalisation.
///
/// Replaces the global equalisation this used to do. Global equalisation maps
/// one CDF over the whole frame, so a dark IR frame — where nearly all samples
/// sit in a narrow band — gets that band stretched across the full range,
/// turning sensor noise into hard posterised contours. Measured on this
/// camera, the face detector scored ~0.11 on globally-equalised frames (below
/// its 0.5 threshold, indistinguishable from an empty room) and 0.60-0.99 on
/// the same frames under CLAHE.
///
/// CLAHE instead equalises per tile with a ceiling on how much any one
/// intensity may be amplified, then bilinearly interpolates between
/// neighbouring tiles' mappings so no tile seams appear.
///
/// Samples are u16 carrying 8-bit data widened by 257 (see `capture_frame`),
/// so 256 bins are exact here.
pub fn clahe_equalize(frame: &mut IrFrame, clip_limit: f32, tiles: u32) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if frame.data.is_empty() || w == 0 || h == 0 || frame.data.len() < w * h {
        return;
    }
    let tiles = tiles.max(1) as usize;
    let tile_w = w.div_ceil(tiles);
    let tile_h = h.div_ceil(tiles);

    // One 256-entry lookup table per tile.
    let mut luts = vec![[0u8; 256]; tiles * tiles];
    for ty in 0..tiles {
        for tx in 0..tiles {
            let x0 = tx * tile_w;
            let y0 = ty * tile_h;
            let x1 = (x0 + tile_w).min(w);
            let y1 = (y0 + tile_h).min(h);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }

            let mut hist = [0u32; 256];
            for y in y0..y1 {
                for x in x0..x1 {
                    hist[(frame.data[y * w + x] >> 8) as usize] += 1;
                }
            }

            // Clip, then hand the excess back evenly. This is what bounds the
            // contrast gain and stops noise being amplified without limit.
            let count = ((x1 - x0) * (y1 - y0)) as f32;
            let limit = ((clip_limit * count) / 256.0).max(1.0) as u32;
            let mut excess: u32 = 0;
            for b in hist.iter_mut() {
                if *b > limit {
                    excess += *b - limit;
                    *b = limit;
                }
            }
            let share = excess / 256;
            let mut remainder = excess % 256;
            for b in hist.iter_mut() {
                *b += share;
                if remainder > 0 {
                    *b += 1;
                    remainder -= 1;
                }
            }

            let lut = &mut luts[ty * tiles + tx];
            let total = count.max(1.0);
            let mut cumulative = 0u32;
            for (i, &b) in hist.iter().enumerate() {
                cumulative += b;
                lut[i] = ((cumulative as f32 / total) * 255.0).clamp(0.0, 255.0) as u8;
            }
        }
    }

    // Bilinear blend between the four nearest tile centres.
    for y in 0..h {
        let gy = ((y as f32 - tile_h as f32 * 0.5) / tile_h as f32).max(0.0);
        let ty0 = (gy as usize).min(tiles - 1);
        let ty1 = (ty0 + 1).min(tiles - 1);
        let fy = gy - ty0 as f32;

        for x in 0..w {
            let gx = ((x as f32 - tile_w as f32 * 0.5) / tile_w as f32).max(0.0);
            let tx0 = (gx as usize).min(tiles - 1);
            let tx1 = (tx0 + 1).min(tiles - 1);
            let fx = gx - tx0 as f32;

            let v = (frame.data[y * w + x] >> 8) as usize;
            let tl = luts[ty0 * tiles + tx0][v] as f32;
            let tr = luts[ty0 * tiles + tx1][v] as f32;
            let bl = luts[ty1 * tiles + tx0][v] as f32;
            let br = luts[ty1 * tiles + tx1][v] as f32;

            let top = tl + (tr - tl) * fx;
            let bottom = bl + (br - bl) * fx;
            let out = (top + (bottom - top) * fy).clamp(0.0, 255.0) as u16;
            frame.data[y * w + x] = out * 257;
        }
    }
}

/// Equalise a frame for detection and encoding, using the project defaults.
pub fn histogram_equalize(frame: &mut IrFrame) {
    clahe_equalize(frame, CLAHE_CLIP_LIMIT, CLAHE_TILES);
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
    fn clahe_expands_a_low_contrast_frame() {
        // A dark, narrow-range frame — what an unlit-ish IR capture looks like.
        let data: Vec<u16> = (0..64 * 64).map(|i| ((i % 20) as u16 + 20) * 257).collect();
        let mut f = frame(data, 64, 64);
        let before = spread(&f);
        histogram_equalize(&mut f);
        assert!(spread(&f) > before, "CLAHE should widen the tonal range");
        assert!(f.data.iter().all(|&v| v % 257 == 0), "output stays 8-bit widened");
    }

    #[test]
    fn clahe_leaves_a_flat_frame_flat() {
        // Uniform input has no contrast to recover; it must not explode into
        // noise, which is precisely what the clip limit is for.
        let mut f = frame(vec![128 * 257; 64 * 64], 64, 64);
        histogram_equalize(&mut f);
        let first = f.data[0];
        assert!(f.data.iter().all(|&v| v == first));
    }

    #[test]
    fn clahe_handles_degenerate_frames() {
        let mut empty = frame(vec![], 0, 0);
        histogram_equalize(&mut empty);
        assert!(empty.data.is_empty());

        // Short buffer must be left alone rather than indexed out of bounds.
        let mut short = frame(vec![100; 10], 32, 32);
        histogram_equalize(&mut short);
        assert_eq!(short.data.len(), 10);
    }

    fn spread(f: &IrFrame) -> u16 {
        let max = f.data.iter().copied().max().unwrap_or(0);
        let min = f.data.iter().copied().min().unwrap_or(0);
        max - min
    }
}
