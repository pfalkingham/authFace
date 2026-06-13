use thiserror::Error;

#[derive(Error, Debug)]
pub enum FaceAuthError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("Inference error: {0}")]
    Inference(String),

    #[error("Verification failed: {0}")]
    VerificationFailed(String),

    #[error("No IR camera found")]
    NoCamera,

    #[error("Camera busy")]
    CameraBusy,

    #[error("No embeddings found for user")]
    NoEmbeddings,

    #[error("Invalid embedding format")]
    InvalidEmbeddingFormat,
}

pub type Result<T> = std::result::Result<T, FaceAuthError>;