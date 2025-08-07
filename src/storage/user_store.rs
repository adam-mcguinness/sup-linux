use crate::common::{FaceAuthError, Result, DevMode};
use crate::core::recognizer::Embedding;
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
    #[allow(dead_code)]
    pub fn new_with_paths(data_dir: PathBuf, enrollment_images_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&enrollment_images_dir)?;
        
        Ok(Self { 
            data_dir,
            enrollment_images_dir,
        })
    }
    
    #[allow(dead_code)]
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

    /// Merge new embeddings with existing user data
    pub fn merge_user_data(&self, existing: &mut UserData, new_embeddings: Vec<Embedding>, 
                          new_qualities: Vec<f32>, replace_weak: bool) -> (usize, usize) {
        let initial_count = existing.embeddings.len();
        let mut replaced_count = 0;
        
        if replace_weak && existing.embedding_qualities.is_some() {
            // Find indices of weakest embeddings
            let qualities = existing.embedding_qualities.as_ref().unwrap();
            let mut quality_indices: Vec<(usize, f32)> = qualities
                .iter()
                .enumerate()
                .map(|(i, &q)| (i, q))
                .collect();
            quality_indices.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            
            // Replace weak embeddings with new ones if new ones are better
            for (new_emb, new_qual) in new_embeddings.iter().zip(new_qualities.iter()) {
                let mut replaced = false;
                for &(idx, old_qual) in quality_indices.iter() {
                    if *new_qual > old_qual && replaced_count < new_embeddings.len() {
                        existing.embeddings[idx] = new_emb.clone();
                        if let Some(ref mut quals) = existing.embedding_qualities {
                            quals[idx] = *new_qual;
                        }
                        replaced = true;
                        replaced_count += 1;
                        break;
                    }
                }
                
                // If not replaced (all existing are better), append
                if !replaced {
                    existing.embeddings.push(new_emb.clone());
                    if let Some(ref mut quals) = existing.embedding_qualities {
                        quals.push(*new_qual);
                    }
                }
            }
        } else {
            // Just append new embeddings
            existing.embeddings.extend(new_embeddings);
            match existing.embedding_qualities.as_mut() {
                Some(quals) => quals.extend(new_qualities),
                None => existing.embedding_qualities = Some(new_qualities),
            }
        }
        
        // Recalculate averaged embedding
        existing.averaged_embedding = Some(Self::average_embeddings(&existing.embeddings));
        
        let final_count = existing.embeddings.len();
        (final_count - initial_count, replaced_count)
    }
    
    fn average_embeddings(embeddings: &[Embedding]) -> Embedding {
        if embeddings.is_empty() {
            return vec![];
        }
        
        let embedding_size = embeddings[0].len();
        let mut averaged = vec![0.0f32; embedding_size];
        
        for embedding in embeddings {
            for (i, &value) in embedding.iter().enumerate() {
                averaged[i] += value;
            }
        }
        
        let count = embeddings.len() as f32;
        for value in &mut averaged {
            *value /= count;
        }
        
        averaged
    }
}