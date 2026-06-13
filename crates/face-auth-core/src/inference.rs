use std::path::Path;
use crate::error::FaceAuthError;
use tract_onnx::prelude::*;

pub struct FaceEncoder {
    model: SimplePlan<TypedFact, Box<dyn TypedOp>, TypedModel>,
}

impl FaceEncoder {
    pub fn new(model_path: &str) -> anyhow::Result<Self> {
        let model = onnx()
            .model_for_path(model_path)?
            .with_input_fact(0, InferenceFact::dt_shape(f32::datum_type(), tvec!(1, 3, 112, 112)))?
            .into_optimized()?
            .into_runnable()?;
        Ok(Self { model })
    }
    
    pub fn encode(&mut self, input: tract_ndarray::ArrayView3<f32>) -> anyhow::Result<Vec<f32>> {
        let mut input = input.to_owned().into_dyn();
        input.insert_axis_inplace(tract_ndarray::Axis(0));
        let input_value = Tensor::from(input).into_tvalue();
        let result = self.model.run(tvec!(input_value))?;
        
        let output = result[0].to_array_view::<f32>()?;
        let embedding = output.iter().copied().collect::<Vec<f32>>();
        
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        Ok(embedding.iter().map(|x| x / norm).collect())
    }
}

pub fn load_model(model_path: &str) -> anyhow::Result<FaceEncoder> {
    if !Path::new(model_path).exists() {
        return Err(FaceAuthError::Inference(format!("Model not found: {}", model_path)).into());
    }
    FaceEncoder::new(model_path)
}