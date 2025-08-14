use sup_linux::{
    camera::Camera,
    config::Config,
    detector::FaceDetector,
    recognizer::{FaceRecognizer, cosine_similarity},
    error::Result,
    protocol::{
        Request, Response, AuthRequest, AuthResponse, EnrollRequest, EnrollResponse, 
        EnhanceRequest, EnhanceResponse, StreamMessage, MSG_TYPE_RESPONSE, MSG_TYPE_STREAM
    },
    storage::UserStore,
    cli::ascii_preview::AsciiRenderer,
};
use clap::Parser;
use std::os::unix::net::{UnixListener, UnixStream};
use std::io::{Read, Write};
use std::time::{Duration, SystemTime, Instant};
use std::path::{Path, PathBuf};
use std::fs;
use std::collections::VecDeque;
use sha2::{Sha256, Digest};
use anyhow::Context as _;

#[derive(Parser, Debug)]
#[command(name = "suplinux-service")]
#[command(about = "SupLinux authentication service")]
struct Args {
    /// Run in development mode
    #[arg(long)]
    dev: bool,
    
    /// Socket path in dev mode
    #[arg(long, default_value = "/tmp/suplinux.sock")]
    dev_socket: String,
    
    /// Data directory in dev mode
    #[arg(long, default_value = "./dev_data")]
    dev_data_dir: String,
}

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
    // Parse command-line arguments
    let args = Args::parse();
    
    // Set up logging
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    
    tracing::info!("Starting SupLinux service (dev_mode: {})", args.dev);
    
    // Determine paths based on mode
    let (socket_path, data_dir, config_path) = if args.dev {
        (
            args.dev_socket.as_str(),
            PathBuf::from(args.dev_data_dir),
            PathBuf::from("configs/face-auth.toml"),
        )
    } else {
        (
            "/run/suplinux/service.sock",
            PathBuf::from("/var/lib/suplinux"),
            PathBuf::from("/etc/suplinux/face-auth.toml"),
        )
    };
    
    // Clean up old socket if exists
    if Path::new(socket_path).exists() {
        fs::remove_file(socket_path)?;
    }
    
    // Create socket directory
    if let Some(parent) = Path::new(socket_path).parent() {
        fs::create_dir_all(parent)?;
    }
    
    // Create Unix socket
    let listener = UnixListener::bind(socket_path)
        .context("Failed to bind Unix socket")?;
    
    // Set socket permissions (allow all users to connect)
    // Authorization is handled per-request in handle_enroll_request
    std::process::Command::new("chmod")
        .args(&["666", socket_path])
        .status()?;
    
    tracing::info!("Listening on {}", socket_path);
    
    // Initialize components (but NOT camera - we'll create it per request)
    let config = if config_path.exists() {
        Config::load_from_path(&config_path)?
    } else {
        Config::load()?
    };
    // Only initialize models once - they can be reused
    let detector = FaceDetector::new(&config)?;
    let recognizer = FaceRecognizer::new(&config)?;
    
    // Handle connections
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_client(stream, &detector, &recognizer, &config, &data_dir) {
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
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    config: &Config,
    data_dir: &Path,
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
    
    // Process request based on type - enrollment/enhance may stream updates
    match request {
        Request::Authenticate(auth_req) => {
            tracing::info!("Processing auth request for user: {}", auth_req.username);
            let response = handle_auth_request(detector, recognizer, auth_req, config, data_dir);
            
            // Send response (no streaming for auth)
            let response_data = bincode::serialize(&response)
                .map_err(|e| anyhow::anyhow!("Failed to serialize response: {}", e))?;
            let response_len = (response_data.len() as u32).to_le_bytes();
            
            stream.write_all(&response_len)?;
            stream.write_all(&response_data)?;
            stream.flush()?;
        }
        Request::Enroll(enroll_req) => {
            tracing::info!("Processing enrollment request for user: {}", enroll_req.username);
            handle_enroll_request_with_stream(&mut stream, detector, recognizer, enroll_req, &peer_cred, config, data_dir)?;
        }
        Request::Enhance(enhance_req) => {
            tracing::info!("Processing enhance request for user: {}", enhance_req.username);
            handle_enhance_request_with_stream(&mut stream, detector, recognizer, enhance_req, &peer_cred, config, data_dir)?;
        }
    }
    
    Ok(())
}

fn handle_auth_request(
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    request: AuthRequest,
    config: &Config,
    data_dir: &Path,
) -> Response {
    // Create camera just for this authentication
    let mut camera = match Camera::new(config) {
        Ok(c) => c,
        Err(e) => {
            return Response::Error(format!("Failed to initialize camera: {}", e));
        }
    };
    
    let result = perform_authentication(&mut camera, detector, recognizer, &request.username, &request.challenge, config, data_dir);
    
    // Camera will be dropped here, releasing the device
    drop(camera);
    
    match result {
        Ok(auth_response) => Response::Auth(auth_response),
        Err(e) => {
            tracing::error!("Auth failed: {}", e);
            Response::Error(format!("Authentication failed: {}", e))
        }
    }
}

// Helper function to format enrollment report
fn format_enrollment_report(
    username: &str,
    captured: usize,
    total: usize,
    quality_scores: &[f32],
    consistency: f32,
    success: bool,
    width: usize,
    height: usize,
) -> String {
    let mut lines = Vec::new();
    
    // Header
    lines.push("╔══════════════════════════════════════════════════════╗".to_string());
    lines.push("║         ENROLLMENT COMPLETE - REPORT                 ║".to_string());
    lines.push("╚══════════════════════════════════════════════════════╝".to_string());
    lines.push(String::new());
    
    // User and status
    lines.push(format!("User: {}", username));
    if success {
        lines.push(format!("Status: ✅ SUCCESS ({}/{} captures)", captured, total));
    } else {
        lines.push(format!("Status: ❌ FAILED ({}/{} captures)", captured, total));
    }
    lines.push(String::new());
    
    // Quality scores if we have any captures
    if !quality_scores.is_empty() {
        lines.push("📊 Quality Scores:".to_string());
        let mut total_quality = 0.0;
        for (i, score) in quality_scores.iter().enumerate() {
            let percentage = (score * 100.0) as u32;
            let bar_length = (percentage as usize * 20) / 100;
            let bar = "█".repeat(bar_length);
            let empty = "░".repeat(20_usize.saturating_sub(bar_length));
            lines.push(format!("  Capture {}: [{}{}] {}%", i + 1, bar, empty, percentage));
            total_quality += score;
        }
        lines.push(String::new());
        
        // Average quality
        let avg_quality = total_quality / quality_scores.len() as f32;
        let avg_percentage = (avg_quality * 100.0) as u32;
        let rating = if avg_quality >= 0.8 {
            "Excellent ⭐⭐⭐⭐⭐"
        } else if avg_quality >= 0.7 {
            "Good ⭐⭐⭐⭐"
        } else if avg_quality >= 0.6 {
            "Acceptable ⭐⭐⭐"
        } else {
            "Poor ⭐⭐"
        };
        lines.push(format!("📈 Average Quality: {}% ({})", avg_percentage, rating));
        
        // Consistency score
        let consistency_percentage = (consistency * 100.0) as u32;
        let consistency_rating = if consistency >= 0.85 {
            "Excellent - Optimal variation"
        } else if consistency >= 0.75 {
            "Good - Well balanced"
        } else if consistency >= 0.65 {
            "Acceptable - Adequate"
        } else {
            "Poor - Too inconsistent"
        };
        lines.push(format!("🔄 Consistency: {}% ({})", consistency_percentage, consistency_rating));
        lines.push(String::new());
    }
    
    // Final message
    if success {
        lines.push("✅ Enrollment successful!".to_string());
        lines.push(format!("   {} high-quality face captures saved", captured));
    } else {
        lines.push("❌ Enrollment failed!".to_string());
        lines.push(String::new());
        lines.push("⚠️ Suggestions:".to_string());
        if captured == 0 {
            lines.push("   • Ensure your face is visible to the camera".to_string());
            lines.push("   • Check lighting conditions".to_string());
            lines.push("   • Remove glasses if wearing them".to_string());
        } else if captured < total {
            lines.push("   • Keep your face in view throughout enrollment".to_string());
            lines.push("   • Maintain consistent distance from camera".to_string());
            lines.push("   • Try better lighting conditions".to_string());
        }
        if !quality_scores.is_empty() {
            let avg_quality = quality_scores.iter().sum::<f32>() / quality_scores.len() as f32;
            if avg_quality < 0.6 {
                lines.push("   • Image quality was too low".to_string());
                lines.push("   • Clean the camera lens".to_string());
            }
        }
    }
    
    // Pad to requested height if needed
    while lines.len() < height {
        lines.push(String::new());
    }
    
    // Ensure all lines are properly padded to width
    lines.iter()
        .take(height)
        .map(|line| {
            if line.len() > width {
                line.chars().take(width).collect()
            } else {
                format!("{:width$}", line, width = width)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// Helper function to format enhancement report
fn format_enhancement_report(
    username: &str,
    captured: usize,
    total: usize,
    embeddings_before: usize,
    embeddings_after: usize,
    quality_scores: &[f32],
    replaced: usize,
    success: bool,
    width: usize,
    height: usize,
) -> String {
    let mut lines = Vec::new();
    
    // Header
    lines.push("╔══════════════════════════════════════════════════════╗".to_string());
    lines.push("║        ENHANCEMENT COMPLETE - REPORT                 ║".to_string());
    lines.push("╚══════════════════════════════════════════════════════╝".to_string());
    lines.push(String::new());
    
    // User and status
    lines.push(format!("User: {}", username));
    if success {
        lines.push(format!("Status: ✅ SUCCESS ({}/{} new captures)", captured, total));
    } else {
        lines.push(format!("Status: ⚠️  PARTIAL ({}/{} new captures)", captured, total));
    }
    lines.push(String::new());
    
    // Embedding changes
    lines.push("📊 Embedding Changes:".to_string());
    lines.push(format!("  Before: {} embeddings", embeddings_before));
    lines.push(format!("  After:  {} embeddings", embeddings_after));
    if replaced > 0 {
        lines.push(format!("  Replaced: {} weak embeddings", replaced));
    } else {
        lines.push(format!("  Added:  {} new embeddings", embeddings_after - embeddings_before));
    }
    lines.push(String::new());
    
    // Quality scores for new captures
    if !quality_scores.is_empty() {
        lines.push("📈 New Capture Quality:".to_string());
        for (i, score) in quality_scores.iter().enumerate() {
            let percentage = (score * 100.0) as u32;
            let bar_length = (percentage as usize * 20) / 100;
            let bar = "█".repeat(bar_length);
            let empty = "░".repeat(20_usize.saturating_sub(bar_length));
            lines.push(format!("  Capture {}: [{}{}] {}%", i + 1, bar, empty, percentage));
        }
        
        let avg_quality = quality_scores.iter().sum::<f32>() / quality_scores.len() as f32;
        let avg_percentage = (avg_quality * 100.0) as u32;
        lines.push(format!("  Average: {}%", avg_percentage));
        lines.push(String::new());
    }
    
    // Final message
    if success && captured > 0 {
        lines.push("✅ Enhancement successful!".to_string());
        lines.push("   Your enrollment is now more robust".to_string());
    } else if captured > 0 {
        lines.push("⚠️  Partial enhancement completed".to_string());
        lines.push(format!("   Added {} new captures", captured));
    } else {
        lines.push("❌ Enhancement failed!".to_string());
        lines.push("   No new captures were added".to_string());
    }
    
    // Pad to requested height if needed
    while lines.len() < height {
        lines.push(String::new());
    }
    
    // Ensure all lines are properly padded to width
    lines.iter()
        .take(height)
        .map(|line| {
            if line.len() > width {
                line.chars().take(width).collect()
            } else {
                format!("{:width$}", line, width = width)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// Helper function to send stream messages
fn send_stream_message(stream: &mut UnixStream, msg: &StreamMessage) -> Result<()> {
    let msg_data = bincode::serialize(msg)
        .map_err(|e| anyhow::anyhow!("Failed to serialize stream message: {}", e))?;
    let msg_len = (msg_data.len() as u32).to_le_bytes();
    
    stream.write_all(&[MSG_TYPE_STREAM])?;
    stream.write_all(&msg_len)?;
    stream.write_all(&msg_data)?;
    stream.flush()?;
    
    Ok(())
}

// Helper function to send final response
fn send_final_response(stream: &mut UnixStream, response: &Response) -> Result<()> {
    let response_data = bincode::serialize(response)
        .map_err(|e| anyhow::anyhow!("Failed to serialize response: {}", e))?;
    let response_len = (response_data.len() as u32).to_le_bytes();
    
    stream.write_all(&[MSG_TYPE_RESPONSE])?;
    stream.write_all(&response_len)?;
    stream.write_all(&response_data)?;
    stream.flush()?;
    
    Ok(())
}

// Wrapper function that handles streaming for enrollment
fn handle_enroll_request_with_stream(
    stream: &mut UnixStream,
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    request: EnrollRequest,
    peer_cred: &PeerCredentials,
    config: &Config,
    data_dir: &Path,
) -> Result<()> {
    // Check if preview is enabled
    if request.enable_preview {
        // Call the enhanced version with streaming
        let response = handle_enroll_request_streaming(
            stream,
            detector,
            recognizer,
            request,
            peer_cred,
            config,
            data_dir,
        )?;
        
        // Send complete message followed by final response
        send_stream_message(stream, &StreamMessage::Complete)?;
        send_final_response(stream, &response)?;
    } else {
        // Call the original non-streaming version
        let response = handle_enroll_request(
            detector,
            recognizer,
            request,
            peer_cred,
            config,
            data_dir,
        );
        
        // Send response without streaming
        send_final_response(stream, &response)?;
    }
    
    Ok(())
}

// Wrapper function that handles streaming for enhancement
fn handle_enhance_request_with_stream(
    stream: &mut UnixStream,
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    request: EnhanceRequest,
    peer_cred: &PeerCredentials,
    config: &Config,
    data_dir: &Path,
) -> Result<()> {
    // Check if preview is enabled
    if request.enable_preview {
        // Call the enhanced version with streaming
        let response = handle_enhance_request_streaming(
            stream,
            detector,
            recognizer,
            request,
            peer_cred,
            config,
            data_dir,
        )?;
        
        // Send complete message followed by final response
        send_stream_message(stream, &StreamMessage::Complete)?;
        send_final_response(stream, &response)?;
    } else {
        // Call the original non-streaming version
        let response = handle_enhance_request(
            detector,
            recognizer,
            request,
            peer_cred,
            config,
            data_dir,
        );
        
        // Send response without streaming
        send_final_response(stream, &response)?;
    }
    
    Ok(())
}

// Streaming version of enrollment that sends ASCII preview frames
fn handle_enroll_request_streaming(
    stream: &mut UnixStream,
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    request: EnrollRequest,
    peer_cred: &PeerCredentials,
    config: &Config,
    data_dir: &Path,
) -> Result<Response> {
    use sup_linux::quality::QualityMetrics;
    
    // Authorization check: Users can only enroll themselves unless they're root
    if peer_cred.uid != 0 {
        let requesting_user = get_username_from_uid(peer_cred.uid);
        if let Ok(req_user) = requesting_user {
            if req_user != request.username {
                tracing::warn!("User {} (UID {}) attempted to enroll as {}", 
                    req_user, peer_cred.uid, request.username);
                return Ok(Response::Enroll(EnrollResponse {
                    success: false,
                    message: format!("Permission denied: You can only enroll yourself"),
                }));
            }
        } else {
            tracing::warn!("Could not determine username for UID {}", peer_cred.uid);
            return Ok(Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to verify user identity"),
            }));
        }
    }
    
    tracing::info!("Starting streaming enrollment for user: {} (requested by UID: {})", 
        request.username, peer_cred.uid);
    
    // Create user store with appropriate paths
    let store = match UserStore::new_with_paths(
        data_dir.join("users"),
        data_dir.join("enrollment"),
    ) {
        Ok(s) => s,
        Err(e) => {
            return Ok(Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to initialize storage: {}", e),
            }));
        }
    };
    
    // Create enrollment images directory for this user
    let enrollment_dir = match store.get_enrollment_images_dir(&request.username) {
        Ok(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return Ok(Response::Enroll(EnrollResponse {
                    success: false,
                    message: format!("Failed to create enrollment directory: {}", e),
                }));
            }
            dir
        }
        Err(e) => {
            return Ok(Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to get enrollment directory: {}", e),
            }));
        }
    };
    
    // Create camera just for this enrollment
    let mut camera = match Camera::new(config) {
        Ok(c) => c,
        Err(e) => {
            return Ok(Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to initialize camera: {}", e),
            }));
        }
    };
    
    // Create ASCII renderer for preview
    let renderer = AsciiRenderer::new(
        config.enrollment.ascii_width,
        config.enrollment.ascii_height
    );
    
    // Capture multiple images
    let mut embeddings = Vec::new();
    let mut quality_scores = Vec::new();
    let total_captures = config.enrollment.num_captures.unwrap_or(5);
    let min_quality = config.enrollment.min_enrollment_quality;
    
    tracing::info!("Capturing {} images for enrollment with ASCII preview", total_captures);
    
    // Start camera session
    let mut session = match camera.start_session() {
        Ok(s) => s,
        Err(e) => {
            return Ok(Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to start camera: {}", e),
            }));
        }
    };
    
    let mut captured = 0;
    let capture_interval_ms = config.enrollment.capture_interval_ms.unwrap_or(2000);
    let capture_interval = Duration::from_millis(capture_interval_ms);
    
    // Calculate dynamic timeout: num_captures * interval * 5 for overhead
    let enrollment_timeout = Duration::from_millis(
        total_captures as u64 * capture_interval_ms * 5
    );
    let enrollment_start = Instant::now();
    let mut last_capture_time = Instant::now();
    
    tracing::info!("Enrollment timeout set to {:.1}s for {} captures with {:.1}s intervals",
                 enrollment_timeout.as_secs_f32(), total_captures, capture_interval.as_secs_f32());
    
    while captured < total_captures && enrollment_start.elapsed() < enrollment_timeout {
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
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to detect faces: {}", e);
                vec![]
            }
        };
        
        // Send ASCII preview frame
        let ascii = renderer.render_frame_with_progress(
            &frame,
            &faces,
            captured,
            total_captures
        );
        
        if let Err(e) = send_stream_message(stream, &StreamMessage::PreviewFrame { 
            ascii,
            captured,
            total: total_captures,
        }) {
            tracing::warn!("Failed to send preview frame: {}", e);
            // Continue even if preview fails
        }
        
        // Check if we have a face and enough time has passed
        if !faces.is_empty() && last_capture_time.elapsed() >= capture_interval {
            let face = &faces[0];
            
            // Calculate quality metrics
            let quality = QualityMetrics::calculate(&frame, face);
            
            // Check if quality meets requirements
            if quality.meets_minimum_requirements(min_quality) {
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
                
                // Use debug level to avoid interfering with ASCII preview
                tracing::debug!("Captured image {}/{} with quality {:.2}", captured, total_captures, quality.overall_score);
                
                // Send status update through the stream (not to stderr)
                if let Err(e) = send_stream_message(stream, &StreamMessage::StatusUpdate { 
                    message: format!("Captured image {}/{} with quality {:.2}", captured, total_captures, quality.overall_score),
                }) {
                    tracing::debug!("Failed to send status update: {}", e);
                }
            } else {
                tracing::debug!("Image quality too low: {:.2}", quality.overall_score);
            }
        }
        
        // Small delay to prevent CPU hogging
        std::thread::sleep(Duration::from_millis(50));
    }
    
    // Check if we have enough captures
    let success = captured >= total_captures;
    
    // Calculate consistency if we have embeddings
    let consistency = if embeddings.len() > 1 {
        sup_linux::quality::calculate_embedding_consistency(&embeddings)
    } else {
        0.0
    };
    
    // Send the enrollment report as final frame
    let report = format_enrollment_report(
        &request.username,
        captured,
        total_captures,
        &quality_scores,
        consistency,
        success,
        config.enrollment.ascii_width.unwrap_or(60),
        config.enrollment.ascii_height.unwrap_or(25),
    );
    
    if let Err(e) = send_stream_message(stream, &StreamMessage::PreviewFrame {
        ascii: report,
        captured,
        total: total_captures,
    }) {
        tracing::debug!("Failed to send enrollment report: {}", e);
    }
    
    // If enrollment failed, return early
    if !success {
        return Ok(Response::Enroll(EnrollResponse {
            success: false,
            message: format!("Enrollment failed: only {}/{} captures completed", captured, total_captures),
        }));
    }
    
    // Calculate averaged embedding for successful enrollment
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
    let user_data = sup_linux::storage::UserData {
        version: 1,
        username: request.username.clone(),
        embeddings,
        averaged_embedding,
        embedding_qualities: Some(quality_scores.clone()),
    };
    
    // Save user data
    if let Err(e) = store.save_user_data(&user_data) {
        return Ok(Response::Enroll(EnrollResponse {
            success: false,
            message: format!("Failed to save user data: {}", e),
        }));
    }
    
    Ok(Response::Enroll(EnrollResponse {
        success: true,
        message: format!("User '{}' enrolled successfully with {} face captures", 
                        request.username, user_data.embeddings.len()),
    }))
}

fn handle_enroll_request(
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    request: EnrollRequest,
    peer_cred: &PeerCredentials,
    config: &Config,
    data_dir: &Path,
) -> Response {
    use sup_linux::quality::QualityMetrics;
    
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
    
    // Create user store with appropriate paths
    let store = match UserStore::new_with_paths(
        data_dir.join("users"),
        data_dir.join("enrollment"),
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
    
    // Create camera just for this enrollment
    let mut camera = match Camera::new(config) {
        Ok(c) => c,
        Err(e) => {
            return Response::Enroll(EnrollResponse {
                success: false,
                message: format!("Failed to initialize camera: {}", e),
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
    let capture_interval_ms = config.enrollment.capture_interval_ms.unwrap_or(2000);
    let capture_interval = Duration::from_millis(capture_interval_ms);
    
    // Calculate dynamic timeout: num_captures * interval * 3 for overhead
    let enrollment_timeout = Duration::from_millis(
        total_captures as u64 * capture_interval_ms * 3
    );
    let enrollment_start = Instant::now();
    let mut last_capture_time = Instant::now();
    
    tracing::info!("Enrollment timeout set to {:.1}s for {} captures with {:.1}s intervals",
                 enrollment_timeout.as_secs_f32(), total_captures, capture_interval.as_secs_f32());
    
    while captured < total_captures && enrollment_start.elapsed() < enrollment_timeout {
        
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
    let user_data = sup_linux::storage::UserData {
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
    data_dir: &Path,
) -> Result<AuthResponse> {
    // Load user's stored embeddings
    let store = UserStore::new_with_paths(
        data_dir.join("users"),
        data_dir.join("enrollment"),
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
    user_data: &sup_linux::storage::UserData,
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

// Streaming version of enhancement that sends ASCII preview frames
fn handle_enhance_request_streaming(
    stream: &mut UnixStream,
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    request: EnhanceRequest,
    peer_cred: &PeerCredentials,
    config: &Config,
    data_dir: &Path,
) -> Result<Response> {
    use sup_linux::quality::QualityMetrics;
    
    // Authorization check: Users can only enhance themselves unless they're root
    if peer_cred.uid != 0 {
        let requesting_user = get_username_from_uid(peer_cred.uid);
        if let Ok(req_user) = requesting_user {
            if req_user != request.username {
                tracing::warn!("User {} (UID {}) attempted to enhance as {}", 
                    req_user, peer_cred.uid, request.username);
                return Ok(Response::Enhance(EnhanceResponse {
                    success: false,
                    message: format!("Permission denied: You can only enhance your own enrollment"),
                    embeddings_before: 0,
                    embeddings_after: 0,
                    replaced_count: 0,
                }));
            }
        } else {
            tracing::warn!("Could not determine username for UID {}", peer_cred.uid);
            return Ok(Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to verify user identity"),
                embeddings_before: 0,
                embeddings_after: 0,
                replaced_count: 0,
            }));
        }
    }
    
    tracing::info!("Starting streaming enhancement for user: {} (requested by UID: {})", 
        request.username, peer_cred.uid);
    
    // Create user store
    let store = match UserStore::new_with_paths(
        data_dir.join("users"),
        data_dir.join("enrollment"),
    ) {
        Ok(s) => s,
        Err(e) => {
            return Ok(Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to initialize storage: {}", e),
                embeddings_before: 0,
                embeddings_after: 0,
                replaced_count: 0,
            }));
        }
    };
    
    // Load existing user data
    let mut user_data = match store.get_user(&request.username) {
        Ok(data) => data,
        Err(_) => {
            return Ok(Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("User {} not found. Please enroll first.", request.username),
                embeddings_before: 0,
                embeddings_after: 0,
                replaced_count: 0,
            }));
        }
    };
    
    let embeddings_before = user_data.embeddings.len();
    
    // Get enrollment images directory
    let enrollment_dir = match store.get_enrollment_images_dir(&request.username) {
        Ok(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return Ok(Response::Enhance(EnhanceResponse {
                    success: false,
                    message: format!("Failed to create enrollment directory: {}", e),
                    embeddings_before,
                    embeddings_after: embeddings_before,
                    replaced_count: 0,
                }));
            }
            dir
        }
        Err(e) => {
            return Ok(Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to get enrollment directory: {}", e),
                embeddings_before,
                embeddings_after: embeddings_before,
                replaced_count: 0,
            }));
        }
    };
    
    // Create camera just for this enhancement
    let mut camera = match Camera::new(config) {
        Ok(c) => c,
        Err(e) => {
            return Ok(Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to initialize camera: {}", e),
                embeddings_before,
                embeddings_after: embeddings_before,
                replaced_count: 0,
            }));
        }
    };
    
    // Create ASCII renderer for preview
    let renderer = AsciiRenderer::new(
        config.enrollment.ascii_width,
        config.enrollment.ascii_height
    );
    
    // Capture additional images
    let mut new_embeddings = Vec::new();
    let mut new_quality_scores = Vec::new();
    let additional_captures = request.additional_captures.unwrap_or(3) as usize;
    let min_quality = config.enrollment.min_enrollment_quality;
    
    tracing::info!("Capturing {} additional images for enhancement with ASCII preview", additional_captures);
    
    // Start camera session
    let mut session = match camera.start_session() {
        Ok(s) => s,
        Err(e) => {
            return Ok(Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to start camera: {}", e),
                embeddings_before,
                embeddings_after: embeddings_before,
                replaced_count: 0,
            }));
        }
    };
    
    let mut captured = 0usize;
    let capture_interval_ms = config.enrollment.capture_interval_ms.unwrap_or(2000);
    let capture_interval = Duration::from_millis(capture_interval_ms);
    
    // Calculate dynamic timeout: num_captures * interval * 5 for overhead
    let enhancement_timeout = Duration::from_millis(
        additional_captures as u64 * capture_interval_ms * 5
    );
    let enhancement_start = Instant::now();
    let mut last_capture_time = Instant::now();
    
    tracing::info!("Enhancement timeout set to {:.1}s for {} captures with {:.1}s intervals",
                 enhancement_timeout.as_secs_f32(), additional_captures, capture_interval.as_secs_f32());
    
    // Find next image index for saving
    let mut next_image_idx = 0;
    while enrollment_dir.join(format!("enhance_{}.jpg", next_image_idx)).exists() {
        next_image_idx += 1;
    }
    
    while captured < additional_captures && enhancement_start.elapsed() < enhancement_timeout {
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
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to detect faces: {}", e);
                vec![]
            }
        };
        
        // Send ASCII preview frame
        let ascii = renderer.render_frame_with_progress(
            &frame,
            &faces,
            captured,
            additional_captures
        );
        
        if let Err(e) = send_stream_message(stream, &StreamMessage::PreviewFrame { 
            ascii,
            captured,
            total: additional_captures,
        }) {
            tracing::warn!("Failed to send preview frame: {}", e);
            // Continue even if preview fails
        }
        
        // Check if we have a face and enough time has passed
        if !faces.is_empty() && last_capture_time.elapsed() >= capture_interval {
            let face = &faces[0];
            
            // Calculate quality metrics
            let quality = QualityMetrics::calculate(&frame, face);
            
            // Check if quality meets requirements
            if quality.meets_minimum_requirements(min_quality) {
                // Get embedding
                let embedding = match recognizer.get_embedding(&frame, face) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!("Failed to get embedding: {}", e);
                        continue;
                    }
                };
                
                // Save enhancement image
                let image_path = enrollment_dir.join(format!("enhance_{}.jpg", next_image_idx + captured));
                if let Err(e) = frame.save(&image_path) {
                    tracing::warn!("Failed to save enhancement image: {}", e);
                }
                
                new_embeddings.push(embedding);
                new_quality_scores.push(quality.overall_score);
                captured += 1;
                last_capture_time = Instant::now();
                
                // Use debug level to avoid interfering with ASCII preview
                tracing::debug!("Captured enhancement image {}/{} with quality {:.2}", 
                             captured, additional_captures, quality.overall_score);
                
                // Send status update through the stream (not to stderr)
                if let Err(e) = send_stream_message(stream, &StreamMessage::StatusUpdate { 
                    message: format!("Captured image {}/{} with quality {:.2}", 
                                   captured, additional_captures, quality.overall_score),
                }) {
                    tracing::debug!("Failed to send status update: {}", e);
                }
            } else {
                tracing::debug!("Image quality too low: {:.2}", quality.overall_score);
            }
        }
        
        // Small delay to prevent CPU hogging
        std::thread::sleep(Duration::from_millis(50));
    }
    
    // Check if we captured enough for success
    let success = captured > 0;  // Enhancement can succeed with partial captures
    
    // Merge new embeddings with existing data
    let (added_count, replaced_count) = if !new_embeddings.is_empty() {
        store.merge_user_data(
            &mut user_data,
            new_embeddings,
            new_quality_scores.clone(),
            request.replace_weak
        )
    } else {
        (0, 0)
    };
    
    // Save updated user data if we have new embeddings
    if added_count > 0 {
        if let Err(e) = store.save_user_data(&user_data) {
            return Ok(Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to save enhanced enrollment data: {}", e),
                embeddings_before,
                embeddings_after: embeddings_before,
                replaced_count: 0,
            }));
        }
    }
    
    let embeddings_after = user_data.embeddings.len();
    
    // Send the enhancement report as final frame
    let report = format_enhancement_report(
        &request.username,
        captured,
        additional_captures,
        embeddings_before,
        embeddings_after,
        &new_quality_scores,
        replaced_count,
        success,
        config.enrollment.ascii_width.unwrap_or(60),
        config.enrollment.ascii_height.unwrap_or(25),
    );
    
    if let Err(e) = send_stream_message(stream, &StreamMessage::PreviewFrame {
        ascii: report,
        captured,
        total: additional_captures,
    }) {
        tracing::debug!("Failed to send enhancement report: {}", e);
    }
    
    // Create the response
    if success {
        tracing::info!("Successfully enhanced user: {} (before: {}, after: {}, replaced: {})", 
                     request.username, embeddings_before, embeddings_after, replaced_count);
        Ok(Response::Enhance(EnhanceResponse {
            success: true,
            message: format!(
                "Successfully enhanced enrollment for '{}'. Added {} embeddings{}",
                request.username,
                added_count,
                if replaced_count > 0 {
                    format!(", replaced {} weak embeddings", replaced_count)
                } else {
                    String::new()
                }
            ),
            embeddings_before,
            embeddings_after,
            replaced_count,
        }))
    } else {
        Ok(Response::Enhance(EnhanceResponse {
            success: false,
            message: "Failed to capture any valid face images for enhancement".to_string(),
            embeddings_before,
            embeddings_after: embeddings_before,
            replaced_count: 0,
        }))
    }
}

fn handle_enhance_request(
    detector: &FaceDetector,
    recognizer: &FaceRecognizer,
    request: EnhanceRequest,
    peer_cred: &PeerCredentials,
    config: &Config,
    data_dir: &Path,
) -> Response {
    use sup_linux::quality::QualityMetrics;
    
    // Authorization check: Users can only enhance themselves unless they're root
    if peer_cred.uid != 0 {
        let requesting_user = get_username_from_uid(peer_cred.uid);
        if let Ok(req_user) = requesting_user {
            if req_user != request.username {
                tracing::warn!("User {} (UID {}) attempted to enhance as {}", 
                    req_user, peer_cred.uid, request.username);
                return Response::Enhance(EnhanceResponse {
                    success: false,
                    message: format!("Permission denied: You can only enhance your own enrollment"),
                    embeddings_before: 0,
                    embeddings_after: 0,
                    replaced_count: 0,
                });
            }
        } else {
            tracing::warn!("Could not determine username for UID {}", peer_cred.uid);
            return Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to verify user identity"),
                embeddings_before: 0,
                embeddings_after: 0,
                replaced_count: 0,
            });
        }
    }
    
    tracing::info!("Starting enhancement for user: {} (requested by UID: {})", 
        request.username, peer_cred.uid);
    
    // Create user store
    let store = match UserStore::new_with_paths(
        data_dir.join("users"),
        data_dir.join("enrollment"),
    ) {
        Ok(s) => s,
        Err(e) => {
            return Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to initialize storage: {}", e),
                embeddings_before: 0,
                embeddings_after: 0,
                replaced_count: 0,
            });
        }
    };
    
    // Load existing user data
    let mut user_data = match store.get_user(&request.username) {
        Ok(data) => data,
        Err(_) => {
            return Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("User {} not found. Please enroll first.", request.username),
                embeddings_before: 0,
                embeddings_after: 0,
                replaced_count: 0,
            });
        }
    };
    
    let embeddings_before = user_data.embeddings.len();
    
    // Get enrollment images directory
    let enrollment_dir = match store.get_enrollment_images_dir(&request.username) {
        Ok(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return Response::Enhance(EnhanceResponse {
                    success: false,
                    message: format!("Failed to create enrollment directory: {}", e),
                    embeddings_before,
                    embeddings_after: embeddings_before,
                    replaced_count: 0,
                });
            }
            dir
        }
        Err(e) => {
            return Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to get enrollment directory: {}", e),
                embeddings_before,
                embeddings_after: embeddings_before,
                replaced_count: 0,
            });
        }
    };
    
    // Create camera just for this enhancement
    let mut camera = match Camera::new(config) {
        Ok(c) => c,
        Err(e) => {
            return Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to initialize camera: {}", e),
                embeddings_before,
                embeddings_after: embeddings_before,
                replaced_count: 0,
            });
        }
    };
    
    // Capture additional images
    let mut new_embeddings = Vec::new();
    let mut new_quality_scores = Vec::new();
    let additional_captures = request.additional_captures.unwrap_or(3);
    let min_quality = config.enrollment.min_enrollment_quality;
    
    tracing::info!("Capturing {} additional images for enhancement", additional_captures);
    
    // Start camera session
    let mut session = match camera.start_session() {
        Ok(s) => s,
        Err(e) => {
            return Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to start camera: {}", e),
                embeddings_before,
                embeddings_after: embeddings_before,
                replaced_count: 0,
            });
        }
    };
    
    let mut captured = 0;
    let capture_interval_ms = config.enrollment.capture_interval_ms.unwrap_or(2000);
    let capture_interval = Duration::from_millis(capture_interval_ms);
    
    // Calculate dynamic timeout: num_captures * interval * 3 for overhead
    let enhancement_timeout = Duration::from_millis(
        additional_captures as u64 * capture_interval_ms * 3
    );
    let enhancement_start = Instant::now();
    let mut last_capture_time = Instant::now();
    
    tracing::info!("Enhancement timeout set to {:.1}s for {} captures with {:.1}s intervals",
                 enhancement_timeout.as_secs_f32(), additional_captures, capture_interval.as_secs_f32());
    
    // Find next image index for saving
    let mut next_image_idx = 0;
    while enrollment_dir.join(format!("enhance_{}.jpg", next_image_idx)).exists() {
        next_image_idx += 1;
    }
    
    while captured < additional_captures && enhancement_start.elapsed() < enhancement_timeout {
        
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
        
        // Save enhancement image
        let image_path = enrollment_dir.join(format!("enhance_{}.jpg", next_image_idx + captured));
        if let Err(e) = frame.save(&image_path) {
            tracing::warn!("Failed to save enhancement image: {}", e);
        }
        
        new_embeddings.push(embedding);
        new_quality_scores.push(quality.overall_score);
        captured += 1;
        last_capture_time = Instant::now();
        
        tracing::info!("Captured enhancement image {}/{} with quality {:.2}", 
                     captured, additional_captures, quality.overall_score);
    }
    
    if new_embeddings.is_empty() {
        return Response::Enhance(EnhanceResponse {
            success: false,
            message: "Failed to capture any valid face images for enhancement".to_string(),
            embeddings_before,
            embeddings_after: embeddings_before,
            replaced_count: 0,
        });
    }
    
    // Merge new embeddings with existing data
    let (added_count, replaced_count) = store.merge_user_data(
        &mut user_data,
        new_embeddings,
        new_quality_scores,
        request.replace_weak
    );
    
    // Save updated user data
    match store.save_user_data(&user_data) {
        Ok(_) => {
            let embeddings_after = user_data.embeddings.len();
            tracing::info!("Successfully enhanced user: {} (before: {}, after: {}, replaced: {})", 
                         request.username, embeddings_before, embeddings_after, replaced_count);
            Response::Enhance(EnhanceResponse {
                success: true,
                message: format!(
                    "Successfully enhanced enrollment for '{}'. Added {} embeddings{}",
                    request.username,
                    added_count,
                    if replaced_count > 0 {
                        format!(", replaced {} weak embeddings", replaced_count)
                    } else {
                        String::new()
                    }
                ),
                embeddings_before,
                embeddings_after,
                replaced_count,
            })
        }
        Err(e) => {
            tracing::error!("Failed to save enhanced user data: {}", e);
            Response::Enhance(EnhanceResponse {
                success: false,
                message: format!("Failed to save enhanced enrollment data: {}", e),
                embeddings_before,
                embeddings_after: embeddings_before,
                replaced_count: 0,
            })
        }
    }
}