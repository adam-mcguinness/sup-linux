#[macro_use]
extern crate pamsm;

use pamsm::{PamServiceModule, Pam, PamFlags, PamError};
use sup_linux::protocol::{Request, Response, AuthRequest, SOCKET_PATH};
use rand::{Rng, thread_rng};
use std::time::{SystemTime, Duration};
use anyhow::Result;
use std::os::unix::net::UnixStream;
use std::io::{Read, Write};

const CHALLENGE_SIZE: usize = 32;

// Protocol types imported from sup_linux::protocol

pub struct SupLinuxPam;

impl PamServiceModule for SupLinuxPam {
    fn authenticate(pamh: Pam, _flags: PamFlags, args: Vec<String>) -> PamError {
        // eprintln!("SupLinux: PAM module authenticate() called");
        
        // Parse timeout from args (format: "timeout=10")
        // This is how long the PAM module waits for the service to respond
        // Should be longer than the service's authentication timeout to allow completion
        let pam_timeout_secs = args.iter()
            .find_map(|arg| {
                if arg.starts_with("timeout=") {
                    arg.strip_prefix("timeout=")?.parse::<u64>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(10); // Default: wait 10s for service response
        
        // Get username from PAM handle
        use pamsm::PamLibExt;
        
        let username = match pamh.get_cached_user() {
            Ok(Some(user_cstr)) => {
                match user_cstr.to_str() {
                    Ok(user) => {
                        // eprintln!("SupLinux: Authenticating user: {}", user);
                        user.to_string()
                    }
                    Err(_) => {
                        // eprintln!("SupLinux: Invalid UTF-8 in username");
                        return PamError::USER_UNKNOWN;
                    }
                }
            }
            Ok(None) => {
                // eprintln!("SupLinux: No username set in PAM");
                return PamError::USER_UNKNOWN;
            }
            Err(e) => {
                // eprintln!("SupLinux: Failed to get username: {:?}", e);
                return PamError::USER_UNKNOWN;
            }
        };

        // Perform authentication with PAM timeout
        match perform_authentication(&username, pam_timeout_secs) {
            Ok(true) => {
                // eprintln!("SupLinux: Face authentication successful for {}", username);
                PamError::SUCCESS
            }
            Ok(false) => {
                // eprintln!("SupLinux: Face authentication failed for {}", username);
                PamError::AUTH_ERR
            }
            Err(e) => {
                // eprintln!("SupLinux: Authentication error: {}", e);
                PamError::SERVICE_ERR
            }
        }
    }
}

fn perform_authentication(username: &str, pam_timeout_secs: u64) -> Result<bool> {
    // Generate random challenge for security
    let challenge = generate_challenge();
    
    // Connect to embedding service
    let mut stream = match UnixStream::connect(SOCKET_PATH) {
        Ok(s) => s,
        Err(e) => {
            // eprintln!("Failed to connect to embedding service: {}", e);
            return Ok(false);
        }
    };
    
    // Set socket timeout - how long PAM waits for service response
    // The service has its own timeout for the actual authentication process
    stream.set_read_timeout(Some(Duration::from_secs(pam_timeout_secs)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    
    // Create authentication request
    let request = Request::Authenticate(AuthRequest {
        username: username.to_string(),
        challenge: challenge.clone(),
        timestamp: SystemTime::now(),
    });
    
    // Send request
    let request_data = bincode::serialize(&request)?;
    let request_len = (request_data.len() as u32).to_le_bytes();
    stream.write_all(&request_len)?;
    stream.write_all(&request_data)?;
    stream.flush()?;
    
    // Read response
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let response_len = u32::from_le_bytes(len_buf) as usize;
    
    if response_len > 1024 * 1024 {
        anyhow::bail!("Response too large");
    }
    
    let mut response_buf = vec![0u8; response_len];
    stream.read_exact(&mut response_buf)?;
    
    let response: Response = bincode::deserialize(&response_buf)?;
    
    // Extract authentication result
    match response {
        Response::Auth(auth) => {
            // Verify signature for security (optional but recommended)
            if !auth.signature.is_empty() {
                // In production, we might want to verify the signature
                // For now, we trust the service since it's on the same machine
            }
            // eprintln!("SupLinux: Authentication {} - {}", 
            //     if auth.success { "succeeded" } else { "failed" },
            //     auth.message);
            Ok(auth.success)
        }
        Response::Error(msg) => {
            // eprintln!("SupLinux: Service error: {}", msg);
            Ok(false)
        }
        _ => {
            // eprintln!("SupLinux: Unexpected response type");
            Ok(false)
        }
    }
}

fn generate_challenge() -> Vec<u8> {
    let mut rng = thread_rng();
    let mut challenge = vec![0u8; CHALLENGE_SIZE];
    rng.fill(&mut challenge[..]);
    challenge
}

// All embedding comparison logic moved to service
// Service now handles K-of-N, embedding fusion, and lost face detection

// Register the PAM module
pam_module!(SupLinuxPam);