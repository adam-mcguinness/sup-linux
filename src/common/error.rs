use thiserror::Error;

#[derive(Error, Debug)]
pub enum FaceAuthError {
    #[error("Camera error: {0}")]
    Camera(String),

    #[error("Model error: {0}")]
    Model(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("User not found: {0}")]
    UserNotFound(String),

    #[error("No face detected")]
    NoFaceDetected,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("ORT error: {0}")]
    Ort(#[from] ort::OrtError),

    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, FaceAuthError>;