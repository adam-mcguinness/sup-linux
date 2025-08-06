use crate::error::{FaceAuthError, Result};
use crate::config::Config;
use crate::detector::FaceBox;
use ort::{Environment, Session, SessionBuilder, Value, GraphOptimizationLevel};
use std::sync::Arc;
use image::{DynamicImage, imageops::FilterType};
use ndarray::{Array4, CowArray};

pub type Embedding = Vec<f32>;

pub struct FaceRecognizer {
    session: Session,
    _environment: Arc<Environment>,
    config: Config,
}

impl FaceRecognizer {
    pub fn new_with_model_path(config: &Config, models_base: &std::path::Path) -> Result<Self> {
        let mut model_path = config.models.recognizer_path.clone();
        if model_path.is_relative() {
            model_path = models_base.join(&model_path);
        }
        
        let environment = Arc::new(
            Environment::builder()
                .with_name("face_recognizer")
                .build()
                .map_err(|e| FaceAuthError::Model(format!("Failed to create environment: {}", e)))?
        );
        
        if !model_path.exists() {
            return Err(FaceAuthError::Model(
                format!("Recognition model not found at: {:?}", model_path)
            ));
        }
        
        let mut session_builder = SessionBuilder::new(&environment)?;
        
        // Apply optimization level from config
        let opt_level = match config.performance.optimization_level {
            0 => GraphOptimizationLevel::Disable,
            1 => GraphOptimizationLevel::Level1,
            2 => GraphOptimizationLevel::Level2,
            _ => GraphOptimizationLevel::Level3,
        };
        session_builder = session_builder.with_optimization_level(opt_level)?;
        
        let session = session_builder.with_model_from_file(model_path)?;
        
        Ok(Self {
            session,
            _environment: environment,
            config: config.clone(),
        })
    }
    
    pub fn new(config: &Config) -> Result<Self> {
        let environment = Arc::new(
            Environment::builder()
                .with_name("face_recognizer")
                .build()
                .map_err(|e| FaceAuthError::Model(format!("Failed to create environment: {}", e)))?
        );

        let model_path = &config.models.recognizer_path;
        if !model_path.exists() {
            return Err(FaceAuthError::Model(
                format!("Recognition model not found at: {:?}", model_path)
            ));
        }

        let mut session_builder = SessionBuilder::new(&environment)?;
        
        // Apply optimization level from config
        let opt_level = match config.performance.optimization_level {
            0 => GraphOptimizationLevel::Disable,
            1 => GraphOptimizationLevel::Level1,
            2 => GraphOptimizationLevel::Level2,
            _ => GraphOptimizationLevel::Level3,
        };
        session_builder = session_builder.with_optimization_level(opt_level)?;
        
        let session = session_builder.with_model_from_file(model_path)?;

        Ok(Self { 
            session, 
            _environment: environment,
            config: config.clone(),
        })
    }

    pub fn get_embedding(&self, image: &DynamicImage, face: &FaceBox) -> Result<Embedding> {
        // Crop face from original image (coordinates are already in original image space)
        let face_img = self.crop_face(image, face)?;

        // Resize to configured size for embedding model
        let resized = face_img.resize_exact(
            self.config.recognizer.input_size, 
            self.config.recognizer.input_size, 
            FilterType::Triangle
        );

        // Convert to array with proper preprocessing for single-channel model
        let input_array = self.preprocess_face(&resized)?;
        let cow_array = CowArray::from(input_array.into_dyn());
        let input_tensor = Value::from_array(self.session.allocator(), &cow_array)?;

        // Run inference
        let outputs = self.session.run(vec![input_tensor])?;

        // Extract embedding
        let embedding = outputs[0].try_extract::<f32>()?.view().to_owned().into_raw_vec();
        Ok(embedding)
    }

    fn crop_face(&self, image: &DynamicImage, face: &FaceBox) -> Result<DynamicImage> {
        let x = face.x1.max(0.0) as u32;
        let y = face.y1.max(0.0) as u32;
        let width = (face.x2 - face.x1).max(1.0) as u32;
        let height = (face.y2 - face.y1).max(1.0) as u32;

        Ok(image.crop_imm(x, y, width, height))
    }

    fn preprocess_face(&self, img: &DynamicImage) -> Result<Array4<f32>> {
        // Convert to grayscale for single-channel embedding model
        let gray = img.to_luma8();
        let size = self.config.recognizer.input_size as usize;
        // Single channel output for embedding model
        let mut array = Array4::<f32>::zeros((1, 1, size, size));

        for y in 0..size {
            for x in 0..size {
                let pixel = gray.get_pixel(x as u32, y as u32);
                // ArcFace normalization
                let norm_val = self.config.recognizer.normalization_value;
                array[[0, 0, y, x]] = (pixel[0] as f32 - norm_val) / norm_val;
            }
        }

        Ok(array)
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}