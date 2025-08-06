use crate::error::{FaceAuthError, Result};
use crate::protocol::{Request, Response, AuthRequest, EnrollRequest};
use std::os::unix::net::UnixStream;
use std::io::{Read, Write};
use std::time::{Duration, SystemTime};
use std::path::Path;
use std::process::{Command, Stdio};
use rand::{Rng, thread_rng};

pub struct ServiceClient {
    socket_path: String,
    dev_mode: bool,
}

impl ServiceClient {
    pub fn new(dev_mode: bool) -> Self {
        let socket_path = if dev_mode {
            "/tmp/linuxsup.sock".to_string()
        } else {
            "/run/linuxsup/embedding.sock".to_string()
        };
        ServiceClient { socket_path, dev_mode }
    }
    
    pub fn enroll(&mut self, username: &str) -> Result<()> {
        // Ensure service is running
        self.ensure_service_running()?;
        
        // Connect to service
        let mut stream = self.connect_with_retry(3)?;
        
        // Create enrollment request
        let request = Request::Enroll(EnrollRequest {
            username: username.to_string(),
        });
        
        // Send request
        self.send_request(&mut stream, &request)?;
        
        // Read response
        let response = self.read_response(&mut stream)?;
        
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
                "Embedding service is not running. Please start it with: sudo systemctl start linuxsup-embedding"
            )));
        }
        
        // Start service in dev mode
        println!("Starting embedding service in development mode...");
        
        let service_binary = std::env::current_exe()?
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Failed to get binary directory"))?
            .join("linuxsup-embedding-service");
        
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
            .stderr(Stdio::inherit())
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
                Ok(mut stream) => {
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
}

fn generate_challenge() -> Vec<u8> {
    let mut rng = thread_rng();
    let mut challenge = vec![0u8; 32];
    rng.fill(&mut challenge[..]);
    challenge
}