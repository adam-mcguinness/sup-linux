// Core modules
pub mod core;
pub mod camera;
pub mod storage;
pub mod service;
pub mod cli;
pub mod common;

// Re-export commonly used types
pub use common::{Config, DevMode, FaceAuthError, Result};
pub use core::{FaceDetector, FaceBox, FaceRecognizer, Embedding, cosine_similarity, QualityMetrics};
pub use camera::Camera;
pub use storage::{UserStore, UserData};
pub use service::{ServiceClient, protocol};

// Legacy compatibility exports (to avoid breaking existing code)
pub mod auth {
    pub use crate::core::auth::*;
}
pub mod detector {
    pub use crate::core::detector::*;
}
pub mod recognizer {
    pub use crate::core::recognizer::*;
}
pub mod quality {
    pub use crate::core::quality::*;
}
pub mod config {
    pub use crate::common::config::*;
}
pub mod dev_mode {
    pub use crate::common::dev_mode::*;
}
pub mod error {
    pub use crate::common::error::*;
}
pub mod paths {
    pub use crate::common::paths::*;
}
pub mod service_client {
    pub use crate::service::client::*;
}
pub mod visualization {
    pub use crate::cli::visualization::*;
}
pub mod ascii_preview {
    pub use crate::cli::ascii_preview::*;
}