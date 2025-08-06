use linux_sup::{
    camera::Camera,
    config::Config,
    detector::FaceDetector,
    recognizer::{FaceRecognizer, cosine_similarity},
    error::Result,
    protocol::{Request, Response, AuthRequest, AuthResponse, EnrollRequest, EnrollResponse, SOCKET_PATH},
    storage::UserStore,
};
use std::os::unix::net::{UnixListener, UnixStream};
use std::io::{Read, Write};
use std::time::{Duration, SystemTime, Instant};
use std::path::{Path, PathBuf};
use std::fs;
use std::collections::VecDeque;
use sha2::{Sha256, Digest};
use anyhow::Context as _;

// Moved SOCKET_PATH to protocol module

#[derive(Debug)]
struct PeerCredentials {
    pid: u32,
    uid: u32,
    _gid: u32,
}

// Protocol types moved to linux_sup::protocol module

// Authentication state tracking
struct AuthenticationState {
    auth_attempts: VecDeque<bool>,       // K-of-N tracking
    successful_matches: u32,              // Count of successes
    embedding_buffer: VecDeque<Vec<f32>>, // For fusion
    last_face_time: Instant,             // Lost face detection
    face_detected_once: bool,            // Reset tracking
}

impl AuthenticationState {
    fn new(buffer_size: usize) -> Self {
        Self {
            auth_attempts: VecDeque::new(),
            successful_matches: 0,
            embedding_buffer: VecDeque::with_capacity(buffer_size),
            last_face_time: Instant::now(),
            face_detected_once: false,
        }
    }
}

fn get_username_from_uid(uid: u32) -> Result<String> {
    use std::ffi::CStr;
    use std::mem;
    
    unsafe {
        let mut pwd: libc::passwd = mem::zeroed();
        let mut buf = vec![0u8; 4096];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        
        let ret = libc::getpwuid_r(
            uid as libc::uid_t,
            &mut pwd,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            &mut result,
        );
        
        if ret != 0 || result.is_null() {
            return Err(anyhow::anyhow!("User not found for UID {}", uid).into());
        }
        
        let username = CStr::from_ptr((*result).pw_name)
            .to_str()
            .map_err(|_| anyhow::anyhow!("Invalid username encoding"))?
            .to_string();
        
        Ok(username)
    }
}

fn get_peer_credentials(stream: &UnixStream) -> Result<PeerCredentials> {
    use std::os::unix::io::AsRawFd;
    use std::mem;
    
    #[repr(C)]
    struct UCred {
        pid: libc::pid_t,
        uid: libc::uid_t,
        gid: libc::gid_t,
    }
    
    unsafe {
        let mut cred: UCred = mem::zeroed();
        let mut cred_len = mem::size_of::<UCred>() as libc::socklen_t;
        
        let ret = libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut cred_len,
        );
        
        if ret != 0 {
            return Err(anyhow::anyhow!("Failed to get peer credentials").into());
        }
        
        Ok(PeerCredentials {
            pid: cred.pid as u32,
            uid: cred.uid as u32,
            _gid: cred.gid as u32,
        })
    }
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
    
    // Set socket permissions (allow all users to connect)
    // Authorization is handled per-request in handle_enroll_request
    std::process::Command::new("chmod")
        .args(&["666", SOCKET_PATH])
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
                if let Err(e) = handle_client(stream, &mut camera, &detector, &recognizer, &config) {
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
    config: &Config,
) -> Result<()> {
    // Get peer credentials to identify who's connecting
    let peer_cred = get_peer_credentials(&stream)?;
    tracing::info!("Connection from UID: {}, PID: {}", peer_cred.uid, peer_cred.pid);
    
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
    let request: Request = bincode::deserialize(&request_buf)
        .map_err(|e| anyhow::anyhow!("Failed to deserialize request: {}", e))?;
    
    // Process request based on type
    let response = match request {
        Request::Authenticate(auth_req) => {
            tracing::info!("Processing auth request for user: {}", auth_req.username);
            handle_auth_request(camera, detector, recognizer, auth_req, config)
        }
        Request::Enroll(enroll_req) => {
            tracing::info!("Processing enrollment request for user: {}", enroll_req.username);
            handle_enroll_request(camera, detector, recognizer, enroll_req, &peer_cred, config)
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

fn handle_auth_request(
    camera: &mut Camera,
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    request: AuthRequest,
    config: &Config,
) -> Response {
    match perform_authentication(camera, detector, recognizer, &request.username, &request.challenge, config) {
        Ok(auth_response) => Response::Auth(auth_response),
        Err(e) => {
            tracing::error!("Auth failed: {}", e);
            Response::Error(format!("Authentication failed: {}", e))
        }
    }
}

fn handle_enroll_request(
    camera: &mut Camera,
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    request: EnrollRequest,
    peer_cred: &PeerCredentials,
    config: &Config,
) -> Response {
    use linux_sup::quality::QualityMetrics;
    
    // Authorization check: Users can only enroll themselves unless they're root
    if peer_cred.uid != 0 {
        // Not root - check if they're trying to enroll themselves
        let requesting_user = get_username_from_uid(peer_cred.uid);
        if let Ok(req_user) = requesting_user {
            if req_user != request.username {
                tracing::warn!("User {} (UID {}) attempted to enroll as {}", 
                    req_user, peer_cred.uid, request.username);
                return Response::Enroll(EnrollResponse {
                    success: false,
                    message: format!("Permission denied: You can only enroll yourself"),
                });
            }
        } else {
            tracing::warn!("Could not determine username for UID {}", peer_cred.uid);
            return Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to verify user identity"),
            });
        }
    }
    
    tracing::info!("Starting enrollment for user: {} (requested by UID: {})", 
        request.username, peer_cred.uid);
    
    // Create user store with system paths
    let store = match UserStore::new_with_paths(
        PathBuf::from("/var/lib/linuxsup/users"),
        PathBuf::from("/var/lib/linuxsup/enrollment"),
    ) {
        Ok(s) => s,
        Err(e) => {
            return Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to initialize storage: {}", e),
            });
        }
    };
    
    // Create enrollment images directory for this user
    let enrollment_dir = match store.get_enrollment_images_dir(&request.username) {
        Ok(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return Response::Enroll(EnrollResponse {
                    success: false,
                    message: format!("Failed to create enrollment directory: {}", e),
                });
            }
            dir
        }
        Err(e) => {
            return Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to get enrollment directory: {}", e),
            });
        }
    };
    
    // Capture multiple images
    let mut embeddings = Vec::new();
    let mut quality_scores = Vec::new();
    let total_captures = config.enrollment.num_captures.unwrap_or(5);
    let min_quality = config.enrollment.min_enrollment_quality;
    
    tracing::info!("Capturing {} images for enrollment", total_captures);
    
    // Start camera session
    let mut session = match camera.start_session() {
        Ok(s) => s,
        Err(e) => {
            return Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to start camera: {}", e),
            });
        }
    };
    
    let mut captured = 0;
    let mut attempts = 0;
    let max_attempts = 30;
    let capture_interval = Duration::from_millis(
        config.enrollment.capture_interval_ms.unwrap_or(2000)
    );
    let mut last_capture_time = Instant::now();
    
    while captured < total_captures && attempts < max_attempts {
        attempts += 1;
        
        // Capture frame
        let frame = match session.capture_frame() {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to capture frame: {}", e);
                continue;
            }
        };
        
        // Detect faces
        let faces = match detector.detect(&frame) {
            Ok(f) if !f.is_empty() => f,
            _ => continue,
        };
        
        // Check if enough time has passed since last capture
        if last_capture_time.elapsed() < capture_interval && captured > 0 {
            continue;
        }
        
        let face = &faces[0];
        
        // Calculate quality metrics
        let quality = QualityMetrics::calculate(&frame, face);
        
        // Check if quality meets requirements
        if !quality.meets_minimum_requirements(min_quality) {
            tracing::debug!("Image quality too low: {:.2}", quality.overall_score);
            continue;
        }
        
        // Get embedding
        let embedding = match recognizer.get_embedding(&frame, face) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to get embedding: {}", e);
                continue;
            }
        };
        
        // Save enrollment image
        let image_path = enrollment_dir.join(format!("enroll_{}.jpg", captured));
        if let Err(e) = frame.save(&image_path) {
            tracing::warn!("Failed to save enrollment image: {}", e);
        }
        
        embeddings.push(embedding);
        quality_scores.push(quality.overall_score);
        captured += 1;
        last_capture_time = Instant::now();
        
        tracing::info!("Captured image {}/{} with quality {:.2}", captured, total_captures, quality.overall_score);
    }
    
    if embeddings.is_empty() {
        return Response::Enroll(EnrollResponse {
            success: false,
            message: "Failed to capture any valid face images".to_string(),
        });
    }
    
    // Calculate averaged embedding
    let averaged_embedding = if !embeddings.is_empty() {
        let embedding_size = embeddings[0].len();
        let mut averaged = vec![0.0f32; embedding_size];
        
        for embedding in &embeddings {
            for (i, &value) in embedding.iter().enumerate() {
                averaged[i] += value;
            }
        }
        
        let count = embeddings.len() as f32;
        for value in &mut averaged {
            *value /= count;
        }
        
        Some(averaged)
    } else {
        None
    };
    
    // Create user data
    let user_data = linux_sup::storage::UserData {
        version: 1,
        username: request.username.clone(),
        embeddings,
        averaged_embedding,
        embedding_qualities: Some(quality_scores),
    };
    
    // Save user data
    match store.save_user_data(&user_data) {
        Ok(_) => {
            tracing::info!("Successfully enrolled user: {}", request.username);
            Response::Enroll(EnrollResponse {
                success: true,
                message: format!("Successfully enrolled user '{}' with {} face captures", request.username, user_data.embeddings.len()),
            })
        }
        Err(e) => {
            tracing::error!("Failed to save user data: {}", e);
            Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to save enrollment data: {}", e),
            })
        }
    }
}

fn perform_authentication(
    camera: &mut Camera,
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    username: &str,
    challenge: &[u8],
    config: &Config,
) -> Result<AuthResponse> {
    // Load user's stored embeddings
    let store = UserStore::new_with_paths(
        PathBuf::from("/var/lib/linuxsup/users"),
        PathBuf::from("/var/lib/linuxsup/enrollment"),
    )?;
    
    let user_data = match store.get_user(username) {
        Ok(data) => data,
        Err(_) => {
            return Ok(AuthResponse {
                success: false,
                message: format!("User {} not enrolled", username),
                attempts: 0,
                signature: vec![],
                timestamp: SystemTime::now(),
            });
        }
    };
    
    // Initialize authentication state
    let mut state = AuthenticationState::new(config.auth.embedding_buffer_size as usize);
    
    // Start camera session
    let mut session = camera.start_session()?;
    tracing::info!("Starting authentication for user: {}", username);
    
    let start_time = Instant::now();
    let timeout = Duration::from_secs(config.auth.timeout_seconds as u64);
    let lost_face_timeout = Duration::from_secs(config.auth.lost_face_timeout as u64);
    let mut total_attempts = 0;
    
    // Authentication loop
    while start_time.elapsed() < timeout {
        total_attempts += 1;
        
        // Check if we've lost the face for too long
        if state.face_detected_once && state.last_face_time.elapsed() > lost_face_timeout {
            tracing::info!("Face lost - resetting authentication progress");
            // Reset K-of-N tracking
            state = AuthenticationState::new(config.auth.embedding_buffer_size as usize);
        }
        
        // Capture frame
        let frame = match session.capture_frame() {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to capture frame: {}", e);
                continue;
            }
        };
        
        // Detect faces
        match detector.detect(&frame) {
            Ok(faces) if !faces.is_empty() => {
                if !state.face_detected_once {
                    tracing::info!("Face detected, beginning verification");
                }
                state.face_detected_once = true;
                state.last_face_time = Instant::now();
                
                let face = &faces[0];
                
                // Get embedding
                let embedding = match recognizer.get_embedding(&frame, face) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!("Failed to get embedding: {}", e);
                        continue;
                    }
                };
                
                // Add to buffer for fusion
                state.embedding_buffer.push_back(embedding.clone());
                if state.embedding_buffer.len() > config.auth.embedding_buffer_size as usize {
                    state.embedding_buffer.pop_front();
                }
                
                // Calculate best similarity
                let similarity = calculate_best_similarity(
                    &embedding,
                    &state.embedding_buffer,
                    &user_data,
                    config.auth.use_embedding_fusion
                );
                
                // Update K-of-N tracking
                let success = similarity > config.auth.similarity_threshold;
                state.auth_attempts.push_back(success);
                if success {
                    state.successful_matches += 1;
                }
                
                // Maintain sliding window
                while state.auth_attempts.len() > config.auth.n_total_attempts as usize {
                    if state.auth_attempts.pop_front() == Some(true) {
                        state.successful_matches -= 1;
                    }
                }
                
                tracing::debug!("Auth attempt: similarity={:.3}, success={}, matches={}/{}", 
                    similarity, success, state.successful_matches, config.auth.k_required_matches);
                
                // Check for K successes
                if state.successful_matches >= config.auth.k_required_matches {
                    tracing::info!("Authentication successful after {} attempts", total_attempts);
                    
                    // Generate signature using the current embedding
                    let signature = generate_signature(&embedding, challenge);
                    
                    return Ok(AuthResponse {
                        success: true,
                        message: format!("Authenticated after {} attempts", total_attempts),
                        attempts: total_attempts,
                        signature,
                        timestamp: SystemTime::now(),
                    });
                }
            }
            Ok(_) => {}, // No face detected, continue
            Err(e) => {
                tracing::warn!("Detection error: {}", e);
            }
        }
        
        // Brief pause between attempts
        std::thread::sleep(Duration::from_millis(50));
    }
    
    // Timeout
    tracing::info!("Authentication timeout for user {} after {} attempts", username, total_attempts);
    Ok(AuthResponse {
        success: false,
        message: "Authentication timeout".to_string(),
        attempts: total_attempts,
        signature: vec![],
        timestamp: SystemTime::now(),
    })
}

fn calculate_best_similarity(
    embedding: &[f32],
    embedding_buffer: &VecDeque<Vec<f32>>,
    user_data: &linux_sup::storage::UserData,
    use_fusion: bool,
) -> f32 {
    let mut best_similarity = 0.0f32;
    
    // Check individual embedding against stored embeddings
    for stored_embedding in user_data.embeddings.iter() {
        let similarity = cosine_similarity(embedding, stored_embedding);
        best_similarity = best_similarity.max(similarity);
    }
    
    // Check against averaged stored embedding if available
    if let Some(ref avg_stored) = user_data.averaged_embedding {
        let similarity = cosine_similarity(embedding, avg_stored);
        best_similarity = best_similarity.max(similarity);
    }
    
    // Check fused embedding if enabled and we have enough samples
    if use_fusion && embedding_buffer.len() >= 2 {
        let fused_embedding = average_embeddings_buffer(embedding_buffer);
        
        for stored_embedding in user_data.embeddings.iter() {
            let similarity = cosine_similarity(&fused_embedding, stored_embedding);
            best_similarity = best_similarity.max(similarity);
        }
        
        if let Some(ref avg_stored) = user_data.averaged_embedding {
            let similarity = cosine_similarity(&fused_embedding, avg_stored);
            best_similarity = best_similarity.max(similarity);
        }
    }
    
    best_similarity
}

fn average_embeddings_buffer(buffer: &VecDeque<Vec<f32>>) -> Vec<f32> {
    if buffer.is_empty() {
        return vec![];
    }
    
    let embedding_size = buffer[0].len();
    let mut averaged = vec![0.0f32; embedding_size];
    
    for embedding in buffer.iter() {
        for (i, &value) in embedding.iter().enumerate() {
            averaged[i] += value;
        }
    }
    
    let count = buffer.len() as f32;
    for value in &mut averaged {
        *value /= count;
    }
    
    averaged
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