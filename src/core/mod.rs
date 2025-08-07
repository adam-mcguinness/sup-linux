pub mod auth;
pub mod detector;
pub mod recognizer;
pub mod quality;

pub use auth::*;
pub use detector::{FaceDetector, FaceBox};
pub use recognizer::{FaceRecognizer, cosine_similarity, Embedding};
pub use quality::{QualityMetrics, calculate_embedding_consistency};