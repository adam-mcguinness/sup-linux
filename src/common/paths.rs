use std::path::PathBuf;

// Simple paths module - we primarily use DevMode for path management
// This module is kept for potential future use but simplified

pub fn system_user_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/suplinux/users")
}

pub fn system_enrollment_dir() -> PathBuf {
    PathBuf::from("/var/lib/suplinux/enrollment")
}

pub fn system_config_file() -> PathBuf {
    PathBuf::from("/etc/suplinux/face-auth.toml")
}

pub fn system_models_dir() -> PathBuf {
    PathBuf::from("/usr/share/suplinux/models")
}