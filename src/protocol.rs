use serde::{Serialize, Deserialize};
use std::time::SystemTime;

// Request types
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
    Authenticate(AuthRequest),
    Enroll(EnrollRequest),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuthRequest {
    pub username: String,
    pub challenge: Vec<u8>,
    pub timestamp: SystemTime,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EnrollRequest {
    pub username: String,
}

// Response types
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Response {
    Auth(AuthResponse),
    Enroll(EnrollResponse),
    Error(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuthResponse {
    pub success: bool,
    pub message: String,
    pub attempts: u32,
    pub signature: Vec<u8>,
    pub timestamp: SystemTime,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EnrollResponse {
    pub success: bool,
    pub message: String,
}

// Socket path constant
pub const SOCKET_PATH: &str = "/run/linuxsup/embedding.sock";