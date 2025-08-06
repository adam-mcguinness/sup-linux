pub mod ascii_preview;
pub mod auth;
pub mod camera;
pub mod config;
pub mod detector;
pub mod dev_mode;
pub mod error;
pub mod paths;
pub mod quality;
pub mod recognizer;
pub mod storage;
pub mod visualization;

// Re-export commonly used types
pub use detector::FaceBox;
pub use error::{FaceAuthError, Result};