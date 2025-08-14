use crate::{
    camera::Camera,
    common::{Config, DevMode, Result},
    core::{
        detector::{FaceDetector, FaceBox},
        recognizer::{FaceRecognizer, cosine_similarity, Embedding},
    },
    storage::UserStore,
};
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use image::{DynamicImage, Rgb};
use imageproc::drawing::draw_hollow_rect_mut;
use imageproc::rect::Rect;

pub struct FaceAuth {
    camera: Camera,
    detector: FaceDetector,
    recognizer: FaceRecognizer,
    store: UserStore,
    config: Config,
    _dev_mode: DevMode,
}

impl FaceAuth {
    pub fn new_with_dev_mode(dev_mode: DevMode) -> Result<Self> {
        let config = Config::load()?;

        Ok(Self {
            camera: Camera::new(&config)?,
            detector: FaceDetector::new(&config)?,
            recognizer: FaceRecognizer::new(&config)?,
            store: UserStore::new_with_dev_mode(&dev_mode)?,
            config,
            _dev_mode: dev_mode,
        })
    }

    pub fn authenticate(&mut self, username: &str) -> Result<bool> {
        let user_data = self.store.get_user(username)?;
        let start_time = Instant::now();
        let timeout = Duration::from_secs(self.config.auth.timeout_seconds as u64);
        let lost_face_timeout = Duration::from_secs(self.config.auth.lost_face_timeout as u64);

        let mut last_face_time = Instant::now();
        let mut face_detected_at_least_once = false;
        let mut attempts = 0;
        
        // K-of-N tracking
        let mut auth_attempts = VecDeque::with_capacity(self.config.auth.n_total_attempts as usize);
        let mut successful_matches = 0u32;
        
        // Rolling embedding buffer for fusion
        let mut embedding_buffer = VecDeque::with_capacity(self.config.auth.embedding_buffer_size as usize);

        println!("Look at the camera...");

        // Start a camera session for efficient streaming (includes warmup)
        let mut session = self.camera.start_session()?;

        while start_time.elapsed() < timeout {
            attempts += 1;
            let loop_start = Instant::now();
            
            // Check if we've lost the face for too long
            if face_detected_at_least_once && last_face_time.elapsed() > lost_face_timeout {
                println!("Face lost - resetting authentication progress");
                // Reset K-of-N tracking
                auth_attempts.clear();
                successful_matches = 0;
                embedding_buffer.clear();
                face_detected_at_least_once = false;
            }

            let capture_start = Instant::now();
            let frame = session.capture_frame()?;
            let capture_time = capture_start.elapsed();

            let detect_start = Instant::now();
            match self.detector.detect(&frame) {
                Ok(faces) if !faces.is_empty() => {
                    if !face_detected_at_least_once {
                        println!("Face detected! Verifying...");
                    }
                    face_detected_at_least_once = true;
                    last_face_time = Instant::now();

                    let face = &faces[0];
                    let embedding = self.recognizer.get_embedding(&frame, face)?;
                    
                    // Add to embedding buffer
                    embedding_buffer.push_back(embedding.clone());
                    if embedding_buffer.len() > self.config.auth.embedding_buffer_size as usize {
                        embedding_buffer.pop_front();
                    }
                    
                    // Calculate best similarity
                    let mut best_similarity = 0.0f32;
                    
                    // Check individual embedding
                    for stored_embedding in user_data.embeddings.iter() {
                        let similarity = cosine_similarity(&embedding, stored_embedding);
                        best_similarity = best_similarity.max(similarity);
                    }
                    
                    // Check against averaged stored embedding if available
                    if let Some(ref avg_stored) = user_data.averaged_embedding {
                        let similarity = cosine_similarity(&embedding, avg_stored);
                        best_similarity = best_similarity.max(similarity);
                    }
                    
                    // Check fused embedding if enabled and we have enough samples
                    if self.config.auth.use_embedding_fusion && embedding_buffer.len() >= 2 {
                        let fused_embedding = average_embeddings_buffer(&embedding_buffer);
                        
                        for stored_embedding in user_data.embeddings.iter() {
                            let similarity = cosine_similarity(&fused_embedding, stored_embedding);
                            best_similarity = best_similarity.max(similarity);
                        }
                        
                        if let Some(ref avg_stored) = user_data.averaged_embedding {
                            let similarity = cosine_similarity(&fused_embedding, avg_stored);
                            best_similarity = best_similarity.max(similarity);
                        }
                    }
                    
                    // Update K-of-N tracking
                    let auth_success = best_similarity > self.config.auth.similarity_threshold;
                    auth_attempts.push_back(auth_success);
                    
                    if auth_success {
                        successful_matches += 1;
                    }
                    
                    // Keep only last N attempts
                    while auth_attempts.len() > self.config.auth.n_total_attempts as usize {
                        if auth_attempts.pop_front() == Some(true) {
                            successful_matches -= 1;
                        }
                    }
                    
                    // Show progress
                    println!("Authentication attempt: similarity {:.3} {} ({}/{} matches)", 
                             best_similarity,
                             if auth_success { "✓" } else { "✗" },
                             successful_matches,
                             self.config.auth.k_required_matches);
                    
                    // Check if we have K successes in last N attempts
                    if successful_matches >= self.config.auth.k_required_matches {
                        println!("✓ Authentication successful! ({} total attempts)", attempts);
                        return Ok(true);
                    }
                },
                Ok(_) => {},  // No face detected, continue silently
                Err(e) => eprintln!("Detection error: {}", e),
            }
            
            let detect_time = detect_start.elapsed();
            let loop_time = loop_start.elapsed();
            
            // Log timing every 10 attempts
            if attempts % 10 == 0 || attempts == 1 {
                println!("Attempt {} timing: capture={:.1}ms, detect={:.1}ms, total={:.1}ms", 
                         attempts, 
                         capture_time.as_secs_f32() * 1000.0,
                         detect_time.as_secs_f32() * 1000.0,
                         loop_time.as_secs_f32() * 1000.0);
            }
        }

        println!("✗ Authentication timeout after {} attempts", attempts);
        Ok(false)
    }

    // Enrollment methods removed - all enrollment goes through the service
    // This ensures dev and production modes work identically
}

// Dev mode versions of public functions
pub fn test_camera_dev(dev_mode: &DevMode) -> Result<()> {
    let config = Config::load()?;
    let mut camera = Camera::new(&config)?;
    let img = camera.capture_frame()?;
    
    let save_path = if dev_mode.is_enabled() {
        dev_mode.get_capture_path("test_capture")
    } else {
        std::path::PathBuf::from("test_capture.jpg")
    };
    
    img.save(&save_path)?;
    println!("Saved test image to {:?}", save_path);
    Ok(())
}

fn visualize_detections(image: &DynamicImage, all_faces: &[FaceBox], filtered_faces: &[FaceBox]) -> DynamicImage {
    let mut img = image.to_rgb8();
    
    // Define colors for different confidence levels
    let high_conf_color = Rgb([0, 255, 0]);    // Green
    let med_conf_color = Rgb([255, 255, 0]);   // Yellow
    let low_conf_color = Rgb([255, 0, 0]);     // Red
    let filtered_color = Rgb([0, 255, 255]);   // Cyan for filtered faces
    
    // Draw all detections first (in confidence-based colors)
    for (_idx, face) in all_faces.iter().enumerate() {
        // Skip invalid boxes
        let width = face.x2 - face.x1;
        let height = face.y2 - face.y1;
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        
        let color = if face.confidence > 0.7 {
            high_conf_color
        } else if face.confidence > 0.5 {
            med_conf_color
        } else {
            low_conf_color
        };
        
        // Ensure coordinates are within image bounds
        let x1 = face.x1.max(0.0) as i32;
        let y1 = face.y1.max(0.0) as i32;
        let x2 = face.x2.min(img.width() as f32) as i32;
        let y2 = face.y2.min(img.height() as f32) as i32;
        
        let rect_width = (x2 - x1).max(1) as u32;
        let rect_height = (y2 - y1).max(1) as u32;
        
        let rect = Rect::at(x1, y1).of_size(rect_width, rect_height);
        
        draw_hollow_rect_mut(&mut img, rect, color);
        
        // Draw a thicker border by drawing multiple rectangles (poor man's thick border)
        if face.confidence > 0.5 && rect_width > 2 && rect_height > 2 {
            let inner_rect = Rect::at(x1 + 1, y1 + 1)
                .of_size(rect_width - 2, rect_height - 2);
            draw_hollow_rect_mut(&mut img, inner_rect, color);
        }
    }
    
    // Highlight filtered faces with thicker cyan border
    for face in filtered_faces {
        // Skip invalid boxes
        let width = face.x2 - face.x1;
        let height = face.y2 - face.y1;
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        
        // Ensure coordinates are within image bounds
        let x1 = (face.x1 - 2.0).max(0.0) as i32;
        let y1 = (face.y1 - 2.0).max(0.0) as i32;
        let x2 = (face.x2 + 2.0).min(img.width() as f32) as i32;
        let y2 = (face.y2 + 2.0).min(img.height() as f32) as i32;
        
        let rect_width = (x2 - x1).max(1) as u32;
        let rect_height = (y2 - y1).max(1) as u32;
        
        let rect = Rect::at(x1, y1).of_size(rect_width, rect_height);
        draw_hollow_rect_mut(&mut img, rect, filtered_color);
    }
    
    DynamicImage::ImageRgb8(img)
}

pub fn test_detection_dev(dev_mode: &DevMode) -> Result<()> {
    let config = Config::load()?;
    let mut camera = Camera::new(&config)?;
    let detector = FaceDetector::new(&config)?;

    println!("Capturing frame from camera {}...", config.camera.device_index);
    let frame = camera.capture_frame()?;
    
    let save_path = if dev_mode.is_enabled() {
        dev_mode.get_capture_path("detection_test")
    } else {
        std::path::PathBuf::from("detection_test.jpg")
    };
    
    frame.save(&save_path)?;
    println!("Saved original image to {:?}", save_path);

    println!("Detecting faces...");
    
    // Get both all detections and filtered detections
    let (all_faces, filtered_faces) = detector.detect_debug(&frame)?;
    
    println!("Found {} face(s) above threshold {}", filtered_faces.len(), config.auth.detection_confidence);
    
    // Only show details if we have a reasonable number of detections
    if all_faces.len() <= 10 {
        for (i, face) in filtered_faces.iter().enumerate() {
            println!("  Face {}: confidence {:.3}", i + 1, face.confidence);
        }
    }
    
    // Create visualization with bounding boxes
    let annotated_image = visualize_detections(&frame, &all_faces, &filtered_faces);
    
    // Save annotated image
    let debug_path = if dev_mode.is_enabled() {
        dev_mode.get_debug_path("detection_annotated")
    } else {
        std::path::PathBuf::from("detection_annotated.jpg")
    };
    
    annotated_image.save(&debug_path)?;
    println!("\nSaved annotated image to: {:?}", debug_path);

    Ok(())
}

pub fn enroll_user_dev(username: &str, dev_mode: &DevMode) -> Result<()> {
    use crate::service::ServiceClient;
    
    // Always use the service now (unified path)
    let mut client = ServiceClient::new(dev_mode.is_enabled());
    client.enroll(username)
}

// Removed enroll_via_service - now using ServiceClient for both dev and production

pub fn authenticate_user_dev(username: &str, dev_mode: &DevMode) -> Result<bool> {
    use crate::service::ServiceClient;
    
    // Always use the service now (unified path)
    let mut client = ServiceClient::new(dev_mode.is_enabled());
    client.test_auth(username)
}

pub fn enhance_user_dev(username: &str, additional_captures: u32, replace_weak: bool, dev_mode: &DevMode) -> Result<()> {
    use crate::service::ServiceClient;
    
    // Always use the service now (unified path)
    let mut client = ServiceClient::new(dev_mode.is_enabled());
    client.enhance(username, Some(additional_captures), replace_weak)
}

// Helper function to average embeddings - used by authentication
#[allow(dead_code)]
fn average_embeddings(embeddings: &[Embedding]) -> Embedding {
    if embeddings.is_empty() {
        return vec![];
    }
    
    let embedding_size = embeddings[0].len();
    let mut averaged = vec![0.0f32; embedding_size];
    
    for embedding in embeddings {
        for (i, &value) in embedding.iter().enumerate() {
            averaged[i] += value;
        }
    }
    
    let count = embeddings.len() as f32;
    for value in &mut averaged {
        *value /= count;
    }
    
    averaged
}

// Helper function to average embeddings from a buffer
fn average_embeddings_buffer(buffer: &VecDeque<Embedding>) -> Embedding {
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