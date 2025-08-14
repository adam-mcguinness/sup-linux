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
    pub enable_preview: bool,  // Enable ASCII preview during enrollment
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EnhanceRequest {
    pub username: String,
    pub additional_captures: Option<u32>,
    pub replace_weak: bool,
    pub enable_preview: bool,  // Enable ASCII preview during enhancement
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

// Streaming messages for real-time updates
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum StreamMessage {
    PreviewFrame {
        ascii: String,      // ASCII art representation of camera frame
        captured: usize,    // Number of images captured so far
        total: usize,       // Total images to capture
    },
    StatusUpdate {
        message: String,    // Status message to display
    },
    Complete,              // Enrollment/enhancement complete, final response follows
}

// Message type indicators for stream protocol
pub const MSG_TYPE_RESPONSE: u8 = 0;  // Final response
pub const MSG_TYPE_STREAM: u8 = 1;    // Stream update

// Socket path constant
pub const SOCKET_PATH: &str = "/run/suplinux/service.sock";