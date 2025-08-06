use linuxSup::{auth, paths};
use std::env;

fn main() {
    // PAM-compatible authentication binary
    
    // Get username from PAM_USER environment variable (set by pam_exec)
    let username = match env::var("PAM_USER") {
        Ok(user) => user,
        Err(_) => {
            // Fallback to command line argument for testing
            let args: Vec<String> = env::args().collect();
            if args.len() == 2 {
                args[1].clone()
            } else {
                eprintln!("Error: No PAM_USER environment variable or username argument");
                std::process::exit(1);
            }
        }
    };
    
    // Check if we're in a remote session (SSH) or no display
    let is_remote = env::var("SSH_CLIENT").is_ok() || env::var("SSH_TTY").is_ok();
    let has_display = env::var("DISPLAY").is_ok() || env::var("WAYLAND_DISPLAY").is_ok();
    
    if is_remote || !has_display {
        // Skip face auth for remote sessions or when no display
        eprintln!("Face authentication not available (remote session or no display)");
        std::process::exit(1);
    }
    
    // Log to syslog using simple logging
    eprintln!("LinuxSup: Face authentication attempt for user: {}", username);
    
    // Always use system mode for PAM authentication
    let paths = match paths::Paths::new(false, true) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to initialize paths: {}", e);
            std::process::exit(1);
        }
    };
    
    // Perform authentication with timeout
    // Note: In production, we might want to handle this differently
    match auth::authenticate_user_system(&username, &paths) {
        Ok(true) => {
            // Success
            eprintln!("LinuxSup: Face authentication successful for user: {}", username);
            std::process::exit(0);
        }
        Ok(false) => {
            // Authentication failed
            eprintln!("Face authentication failed");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Authentication error: {}", e);
            std::process::exit(1);
        }
    }
}