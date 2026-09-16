use tract_onnx::prelude::*;

pub struct FaceEncoder {
    model: TypedRunnableModel<TypedModel>,
}

impl FaceEncoder {
    pub fn new(model_path: &str) -> anyhow::Result<Self> {
        // Check first: tract's own error for a missing file is opaque, and
        // this is the most common misconfiguration.
        if !std::path::Path::new(model_path).exists() {
            anyhow::bail!("face encoder model not found at {model_path}");
        }
        // into_optimized() runs tract's fusion and lowering passes. Without it
        // the graph executes op-by-op as written: this model took ~400ms per
        // frame unoptimized, which dominated unlock latency.
        let model = onnx()
            .model_for_path(model_path)?
            .with_input_fact(
                0,
                InferenceFact::dt_shape(f32::datum_type(), tvec!(1, 3, 112, 112)),
            )?
            .into_optimized()?
            .into_runnable()?;
        Ok(Self { model })
    }

    /// Run the encoder and return an L2-normalised embedding.
    pub fn encode(&mut self, input: tract_ndarray::ArrayView3<f32>) -> anyhow::Result<Vec<f32>> {
        let mut input = input.to_owned().into_dyn();
        input.insert_axis_inplace(tract_ndarray::Axis(0));
        let input_value = Tensor::from(input).into_tvalue();
        let result = self.model.run(tvec!(input_value))?;

        let output = result[0].to_array_view::<f32>()?;
        let embedding: Vec<f32> = output.iter().copied().collect();

        anyhow::ensure!(!embedding.is_empty(), "encoder returned an empty embedding");

        // A zero or non-finite norm would divide into NaN/inf and produce an
        // embedding that fails every comparison for reasons nothing reports.
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        anyhow::ensure!(
            norm.is_finite() && norm > 0.0,
            "encoder produced a degenerate embedding (norm {norm})"
        );

        Ok(embedding.into_iter().map(|x| x / norm).collect())
    }
}
