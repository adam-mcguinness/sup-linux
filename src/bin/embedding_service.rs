use linuxSup::{
    camera::Camera,
    config::Config,
    detector::FaceDetector,
    recognizer::FaceRecognizer,
    error::{Result, FaceAuthError},
};
use std::os::unix::net::{UnixListener, UnixStream};
use std::io::{Read, Write};
use std::time::{Duration, SystemTime, Instant};
use std::path::Path;
use std::fs;
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use anyhow::Context as _;

const SOCKET_PATH: &str = "/run/linuxsup/embedding.sock";
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

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

fn main() -> Result<()> {
    // Set up logging
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    
    tracing::info!("Starting LinuxSup embedding service");
    
    // Clean up old socket if exists
    if Path::new(SOCKET_PATH).exists() {
        fs::remove_file(SOCKET_PATH)?;
    }
    
    // Create socket directory
    if let Some(parent) = Path::new(SOCKET_PATH).parent() {
        fs::create_dir_all(parent)?;
    }
    
    // Create Unix socket
    let listener = UnixListener::bind(SOCKET_PATH)
        .context("Failed to bind Unix socket")?;
    
    // Set socket permissions (only root can connect)
    std::process::Command::new("chmod")
        .args(&["600", SOCKET_PATH])
        .status()?;
    
    tracing::info!("Listening on {}", SOCKET_PATH);
    
    // Initialize components
    let config = Config::load()?;
    let mut camera = Camera::new(&config)?;
    let detector = FaceDetector::new(&config)?;
    let recognizer = FaceRecognizer::new(&config)?;
    
    // Handle connections
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_client(stream, &mut camera, &detector, &recognizer) {
                    tracing::error!("Client error: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Connection error: {}", e);
            }
        }
    }
    
    Ok(())
}

fn handle_client(
    mut stream: UnixStream,
    camera: &mut Camera,
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
) -> Result<()> {
    // Set timeout
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    
    // Read request length
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let request_len = u32::from_le_bytes(len_buf) as usize;
    
    // Sanity check
    if request_len > 1024 * 1024 {  // 1MB max
        return Err(anyhow::anyhow!("Request too large: {} bytes", request_len).into());
    }
    
    // Read request
    let mut request_buf = vec![0u8; request_len];
    stream.read_exact(&mut request_buf)?;
    
    // Deserialize request
    let request: AuthRequest = bincode::deserialize(&request_buf)
        .map_err(|e| anyhow::anyhow!("Failed to deserialize request: {}", e))?;
    
    tracing::info!("Processing embedding request for user: {}", request.username);
    
    // Capture face and generate embedding
    let response = match capture_and_generate_embedding(
        camera,
        detector,
        recognizer,
        &request.challenge,
    ) {
        Ok((embedding, signature)) => AuthResponse {
            embedding,
            signature,
            timestamp: SystemTime::now(),
        },
        Err(e) => {
            tracing::error!("Failed to generate embedding: {}", e);
            // Return empty response on error
            AuthResponse {
                embedding: vec![],
                signature: vec![],
                timestamp: SystemTime::now(),
            }
        }
    };
    
    // Serialize response
    let response_data = bincode::serialize(&response)
        .map_err(|e| anyhow::anyhow!("Failed to serialize response: {}", e))?;
    let response_len = (response_data.len() as u32).to_le_bytes();
    
    // Send response
    stream.write_all(&response_len)?;
    stream.write_all(&response_data)?;
    stream.flush()?;
    
    Ok(())
}

fn capture_and_generate_embedding(
    camera: &mut Camera,
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    challenge: &[u8],
) -> Result<(Vec<f32>, Vec<u8>)> {
    let start_time = Instant::now();
    let mut session = camera.start_session()?;
    
    // Try to capture a face within timeout
    while start_time.elapsed() < CAPTURE_TIMEOUT {
        let frame = session.capture_frame()?;
        
        if let Ok(faces) = detector.detect(&frame) {
            if let Some(face) = faces.first() {
                // Generate embedding
                let embedding = recognizer.get_embedding(&frame, face)?;
                
                // Generate signature
                let signature = generate_signature(&embedding, challenge);
                
                return Ok((embedding, signature));
            }
        }
        
        // Brief pause between attempts
        std::thread::sleep(Duration::from_millis(50));
    }
    
    Err(FaceAuthError::NoFaceDetected)
}

fn generate_signature(embedding: &[f32], challenge: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    
    // Hash embedding data
    for &value in embedding {
        hasher.update(value.to_le_bytes());
    }
    
    // Hash challenge
    hasher.update(challenge);
    
    hasher.finalize().to_vec()
}