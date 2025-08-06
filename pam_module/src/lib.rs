#[macro_use]
extern crate pamsm;

use pamsm::{PamServiceModule, Pam, PamFlags, PamError};
use rand::{Rng, thread_rng};
use sha2::{Sha256, Digest};
use std::time::{SystemTime, Duration};
use anyhow::Result;
use std::os::unix::net::UnixStream;
use std::io::{Read, Write};
use serde::{Serialize, Deserialize};

const SOCKET_PATH: &str = "/run/linuxsup/embedding.sock";
const CHALLENGE_SIZE: usize = 32;
const CHALLENGE_TIMEOUT: Duration = Duration::from_secs(5);
const SIMILARITY_THRESHOLD: f32 = 0.6;

#[derive(Serialize, Deserialize, Debug)]
struct AuthRequest {
    username: String,
    challenge: Vec<u8>,
    timestamp: SystemTime,
}

#[derive(Serialize, Deserialize, Debug)]
struct AuthResponse {
    embedding: Vec<f32>,
    signature: Vec<u8>,
    timestamp: SystemTime,
}

pub struct LinuxSupPam;

impl PamServiceModule for LinuxSupPam {
    fn authenticate(_pamh: Pam, _flags: PamFlags, _args: Vec<String>) -> PamError {
        // For now, let's use environment variable as a workaround
        // TODO: Find correct pamsm API for getting username
        let username = match std::env::var("PAM_USER") {
            Ok(user) => user,
            Err(_) => return PamError::USER_UNKNOWN,
        };

        // Check if we're in a remote session
        if std::env::var("SSH_CLIENT").is_ok() || std::env::var("SSH_TTY").is_ok() {
            eprintln!("LinuxSup: Skipping face auth for remote session");
            return PamError::AUTH_ERR;
        }

        // Check if we have a display
        if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
            eprintln!("LinuxSup: No display available for face auth");
            return PamError::AUTH_ERR;
        }

        // Perform authentication
        match perform_authentication(&username) {
            Ok(true) => {
                eprintln!("LinuxSup: Face authentication successful for {}", username);
                PamError::SUCCESS
            }
            Ok(false) => {
                eprintln!("LinuxSup: Face authentication failed for {}", username);
                PamError::AUTH_ERR
            }
            Err(e) => {
                eprintln!("LinuxSup: Authentication error: {}", e);
                PamError::SERVICE_ERR
            }
        }
    }
}

fn perform_authentication(username: &str) -> Result<bool> {
    // Generate random challenge
    let challenge = generate_challenge();
    let challenge_time = SystemTime::now();
    
    // Connect to embedding service
    let mut stream = match UnixStream::connect(SOCKET_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to embedding service: {}", e);
            return Ok(false);
        }
    };
    
    // Set timeouts
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    
    // Create request
    let request = AuthRequest {
        username: username.to_string(),
        challenge: challenge.clone(),
        timestamp: challenge_time,
    };
    
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
    
    let response: AuthResponse = bincode::deserialize(&response_buf)?;
    
    // Verify response
    if response.embedding.is_empty() {
        return Ok(false);
    }
    
    // Verify timestamp
    if response.timestamp < challenge_time || 
       response.timestamp.duration_since(challenge_time)? > CHALLENGE_TIMEOUT {
        return Ok(false);
    }
    
    // Verify signature
    if !verify_signature(&response.embedding, &challenge, &response.signature) {
        return Ok(false);
    }
    
    // Load stored embeddings
    let stored_embeddings = load_user_embeddings(username)?;
    if stored_embeddings.is_empty() {
        return Ok(false);
    }
    
    // Compare embeddings
    let similarity = compute_best_similarity(&response.embedding, &stored_embeddings);
    
    Ok(similarity > SIMILARITY_THRESHOLD)
}

fn generate_challenge() -> Vec<u8> {
    let mut rng = thread_rng();
    let mut challenge = vec![0u8; CHALLENGE_SIZE];
    rng.fill(&mut challenge[..]);
    challenge
}

fn verify_signature(embedding: &[f32], challenge: &[u8], signature: &[u8]) -> bool {
    let mut hasher = Sha256::new();
    
    for &value in embedding {
        hasher.update(value.to_le_bytes());
    }
    
    hasher.update(challenge);
    
    let expected = hasher.finalize();
    expected.as_slice() == signature
}

fn load_user_embeddings(username: &str) -> Result<Vec<Vec<f32>>> {
    use linuxSup::storage::UserStore;
    use std::path::PathBuf;
    
    // Try system store first
    let system_store = UserStore::new_with_paths(
        PathBuf::from("/var/lib/linuxsup/users"),
        PathBuf::from("/var/lib/linuxsup/enrollment"),
    )?;
    
    if let Ok(user_data) = system_store.get_user(username) {
        let mut embeddings = user_data.embeddings;
        if let Some(avg) = user_data.averaged_embedding {
            embeddings.push(avg);
        }
        return Ok(embeddings);
    }
    
    // Try user's home directory
    if let Some(home) = dirs::home_dir() {
        let user_store = UserStore::new_with_paths(
            home.join(".local/share/linuxsup/users"),
            home.join(".local/share/linuxsup/enrollment"),
        )?;
        
        if let Ok(user_data) = user_store.get_user(username) {
            let mut embeddings = user_data.embeddings;
            if let Some(avg) = user_data.averaged_embedding {
                embeddings.push(avg);
            }
            return Ok(embeddings);
        }
    }
    
    Ok(vec![])
}

fn compute_best_similarity(embedding: &[f32], stored_embeddings: &[Vec<f32>]) -> f32 {
    stored_embeddings.iter()
        .map(|stored| cosine_similarity(embedding, stored))
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(0.0)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    
    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    
    dot_product / (norm_a * norm_b)
}

// Register the PAM module
pam_module!(LinuxSupPam);