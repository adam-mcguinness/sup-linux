use std::path::PathBuf;
use std::fs;
use crate::error::{Result, FaceAuthError};

pub enum RunMode {
    Development(PathBuf),  // Base directory for dev mode
    System,                // Use system paths
    User,                  // Use user home directory paths
}

pub struct Paths {
    mode: RunMode,
}

impl Paths {
    pub fn new(dev: bool, system: bool) -> Result<Self> {
        match (dev, system) {
            (true, true) => {
                return Err(FaceAuthError::Other(anyhow::anyhow!(
                    "Cannot use both --dev and --system flags"
                )));
            }
            (true, false) => {
                let base_dir = PathBuf::from("./dev_data");
                
                // Create dev directories
                fs::create_dir_all(&base_dir)?;
                fs::create_dir_all(base_dir.join("users"))?;
                fs::create_dir_all(base_dir.join("enrollment"))?;
                fs::create_dir_all(base_dir.join("captures"))?;
                fs::create_dir_all(base_dir.join("debug"))?;
                
                println!("📁 Development mode - using local directory: {}", 
                         base_dir.display());
                
                Ok(Self {
                    mode: RunMode::Development(base_dir),
                })
            }
            (false, true) => {
                println!("🔧 System mode - using system paths");
                println!("⚠️  WARNING: This is a TEST VERSION - NOT SECURE");
                
                // Create system directories (may require root)
                let dirs = vec![
                    "/etc/linuxsup",
                    "/var/lib/linuxsup",
                    "/var/lib/linuxsup/users",
                    "/var/lib/linuxsup/enrollment",
                ];
                
                for dir in &dirs {
                    if let Err(e) = fs::create_dir_all(dir) {
                        if e.kind() == std::io::ErrorKind::PermissionDenied {
                            eprintln!("Permission denied creating {}. Run with sudo for initial setup.", dir);
                        }
                    }
                }
                
                Ok(Self {
                    mode: RunMode::System,
                })
            }
            (false, false) => {
                // Default to user mode if not root, dev mode otherwise
                if std::env::var("USER").unwrap_or_default() == "root" {
                    // Running as root without --system flag, use dev mode
                    Self::new(true, false)
                } else {
                    // Running as regular user, use user home directory
                    let home = dirs::home_dir()
                        .ok_or_else(|| FaceAuthError::Other(anyhow::anyhow!("Could not find home directory")))?;
                    let base_dir = home.join(".local/share/linuxsup");
                    
                    // Create user directories
                    fs::create_dir_all(&base_dir)?;
                    fs::create_dir_all(base_dir.join("users"))?;
                    fs::create_dir_all(base_dir.join("enrollment"))?;
                    
                    println!("👤 User mode - using home directory: {}", base_dir.display());
                    
                    Ok(Self {
                        mode: RunMode::User,
                    })
                }
            }
        }
    }
    
    pub fn config_file(&self) -> PathBuf {
        match &self.mode {
            RunMode::Development(base) => base.join("configs/face-auth.toml"),
            RunMode::System => PathBuf::from("/etc/linuxsup/face-auth.toml"),
            RunMode::User => {
                // Try user config first, then system config
                if let Ok(home) = dirs::home_dir().ok_or(()) {
                    let user_config = home.join(".config/linuxsup/face-auth.toml");
                    if user_config.exists() {
                        return user_config;
                    }
                }
                // Fall back to system config
                PathBuf::from("/etc/linuxsup/face-auth.toml")
            }
        }
    }
    
    pub fn user_data_dir(&self) -> PathBuf {
        match &self.mode {
            RunMode::Development(base) => base.join("users"),
            RunMode::System => PathBuf::from("/var/lib/linuxsup/users"),
            RunMode::User => {
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".local/share/linuxsup/users")
            }
        }
    }
    
    pub fn enrollment_dir(&self) -> PathBuf {
        match &self.mode {
            RunMode::Development(base) => base.join("enrollment"),
            RunMode::System => PathBuf::from("/var/lib/linuxsup/enrollment"),
            RunMode::User => {
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".local/share/linuxsup/enrollment")
            }
        }
    }
    
    pub fn models_dir(&self) -> PathBuf {
        match &self.mode {
            RunMode::Development(_) => PathBuf::from("./models"),
            RunMode::System | RunMode::User => PathBuf::from("/usr/share/linuxsup/models"),
        }
    }
    
    pub fn get_capture_path(&self, prefix: &str) -> PathBuf {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        match &self.mode {
            RunMode::Development(base) => {
                let captures_dir = base.join("captures");
                fs::create_dir_all(&captures_dir).ok();
                captures_dir.join(format!("{}_{}.jpg", prefix, timestamp))
            }
            RunMode::System | RunMode::User => {
                // In system/user mode, use temp directory
                PathBuf::from(format!("/tmp/linuxsup_{}_{}.jpg", prefix, timestamp))
            }
        }
    }
    
    pub fn get_debug_path(&self, prefix: &str) -> PathBuf {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        match &self.mode {
            RunMode::Development(base) => {
                let debug_dir = base.join("debug");
                fs::create_dir_all(&debug_dir).ok();
                debug_dir.join(format!("{}_{}.jpg", prefix, timestamp))
            }
            RunMode::System | RunMode::User => {
                // In system/user mode, use temp directory
                PathBuf::from(format!("/tmp/linuxsup_debug_{}_{}.jpg", prefix, timestamp))
            }
        }
    }
    
    pub fn is_development(&self) -> bool {
        matches!(self.mode, RunMode::Development(_))
    }
    
    pub fn is_system(&self) -> bool {
        matches!(self.mode, RunMode::System)
    }
}