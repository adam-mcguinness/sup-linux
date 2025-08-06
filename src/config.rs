use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::error::{FaceAuthError, Result};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub camera: CameraConfig,
    pub models: ModelConfig,
    pub auth: AuthConfig,
    pub detector: DetectorConfig,
    pub recognizer: RecognizerConfig,
    pub storage: StorageConfig,
    #[serde(default)]
    pub performance: PerformanceConfig,
    #[serde(default)]
    pub enrollment: EnrollmentConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CameraConfig {
    pub device_index: u32,
    pub width: u32,
    pub height: u32,
    pub warmup_frames: u32,
    #[serde(default = "default_warmup_delay")]
    pub warmup_delay_ms: u64,
}

fn default_warmup_delay() -> u64 {
    50
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelConfig {
    pub detector_path: PathBuf,
    pub recognizer_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthConfig {
    pub similarity_threshold: f32,
    pub timeout_seconds: u32,
    pub detection_confidence: f32,
    #[serde(default = "default_k_required")]
    pub k_required_matches: u32,
    #[serde(default = "default_n_attempts")]
    pub n_total_attempts: u32,
    #[serde(default = "default_buffer_size")]
    pub embedding_buffer_size: u32,
    #[serde(default = "default_true")]
    pub use_embedding_fusion: bool,
    #[serde(default = "default_lost_face_timeout")]
    pub lost_face_timeout: u32,
}

fn default_k_required() -> u32 { 2 }
fn default_n_attempts() -> u32 { 3 }
fn default_buffer_size() -> u32 { 3 }
fn default_true() -> bool { true }
fn default_lost_face_timeout() -> u32 { 3 }

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DetectorConfig {
    pub input_width: u32,
    pub input_height: u32,
    pub normalization_mean: f32,
    pub normalization_std: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecognizerConfig {
    pub input_size: u32,
    pub normalization_value: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StorageConfig {
    pub enrollment_images_dir: PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PerformanceConfig {
    #[serde(default = "default_true")]
    pub enable_quantization: bool,
    #[serde(default = "default_optimization_level")]
    pub optimization_level: u32,
}

fn default_optimization_level() -> u32 { 3 }

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EnrollmentConfig {
    #[serde(default = "default_true")]
    pub store_averaged_embedding: bool,
    #[serde(default = "default_true")]
    pub capture_quality_metrics: bool,
    #[serde(default = "default_enrollment_quality")]
    pub min_enrollment_quality: f32,
    #[serde(default = "default_num_captures")]
    pub num_captures: Option<usize>,
    #[serde(default = "default_capture_interval")]
    pub capture_interval_ms: Option<u64>,
    #[serde(default = "default_true_option")]
    pub enable_ascii_preview: Option<bool>,
    #[serde(default)]
    pub ascii_width: Option<usize>,
    #[serde(default)]
    pub ascii_height: Option<usize>,
}

fn default_enrollment_quality() -> f32 { 0.7 }
fn default_num_captures() -> Option<usize> { Some(5) }
fn default_capture_interval() -> Option<u64> { Some(2000) }
fn default_true_option() -> Option<bool> { Some(true) }


impl Config {
    pub fn load() -> Result<Self> {
        let config_path = "configs/face-auth.toml";
        Self::load_from_path(&std::path::PathBuf::from(config_path))
    }
    
    pub fn load_from_path(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Config file not found: {}. Please create it from the example.", path.display()
            )));
        }
        
        println!("Loading config from: {}", path.display());
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)
            .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Config parse error: {}", e)))?;
        
        config.validate()?;
        Ok(config)
    }
    
    pub fn validate(&self) -> Result<()> {
        // Validate camera dimensions
        if self.camera.width == 0 || self.camera.width > 4096 {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Camera width must be between 1 and 4096, got {}", self.camera.width
            )));
        }
        if self.camera.height == 0 || self.camera.height > 4096 {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Camera height must be between 1 and 4096, got {}", self.camera.height
            )));
        }
        
        // Validate thresholds
        if self.auth.similarity_threshold < 0.0 || self.auth.similarity_threshold > 1.0 {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Similarity threshold must be between 0.0 and 1.0, got {}", 
                self.auth.similarity_threshold
            )));
        }
        if self.auth.detection_confidence < 0.0 || self.auth.detection_confidence > 1.0 {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Detection confidence must be between 0.0 and 1.0, got {}", 
                self.auth.detection_confidence
            )));
        }
        
        // Validate timeout
        if self.auth.timeout_seconds < 1 || self.auth.timeout_seconds > 60 {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Timeout must be between 1 and 60 seconds, got {}", 
                self.auth.timeout_seconds
            )));
        }
        
        // Validate detector dimensions
        if self.detector.input_width == 0 || self.detector.input_width > 4096 {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Detector input width must be between 1 and 4096, got {}", 
                self.detector.input_width
            )));
        }
        if self.detector.input_height == 0 || self.detector.input_height > 4096 {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Detector input height must be between 1 and 4096, got {}", 
                self.detector.input_height
            )));
        }
        
        // Validate recognizer input size
        if self.recognizer.input_size == 0 || self.recognizer.input_size > 1024 {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Recognizer input size must be between 1 and 1024, got {}", 
                self.recognizer.input_size
            )));
        }
        
        Ok(())
    }
}