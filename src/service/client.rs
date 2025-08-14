use crate::common::{FaceAuthError, Result};
use crate::service::protocol::{
    Request, Response, AuthRequest, EnrollRequest, EnhanceRequest,
    StreamMessage, MSG_TYPE_RESPONSE, MSG_TYPE_STREAM
};
use std::os::unix::net::UnixStream;
use std::io::{self, Read, Write};
use std::time::{Duration, SystemTime};
use std::path::Path;
use std::process::{Command, Stdio};
use rand::{Rng, thread_rng};
use crossterm::{terminal, cursor};

pub struct ServiceClient {
    socket_path: String,
    dev_mode: bool,
}

impl ServiceClient {
    pub fn new(dev_mode: bool) -> Self {
        let socket_path = if dev_mode {
            "/tmp/suplinux.sock".to_string()
        } else {
            "/run/suplinux/service.sock".to_string()
        };
        ServiceClient { socket_path, dev_mode }
    }
    
    pub fn enroll(&mut self, username: &str) -> Result<()> {
        // Ensure service is running
        self.ensure_service_running()?;
        
        // Connect to service
        let mut stream = self.connect_with_retry(3)?;
        
        // Create enrollment request with preview enabled
        let request = Request::Enroll(EnrollRequest {
            username: username.to_string(),
            enable_preview: true,  // Always enable preview for better UX
        });
        
        // Send request
        self.send_request(&mut stream, &request)?;
        
        // Handle streaming preview if enabled
        let response = self.read_enrollment_with_preview(&mut stream)?;
        
        match response {
            Response::Enroll(enroll_resp) => {
                if enroll_resp.success {
                    println!("✅ {}", enroll_resp.message);
                    Ok(())
                } else {
                    Err(FaceAuthError::Other(anyhow::anyhow!(enroll_resp.message)))
                }
            }
            Response::Error(msg) => {
                Err(FaceAuthError::Other(anyhow::anyhow!("Service error: {}", msg)))
            }
            _ => {
                Err(FaceAuthError::Other(anyhow::anyhow!("Unexpected response type")))
            }
        }
    }
    
    pub fn test_auth(&mut self, username: &str) -> Result<bool> {
        // Ensure service is running
        self.ensure_service_running()?;
        
        // Connect to service
        let mut stream = self.connect_with_retry(3)?;
        
        // Generate challenge
        let challenge = generate_challenge();
        
        // Create auth request
        let request = Request::Authenticate(AuthRequest {
            username: username.to_string(),
            challenge: challenge.clone(),
            timestamp: SystemTime::now(),
        });
        
        // Send request
        self.send_request(&mut stream, &request)?;
        
        // Read response
        let response = self.read_response(&mut stream)?;
        
        match response {
            Response::Auth(auth) => {
                println!("Authentication {} - {}", 
                    if auth.success { "succeeded" } else { "failed" },
                    auth.message);
                Ok(auth.success)
            }
            Response::Error(msg) => {
                eprintln!("Service error: {}", msg);
                Ok(false)
            }
            _ => {
                eprintln!("Unexpected response type");
                Ok(false)
            }
        }
    }
    
    pub fn enhance(&mut self, username: &str, additional_captures: Option<u32>, replace_weak: bool) -> Result<()> {
        // Ensure service is running
        self.ensure_service_running()?;
        
        // Connect to service
        let mut stream = self.connect_with_retry(3)?;
        
        // Create enhance request with preview enabled
        let request = Request::Enhance(EnhanceRequest {
            username: username.to_string(),
            additional_captures,
            replace_weak,
            enable_preview: true,  // Always enable preview for better UX
        });
        
        // Send request
        self.send_request(&mut stream, &request)?;
        
        // Handle streaming preview if enabled
        let response = self.read_enrollment_with_preview(&mut stream)?;
        
        match response {
            Response::Enhance(enhance_resp) => {
                if enhance_resp.success {
                    println!("✅ {}", enhance_resp.message);
                    println!("   Embeddings: {} → {} (replaced: {})", 
                             enhance_resp.embeddings_before, 
                             enhance_resp.embeddings_after,
                             enhance_resp.replaced_count);
                    Ok(())
                } else {
                    Err(FaceAuthError::Other(anyhow::anyhow!(enhance_resp.message)))
                }
            }
            Response::Error(msg) => {
                Err(FaceAuthError::Other(anyhow::anyhow!("Service error: {}", msg)))
            }
            _ => {
                Err(FaceAuthError::Other(anyhow::anyhow!("Unexpected response type")))
            }
        }
    }
    
    pub fn ensure_service_running(&self) -> Result<()> {
        // Check if socket exists
        if Path::new(&self.socket_path).exists() {
            // Try to connect to verify it's actually running
            if UnixStream::connect(&self.socket_path).is_ok() {
                return Ok(());
            }
        }
        
        // Only auto-start in dev mode
        if !self.dev_mode {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Service is not running. Please start it with: sudo systemctl start suplinux"
            )));
        }
        
        // Start service in dev mode
        println!("Starting service in development mode...");
        
        let service_binary = std::env::current_exe()?
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Failed to get binary directory"))?
            .join("suplinux-service");
        
        if !service_binary.exists() {
            return Err(FaceAuthError::Other(anyhow::anyhow!(
                "Service binary not found at {:?}. Please build the project first.", service_binary
            )));
        }
        
        // Spawn service in background
        Command::new(&service_binary)
            .arg("--dev")
            .arg("--dev-socket")
            .arg(&self.socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())  // Suppress stderr to avoid interfering with ASCII preview
            .spawn()
            .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to start service: {}", e)))?;
        
        // Wait for service to be ready (check socket)
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(500));
            if Path::new(&self.socket_path).exists() {
                println!("Service started successfully");
                return Ok(());
            }
        }
        
        Err(FaceAuthError::Other(anyhow::anyhow!("Service failed to start within timeout")))
    }
    
    fn connect_with_retry(&self, max_retries: u32) -> Result<UnixStream> {
        for attempt in 0..max_retries {
            match UnixStream::connect(&self.socket_path) {
                Ok(stream) => {
                    // Set timeout
                    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
                    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
                    return Ok(stream);
                }
                Err(e) if attempt < max_retries - 1 => {
                    eprintln!("Failed to connect (attempt {}): {}", attempt + 1, e);
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => {
                    return Err(FaceAuthError::Other(anyhow::anyhow!(
                        "Failed to connect to service: {}", e
                    )));
                }
            }
        }
        unreachable!()
    }
    
    fn send_request(&self, stream: &mut UnixStream, request: &Request) -> Result<()> {
        let request_data = bincode::serialize(request)
            .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to serialize request: {}", e)))?;
        let request_len = (request_data.len() as u32).to_le_bytes();
        
        stream.write_all(&request_len)?;
        stream.write_all(&request_data)?;
        stream.flush()?;
        
        Ok(())
    }
    
    fn read_response(&self, stream: &mut UnixStream) -> Result<Response> {
        // Read response length
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let response_len = u32::from_le_bytes(len_buf) as usize;
        
        if response_len > 1024 * 1024 {
            return Err(FaceAuthError::Other(anyhow::anyhow!("Response too large")));
        }
        
        // Read response
        let mut response_buf = vec![0u8; response_len];
        stream.read_exact(&mut response_buf)?;
        
        // Deserialize response
        let response: Response = bincode::deserialize(&response_buf)
            .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to deserialize response: {}", e)))?;
        
        Ok(response)
    }
    
    fn read_enrollment_with_preview(&self, stream: &mut UnixStream) -> Result<Response> {
        // Track the preview area
        let mut preview_height = 0;
        let mut first_frame = true;
        
        let result = (|| -> Result<Response> {
            loop {
                // Read message type indicator
                let mut type_buf = [0u8; 1];
                stream.read_exact(&mut type_buf)?;
                
                // Read message length
                let mut len_buf = [0u8; 4];
                stream.read_exact(&mut len_buf)?;
                let msg_len = u32::from_le_bytes(len_buf) as usize;
                
                if msg_len > 1024 * 1024 {
                    return Err(FaceAuthError::Other(anyhow::anyhow!("Message too large")));
                }
                
                // Read message data
                let mut msg_buf = vec![0u8; msg_len];
                stream.read_exact(&mut msg_buf)?;
                
                match type_buf[0] {
                    MSG_TYPE_STREAM => {
                        // Handle stream message
                        let stream_msg: StreamMessage = bincode::deserialize(&msg_buf)
                            .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to deserialize stream message: {}", e)))?;
                        
                        match stream_msg {
                            StreamMessage::PreviewFrame { ascii, captured: _, total: _ } => {
                                // Split ASCII into lines for proper handling
                                let lines: Vec<&str> = ascii.lines().collect();
                                let frame_height = lines.len();
                                
                                if first_frame {
                                    // First frame - just print it
                                    println!("\n📷 Starting enrollment - look at the camera:");
                                    
                                    // Print all lines, keeping track of cursor position
                                    for (i, line) in lines.iter().enumerate() {
                                        if i < lines.len() - 1 {
                                            println!("{}", line);
                                        } else {
                                            // Last line - use print! to stay on same line
                                            print!("{}", line);
                                            io::stdout().flush().ok();
                                        }
                                    }
                                    preview_height = frame_height;
                                    first_frame = false;
                                } else {
                                    // Move cursor back to the start of the first line of the preview
                                    // Since we used print! on the last line, cursor is at end of last line
                                    // We need to go up (height - 1) lines and then to start of line
                                    if preview_height > 0 {
                                        crossterm::execute!(
                                            io::stdout(),
                                            cursor::MoveUp((preview_height - 1) as u16),
                                            cursor::MoveToColumn(0)
                                        ).ok();
                                    }
                                    
                                    // Overwrite each line
                                    for (i, line) in lines.iter().enumerate() {
                                        // Clear the current line first
                                        crossterm::execute!(
                                            io::stdout(),
                                            terminal::Clear(terminal::ClearType::CurrentLine)
                                        ).ok();
                                        
                                        if i < lines.len() - 1 {
                                            println!("{}", line);
                                        } else {
                                            // Last line - use print! to stay on same line
                                            print!("{}", line);
                                            io::stdout().flush().ok();
                                        }
                                    }
                                    
                                    // If new frame is shorter, we need to clear the extra lines
                                    if frame_height < preview_height {
                                        // Move to next line after current frame
                                        println!();
                                        
                                        // Clear the extra lines
                                        for _ in frame_height..preview_height {
                                            crossterm::execute!(
                                                io::stdout(),
                                                terminal::Clear(terminal::ClearType::CurrentLine)
                                            ).ok();
                                            println!();
                                        }
                                        
                                        // Move back to end of actual frame
                                        crossterm::execute!(
                                            io::stdout(),
                                            cursor::MoveUp((preview_height - frame_height + 1) as u16)
                                        ).ok();
                                    }
                                    
                                    preview_height = frame_height;
                                }
                                
                                io::stdout().flush().ok();
                            }
                            StreamMessage::StatusUpdate { message: _ } => {
                                // Status updates appear below the preview
                                // Don't print during streaming to avoid disrupting the display
                                // These will be shown in the final response
                            }
                            StreamMessage::Complete => {
                                // Move cursor below preview for final message
                                println!("\n"); // Add spacing before final message
                                continue;
                            }
                        }
                    }
                    MSG_TYPE_RESPONSE => {
                        // Final response received
                        let response: Response = bincode::deserialize(&msg_buf)
                            .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to deserialize response: {}", e)))?;
                        return Ok(response);
                    }
                    _ => {
                        return Err(FaceAuthError::Other(anyhow::anyhow!("Unknown message type")));
                    }
                }
            }
        })();
        
        result
    }
}

fn generate_challenge() -> Vec<u8> {
    let mut rng = thread_rng();
    let mut challenge = vec![0u8; 32];
    rng.fill(&mut challenge[..]);
    challenge
}