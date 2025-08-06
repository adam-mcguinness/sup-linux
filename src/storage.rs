use crate::error::{FaceAuthError, Result};
use crate::recognizer::Embedding;
use crate::dev_mode::DevMode;
use directories::ProjectDirs;
use std::path::PathBuf;
use std::fs;
use serde::{Serialize, Deserialize};

const STORAGE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct UserData {
    pub version: u32,
    pub username: String,
    pub embeddings: Vec<Embedding>,
    #[serde(default)]
    pub averaged_embedding: Option<Embedding>,
    #[serde(default)]
    pub embedding_qualities: Option<Vec<f32>>,
}

pub struct UserStore {
    data_dir: PathBuf,
    enrollment_images_dir: PathBuf,
}

impl UserStore {
    pub fn new_with_paths(data_dir: PathBuf, enrollment_images_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&enrollment_images_dir)?;
        
        Ok(Self { 
            data_dir,
            enrollment_images_dir,
        })
    }
    
    pub fn new() -> Result<Self> {
        let dirs = ProjectDirs::from("com", "faceauth", "FaceAuth")
            .ok_or_else(|| FaceAuthError::Storage("Failed to get project dirs".into()))?;

        let data_dir = dirs.data_dir().to_path_buf();
        let enrollment_images_dir = data_dir.join("enrollment_images");
        
        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&enrollment_images_dir)?;

        Ok(Self { 
            data_dir,
            enrollment_images_dir,
        })
    }
    
    pub fn new_with_dev_mode(dev_mode: &DevMode) -> Result<Self> {
        let (data_dir, enrollment_images_dir) = if dev_mode.is_enabled() {
            (
                dev_mode.data_dir(),
                dev_mode.enrollment_images_dir(),
            )
        } else {
            // Use system directories
            let dirs = ProjectDirs::from("com", "faceauth", "FaceAuth")
                .ok_or_else(|| FaceAuthError::Storage("Failed to get project dirs".into()))?;
            
            let data_dir = dirs.data_dir().to_path_buf();
            let enrollment_images_dir = data_dir.join("enrollment_images");
            
            (data_dir, enrollment_images_dir)
        };
        
        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&enrollment_images_dir)?;
        
        if dev_mode.is_enabled() {
            tracing::debug!("UserStore using dev directories: {:?}", data_dir);
        }
        
        Ok(Self { 
            data_dir,
            enrollment_images_dir,
        })
    }

    pub fn save_user_data(&self, user_data: &UserData) -> Result<()> {
        let user_file = self.data_dir.join(format!("{}.bincode", user_data.username));
        let encoded = bincode::serialize(user_data)
            .map_err(|e| FaceAuthError::Storage(format!("Failed to serialize: {}", e)))?;
        fs::write(user_file, encoded)?;
        Ok(())
    }

    pub fn get_user(&self, username: &str) -> Result<UserData> {
        let user_file = self.data_dir.join(format!("{}.bincode", username));

        if !user_file.exists() {
            return Err(FaceAuthError::UserNotFound(username.to_string()));
        }

        let data = fs::read(user_file)?;
        let mut user_data: UserData = bincode::deserialize(&data)
            .map_err(|e| FaceAuthError::Storage(format!("Failed to deserialize: {}", e)))?;

        // Handle version migration if needed
        if user_data.version < STORAGE_VERSION {
            // Future migration logic would go here
            user_data.version = STORAGE_VERSION;
        }

        Ok(user_data)
    }

    pub fn get_enrollment_images_dir(&self, username: &str) -> Result<PathBuf> {
        let user_dir = self.enrollment_images_dir.join(username);
        Ok(user_dir)
    }
}