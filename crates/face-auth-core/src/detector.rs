use image::DynamicImage;
use tract_ndarray::s;
use tract_onnx::prelude::*;

use crate::capture::IrFrame;

/// Minimum pixel variance for a frame to be worth running the detector on.
/// Filters out a covered lens or a dark room cheaply, before inference.
const MIN_FRAME_VARIANCE: f64 = 100_000.0;

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

/// Cheap "is anything actually there" check on the raw frame.
///
/// Accumulates in f64: a 640x400 frame sums ~256k terms of up to 4.3e9, well
/// past what f32's ~7 significant digits can carry.
pub fn raw_frame_has_content(frame: &IrFrame) -> bool {
    if frame.data.is_empty() {
        return false;
    }
    let len = frame.data.len() as f64;
    let sum: f64 = frame.data.iter().map(|&v| v as f64).sum();
    let mean = sum / len;
    let variance: f64 = frame
        .data
        .iter()
        .map(|&v| {
            let d = v as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / len;
    variance > MIN_FRAME_VARIANCE
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

    fn frame(data: Vec<u16>, width: u32, height: u32) -> IrFrame {
        IrFrame { data, width, height }
    }

    #[test]
    fn empty_frame_has_no_content() {
        assert!(!raw_frame_has_content(&frame(vec![], 0, 0)));
    }

    #[test]
    fn flat_frame_has_no_content() {
        // A covered lens: every pixel identical, so zero variance.
        assert!(!raw_frame_has_content(&frame(vec![4096; 1024], 32, 32)));
    }

    #[test]
    fn high_contrast_frame_has_content() {
        let data: Vec<u16> = (0..1024)
            .map(|i| if i % 2 == 0 { 0 } else { 65535 })
            .collect();
        assert!(raw_frame_has_content(&frame(data, 32, 32)));
    }
}
