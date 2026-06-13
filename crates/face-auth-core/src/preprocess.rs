use image::{DynamicImage, ImageBuffer, Luma};
use tract_onnx::prelude::tract_ndarray::Array3;

pub fn preprocess_ir_frame(frame: &super::capture::IrFrame) -> anyhow::Result<Array3<f32>> {
    let width = frame.width as u32;
    let height = frame.height as u32;
    
    let img_buffer: ImageBuffer<Luma<u16>, Vec<u16>> = ImageBuffer::from_fn(width, height, |x, y| {
        let idx = (y * width + x) as usize;
        let val = frame.data.get(idx).copied().unwrap_or(0);
        Luma([val])
    });
    
    let dynamic_img = DynamicImage::ImageLuma16(img_buffer);
    let resized = dynamic_img.resize_exact(112, 112, image::imageops::FilterType::Lanczos3);
    let gray_img = resized.to_luma16();
    
    let mut array = Array3::<f32>::zeros((3, 112, 112));
    
    for y in 0..112usize {
        for x in 0..112usize {
            let pixel = gray_img.get_pixel(x as u32, y as u32).0[0] as f32 / 65535.0;
            let normalized = (pixel - 0.5) / 0.5;
            for c in 0..3usize {
                array[[c, y, x]] = normalized;
            }
        }
    }
    
    Ok(array)
}

pub fn histogram_equalize(frame: &mut super::capture::IrFrame) {
    let mut hist = [0u32; 65536];
    
    for &val in &frame.data {
        hist[val as usize] += 1;
    }
    
    let total = frame.data.len() as f32;
    let mut cdf = [0f32; 65536];
    let mut sum = 0f32;
    
    for i in 0..65536 {
        sum += hist[i] as f32;
        cdf[i] = sum / total;
    }
    
    for val in &mut frame.data {
        *val = (cdf[*val as usize] * 65535.0) as u16;
    }
}