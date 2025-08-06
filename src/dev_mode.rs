use std::path::PathBuf;
use std::fs;
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct DevMode {
    enabled: bool,
    base_dir: PathBuf,
}

impl DevMode {
    pub fn new(enabled: bool) -> Result<Self> {
        let base_dir = if enabled {
            PathBuf::from("./dev_data")
        } else {
            PathBuf::new() // Not used when disabled
        };
        
        // Create dev directories if in dev mode
        if enabled {
            fs::create_dir_all(&base_dir)?;
            fs::create_dir_all(base_dir.join("users"))?;
            fs::create_dir_all(base_dir.join("enrollment"))?;
            fs::create_dir_all(base_dir.join("captures"))?;
            fs::create_dir_all(base_dir.join("logs"))?;
            fs::create_dir_all(base_dir.join("config"))?;
            fs::create_dir_all(base_dir.join("debug"))?;
            
            println!("📁 Development mode enabled - data will be saved to: {}", 
                     base_dir.display());
        }
        
        Ok(Self { enabled, base_dir })
    }
    
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    
    pub fn data_dir(&self) -> PathBuf {
        if self.enabled {
            self.base_dir.join("users")
        } else {
            // This should not be called when not in dev mode
            panic!("data_dir() called when dev mode is disabled")
        }
    }
    
    pub fn enrollment_images_dir(&self) -> PathBuf {
        if self.enabled {
            self.base_dir.join("enrollment")
        } else {
            panic!("enrollment_images_dir() called when dev mode is disabled")
        }
    }
    
    pub fn captures_dir(&self) -> PathBuf {
        if self.enabled {
            self.base_dir.join("captures")
        } else {
            panic!("captures_dir() called when dev mode is disabled")
        }
    }
    
    #[allow(dead_code)]
    pub fn logs_dir(&self) -> PathBuf {
        if self.enabled {
            self.base_dir.join("logs")
        } else {
            panic!("logs_dir() called when dev mode is disabled")
        }
    }
    
    #[allow(dead_code)]
    pub fn config_dir(&self) -> PathBuf {
        if self.enabled {
            self.base_dir.join("config")
        } else {
            panic!("config_dir() called when dev mode is disabled")
        }
    }
    
    pub fn debug_dir(&self) -> PathBuf {
        if self.enabled {
            self.base_dir.join("debug")
        } else {
            panic!("debug_dir() called when dev mode is disabled")
        }
    }
    
    pub fn get_capture_path(&self, prefix: &str) -> PathBuf {
        if self.enabled {
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            self.captures_dir().join(format!("{}_{}.jpg", prefix, timestamp))
        } else {
            // In production mode, use current directory
            PathBuf::from(format!("{}.jpg", prefix))
        }
    }
    
    pub fn get_debug_path(&self, prefix: &str) -> PathBuf {
        if self.enabled {
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            self.debug_dir().join(format!("{}_{}.jpg", prefix, timestamp))
        } else {
            PathBuf::from(format!("{}_debug.jpg", prefix))
        }
    }
}