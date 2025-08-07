use serde::{Serialize, Deserialize};
use std::time::SystemTime;

// Request types
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
    Authenticate(AuthRequest),
    Enroll(EnrollRequest),
    Enhance(EnhanceRequest),
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EnhanceRequest {
    pub username: String,
    pub additional_captures: Option<u32>,
    pub replace_weak: bool,
}

// Response types
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Response {
    Auth(AuthResponse),
    Enroll(EnrollResponse),
    Enhance(EnhanceResponse),
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EnhanceResponse {
    pub success: bool,
    pub message: String,
    pub embeddings_before: usize,
    pub embeddings_after: usize,
    pub replaced_count: usize,
}

// Socket path constant
pub const SOCKET_PATH: &str = "/run/suplinux/service.sock";