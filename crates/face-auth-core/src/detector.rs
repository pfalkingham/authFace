use image::DynamicImage;
use tract_ndarray::s;
use tract_onnx::prelude::*;

use crate::capture::IrFrame;

// Frame-quality gates, expressed in 8-bit-equivalent units so the numbers can
// be compared against what `cargo run --example frame-stats` prints.
//
// Measured on the reference ASUS IR sensor:
//   illuminated frames   mean 48-96,  variance  82-317
//   dark (strobe off)    mean 1.8-8,  variance 3.4-39
//
// The old gate was a variance floor of 100_000 in the u16 domain, which is a
// variance of 1.5 in these units — low enough that the dark frames passed it
// and were then histogram-equalised into a grey noise field.
const MIN_FRAME_MEAN_8BIT: f64 = 12.0;
const MIN_FRAME_VARIANCE_8BIT: f64 = 20.0;

/// Samples are u16 but carry 8-bit data widened by 257 (see `capture_frame`).
const U16_PER_8BIT: f64 = 257.0;

/// Why a frame was rejected before inference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameQuality {
    Ok,
    Empty,
    /// Almost certainly an unlit frame from a strobing IR illuminator.
    TooDark { mean_8bit: f64 },
    /// Uniform field — a covered lens, or a wall.
    TooFlat { variance_8bit: f64 },
}

impl std::fmt::Display for FrameQuality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameQuality::Ok => write!(f, "ok"),
            FrameQuality::Empty => write!(f, "empty frame"),
            FrameQuality::TooDark { mean_8bit } => write!(
                f,
                "too dark (mean {mean_8bit:.1}/255, need {MIN_FRAME_MEAN_8BIT}) \
                 — unlit frame, or the IR illuminator is not firing"
            ),
            FrameQuality::TooFlat { variance_8bit } => write!(
                f,
                "too flat (variance {variance_8bit:.1}, need {MIN_FRAME_VARIANCE_8BIT}) \
                 — lens covered, or nothing in view"
            ),
        }
    }
}

const DETECTOR_WIDTH: usize = 320;
const DETECTOR_HEIGHT: usize = 240;

pub struct FaceDetector {
    model: InferenceSimplePlan<InferenceModel>,
    threshold: f32,
}

impl FaceDetector {
    pub fn new(model_path: &str, threshold: f32) -> anyhow::Result<Self> {
        if !std::path::Path::new(model_path).exists() {
            anyhow::bail!("face detector model not found at {model_path}");
        }
        let model = onnx().model_for_path(model_path)?.into_runnable()?;
        Ok(Self { model, threshold })
    }

    pub fn detect(&mut self, frame: &IrFrame) -> anyhow::Result<bool> {
        let input = preprocess_for_detector(frame)?;
        let mut input = input.into_dyn();
        input.insert_axis_inplace(tract_ndarray::Axis(0));
        let input_tensor = Tensor::from(input).into_tvalue();
        let result = self.model.run(tvec!(input_tensor))?;

        let scores = result[0].to_array_view::<f32>()?;

        // Expected shape is [1, anchors, 2] with the face probability in
        // column 1. Check before slicing: `s![0, .., 1]` panics on anything
        // else, and this runs inside PAM.
        let shape = scores.shape();
        anyhow::ensure!(
            shape.len() == 3 && shape[0] == 1 && shape[2] >= 2,
            "unexpected detector output shape {:?}, expected [1, N, 2]",
            shape
        );

        let face_scores = scores.slice(s![0, .., 1]);
        let max_face = face_scores
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .fold(f32::NEG_INFINITY, f32::max);

        Ok(max_face >= self.threshold)
    }
}

/// Cheap "is this frame worth running inference on" check.
///
/// Accumulates in f64: a 640x400 frame sums ~256k terms of up to 4.3e9, well
/// past what f32's ~7 significant digits can carry.
pub fn assess_frame(frame: &IrFrame) -> FrameQuality {
    if frame.data.is_empty() {
        return FrameQuality::Empty;
    }
    let len = frame.data.len() as f64;
    let mean = frame.data.iter().map(|&v| v as f64).sum::<f64>() / len;
    let variance: f64 = frame
        .data
        .iter()
        .map(|&v| {
            let d = v as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / len;

    let mean_8bit = mean / U16_PER_8BIT;
    let variance_8bit = variance / (U16_PER_8BIT * U16_PER_8BIT);

    if mean_8bit < MIN_FRAME_MEAN_8BIT {
        return FrameQuality::TooDark { mean_8bit };
    }
    if variance_8bit < MIN_FRAME_VARIANCE_8BIT {
        return FrameQuality::TooFlat { variance_8bit };
    }
    FrameQuality::Ok
}

pub fn raw_frame_has_content(frame: &IrFrame) -> bool {
    assess_frame(frame) == FrameQuality::Ok
}

fn preprocess_for_detector(frame: &IrFrame) -> anyhow::Result<tract_ndarray::Array3<f32>> {
    let img_buffer = crate::preprocess::frame_to_luma16(frame)?;

    let dynamic_img = DynamicImage::ImageLuma16(img_buffer);
    let resized = dynamic_img.resize_exact(
        DETECTOR_WIDTH as u32,
        DETECTOR_HEIGHT as u32,
        image::imageops::FilterType::Lanczos3,
    );
    let rgb = resized.to_rgb8();

    let mut array =
        tract_ndarray::Array3::<f32>::zeros((3, DETECTOR_HEIGHT, DETECTOR_WIDTH));

    for y in 0..DETECTOR_HEIGHT {
        for x in 0..DETECTOR_WIDTH {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            array[[0, y, x]] = pixel.0[0] as f32 / 255.0;
            array[[1, y, x]] = pixel.0[1] as f32 / 255.0;
            array[[2, y, x]] = pixel.0[2] as f32 / 255.0;
        }
    }

    Ok(array)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a frame from 8-bit values, widened the way `capture_frame` does.
    fn frame8(values: Vec<u8>, width: u32, height: u32) -> IrFrame {
        IrFrame {
            data: values.into_iter().map(|v| v as u16 * 257).collect(),
            width,
            height,
        }
    }

    #[test]
    fn empty_frame_is_rejected() {
        assert_eq!(assess_frame(&frame8(vec![], 0, 0)), FrameQuality::Empty);
    }

    #[test]
    fn covered_lens_is_too_flat() {
        // Uniform mid-grey: bright enough, but no structure at all.
        let q = assess_frame(&frame8(vec![128; 1024], 32, 32));
        assert!(matches!(q, FrameQuality::TooFlat { .. }), "got {q:?}");
    }

    #[test]
    fn unlit_strobe_frame_is_too_dark() {
        // Modelled on a real unlit frame from the reference sensor, which
        // measured mean 8.0 and variance 38 in 8-bit units. Values 0..=16 give
        // mean 8.0, variance 24 — dark, but with *more* than enough variance to
        // clear the old variance-only gate, which is exactly why those frames
        // reached histogram equalisation and became a grey noise field.
        let data: Vec<u8> = (0..1024).map(|i| (i % 17) as u8).collect();
        let q = assess_frame(&frame8(data, 32, 32));
        assert!(matches!(q, FrameQuality::TooDark { .. }), "got {q:?}");

        // The old gate: variance > 100_000 in the u16 domain. Confirm this
        // frame would have sailed through it, so the test documents the bug.
        let frame = frame8((0..1024).map(|i| (i % 17) as u8).collect(), 32, 32);
        let len = frame.data.len() as f64;
        let mean = frame.data.iter().map(|&v| v as f64).sum::<f64>() / len;
        let var_u16 =
            frame.data.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / len;
        assert!(var_u16 > 100_000.0, "u16 variance was {var_u16}");
    }

    #[test]
    fn illuminated_frame_is_accepted() {
        // Mean ~64, plenty of structure — like a real lit frame.
        let data: Vec<u8> = (0..1024).map(|i| (i % 128) as u8).collect();
        assert_eq!(assess_frame(&frame8(data, 32, 32)), FrameQuality::Ok);
        assert!(raw_frame_has_content(&frame8(
            (0..1024).map(|i| (i % 128) as u8).collect(),
            32,
            32
        )));
    }

    #[test]
    fn brightness_alone_does_not_pass() {
        // A bright but featureless frame must still be rejected.
        let q = assess_frame(&frame8(vec![200; 1024], 32, 32));
        assert!(matches!(q, FrameQuality::TooFlat { .. }), "got {q:?}");
    }
}
