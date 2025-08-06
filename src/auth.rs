use crate::{
    ascii_preview::{AsciiRenderer, clear_screen, check_for_escape, show_capture_flash},
    camera::Camera,
    config::Config,
    detector::{FaceDetector, FaceBox},
    dev_mode::DevMode,
    error::{FaceAuthError, Result},
    quality::{QualityMetrics, calculate_embedding_consistency},
    recognizer::{FaceRecognizer, cosine_similarity, Embedding},
    storage::{UserStore, UserData},
};
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::io::{self, Write};
use image::{DynamicImage, Rgb};
use imageproc::drawing::draw_hollow_rect_mut;
use imageproc::rect::Rect;
use crossterm::terminal;

pub struct FaceAuth {
    camera: Camera,
    detector: FaceDetector,
    recognizer: FaceRecognizer,
    store: UserStore,
    config: Config,
    dev_mode: DevMode,
}

impl FaceAuth {
    pub fn new() -> Result<Self> {
        let config = Config::load()?;
        let dev_mode = DevMode::new(false)?;

        Ok(Self {
            camera: Camera::new(&config)?,
            detector: FaceDetector::new(&config)?,
            recognizer: FaceRecognizer::new(&config)?,
            store: UserStore::new()?,
            config,
            dev_mode,
        })
    }
    
    pub fn new_with_dev_mode(dev_mode: DevMode) -> Result<Self> {
        let config = Config::load()?;

        Ok(Self {
            camera: Camera::new(&config)?,
            detector: FaceDetector::new(&config)?,
            recognizer: FaceRecognizer::new(&config)?,
            store: UserStore::new_with_dev_mode(&dev_mode)?,
            config,
            dev_mode,
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

    pub fn enroll(&mut self, username: &str) -> Result<()> {
        println!("Starting enrollment for '{}'", username);
        println!("We'll capture 3 high-quality images for better recognition");

        let mut embeddings = Vec::new();
        let mut quality_scores = Vec::new();
        let min_quality = self.config.enrollment.min_enrollment_quality;

        // Create enrollment images directory for this user
        let enrollment_dir = self.store.get_enrollment_images_dir(username)?;
        std::fs::create_dir_all(&enrollment_dir)?;

        let mut captured = 0;
        let mut attempts = 0;
        
        while captured < 3 && attempts < 10 {
            attempts += 1;
            println!("\nCapture {} of 3 - look directly at the camera...", captured + 1);
            std::thread::sleep(Duration::from_secs(1));

            let frame = self.camera.capture_frame()?;
            
            let faces = self.detector.detect(&frame)?;
            
            if faces.is_empty() {
                println!("⚠ No face detected, please position your face in view");
                continue;
            }
            
            // Calculate quality metrics
            let quality = QualityMetrics::calculate(&frame, &faces[0]);
            println!("  {}", quality.get_quality_assessment());
            
            // Check if quality meets requirements
            if !quality.meets_minimum_requirements(min_quality) {
                println!("⚠ Image quality too low. Suggestions:");
                for suggestion in quality.get_improvement_suggestions() {
                    println!("  - {}", suggestion);
                }
                continue;
            }

            // Get embedding
            let embedding = self.recognizer.get_embedding(&frame, &faces[0])?;
            
            // Save enrollment image
            let image_path = enrollment_dir.join(format!("enroll_{}.jpg", captured));
            frame.save(&image_path)?;
            println!("  ✓ Captured! Saved to: {:?}", image_path);
            
            embeddings.push(embedding);
            quality_scores.push(quality.overall_score);
            captured += 1;
        }

        if embeddings.is_empty() {
            return Err(FaceAuthError::NoFaceDetected);
        }
        
        // Check embedding consistency
        let consistency = calculate_embedding_consistency(&embeddings);
        println!("\nEmbedding consistency: {:.2}", consistency);
        
        if consistency < 0.7 {
            println!("⚠ Warning: Low consistency between captures. Consider re-enrolling in better conditions.");
        }

        // Calculate averaged embedding if enabled
        let averaged_embedding = if self.config.enrollment.store_averaged_embedding {
            Some(average_embeddings(&embeddings))
        } else {
            None
        };
        
        let user_data = UserData {
            version: 1,
            username: username.to_string(),
            embeddings,
            averaged_embedding,
            embedding_qualities: Some(quality_scores),
        };

        self.store.save_user_data(&user_data)?;
        println!("\n✓ User '{}' enrolled successfully with {} high-quality face capture(s)!", 
                 username, user_data.embeddings.len());

        Ok(())
    }

    pub fn enroll_with_preview(&mut self, username: &str) -> Result<()> {
        
        println!("Starting enhanced enrollment for '{}'", username);
        println!("We'll capture multiple images as you move your head for better recognition");
        
        // Create enrollment images directory for this user
        let enrollment_dir = self.store.get_enrollment_images_dir(username)?;
        std::fs::create_dir_all(&enrollment_dir)?;
        
        let total_captures = self.config.enrollment.num_captures.unwrap_or(5);
        let capture_interval = Duration::from_millis(
            self.config.enrollment.capture_interval_ms.unwrap_or(2000)
        );
        let min_quality = self.config.enrollment.min_enrollment_quality;
        
        let mut embeddings = Vec::new();
        let mut quality_scores = Vec::new();
        let mut captured = 0;
        let mut last_capture_time = Instant::now();
        
        // Setup terminal for ASCII preview
        terminal::enable_raw_mode()
            .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to enable raw mode: {}", e)))?;
        
        // Create ASCII renderer
        let renderer = AsciiRenderer::new(
            self.config.enrollment.ascii_width,
            self.config.enrollment.ascii_height
        );
        
        // Start camera session for streaming
        let mut session = self.camera.start_session()?;
        
        let result = (|| -> Result<()> {
            // Clear screen once at the start
            clear_screen().ok();
            
            loop {
                // Capture frame from existing session
                let frame = session.capture_frame()?;
                let detected_faces = self.detector.detect(&frame)?;
                
                // Render ASCII preview
                let ascii = renderer.render_frame_with_progress(
                    &frame,
                    &detected_faces,
                    captured,
                    total_captures
                );
                
                // Just move cursor to home and overwrite - no clear
                crossterm::execute!(
                    io::stdout(),
                    crossterm::cursor::MoveTo(0, 0),
                    crossterm::style::Print(&ascii),
                    crossterm::cursor::MoveTo(0, (renderer.height() + 2) as u16),
                    crossterm::style::Print("Press ESC to cancel enrollment                    ")  // Extra spaces to clear any leftover text
                ).ok();
                
                // Check for ESC key
                if check_for_escape()
                    .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to check input: {}", e)))? {
                    return Err(FaceAuthError::Other(anyhow::anyhow!("Enrollment cancelled by user")));
                }
                
                // Auto-capture logic
                if !detected_faces.is_empty() {
                    let face = &detected_faces[0];
                    
                    // Check if enough time has passed since last capture
                    if last_capture_time.elapsed() > capture_interval {
                        // Calculate quality metrics
                        let quality = QualityMetrics::calculate(&frame, face);
                        
                        // Check if quality meets requirements
                        if quality.meets_minimum_requirements(min_quality) {
                            // Get embedding
                            let embedding = self.recognizer.get_embedding(&frame, face)?;
                            
                            // Save enrollment image
                            let image_path = enrollment_dir.join(format!("enroll_{}.jpg", captured));
                            frame.save(&image_path)?;
                            
                            embeddings.push(embedding);
                            quality_scores.push(quality.overall_score);
                            captured += 1;
                            last_capture_time = Instant::now();
                            
                            // Show capture flash
                            show_capture_flash();
                        }
                    }
                }
                
                // Check if we've captured enough images
                if captured >= total_captures {
                    break;
                }
                
                // Small delay to prevent CPU spinning
                std::thread::sleep(Duration::from_millis(50));
            }
            
            // Check embedding consistency
            let consistency = calculate_embedding_consistency(&embeddings);
            println!("\n\nEmbedding consistency: {:.2}", consistency);
            
            if consistency < 0.7 {
                println!("⚠ Warning: Low consistency between captures. Consider re-enrolling in better conditions.");
            }
            
            // Calculate averaged embedding if enabled
            let averaged_embedding = if self.config.enrollment.store_averaged_embedding {
                Some(average_embeddings(&embeddings))
            } else {
                None
            };
            
            let user_data = UserData {
                version: 1,
                username: username.to_string(),
                embeddings,
                averaged_embedding,
                embedding_qualities: Some(quality_scores),
            };
            
            self.store.save_user_data(&user_data)?;
            println!("\n✓ User '{}' enrolled successfully with {} high-quality face captures!", 
                     username, user_data.embeddings.len());
            
            Ok(())
        })();
        
        // Clear screen before restoring terminal
        clear_screen().ok();
        
        // Restore terminal
        terminal::disable_raw_mode()
            .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to disable raw mode: {}", e)))?;
        
        result
    }
}

// Public functions for CLI
pub fn test_camera() -> Result<()> {
    let config = Config::load()?;
    let mut camera = Camera::new(&config)?;
    let img = camera.capture_frame()?;
    img.save("test_capture.jpg")?;
    println!("Saved test image to test_capture.jpg");
    Ok(())
}

pub fn test_detection() -> Result<()> {
    let config = Config::load()?;
    let mut camera = Camera::new(&config)?;
    let detector = FaceDetector::new(&config)?;

    println!("Capturing frame from camera {}...", config.camera.device_index);
    let frame = camera.capture_frame()?;
    frame.save("detection_test.jpg")?;

    println!("Detecting faces...");
    let faces = detector.detect(&frame)?;

    println!("Found {} face(s)", faces.len());
    for (i, face) in faces.iter().enumerate() {
        println!("Face {}: ({:.0}, {:.0}) to ({:.0}, {:.0}) confidence: {:.2}",
                 i, face.x1, face.y1, face.x2, face.y2, face.confidence);
    }

    Ok(())
}

pub fn enroll_user(username: &str) -> Result<()> {
    let mut auth = FaceAuth::new()?;
    let config = Config::load()?;
    
    // Use ASCII preview enrollment if enabled
    if config.enrollment.enable_ascii_preview.unwrap_or(true) {
        auth.enroll_with_preview(username)
    } else {
        auth.enroll(username)
    }
}

pub fn authenticate_user(username: &str) -> Result<bool> {
    let mut auth = FaceAuth::new()?;
    auth.authenticate(username)
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
    let mut auth = FaceAuth::new_with_dev_mode(dev_mode.clone())?;
    let config = Config::load()?;
    
    // Use ASCII preview enrollment if enabled
    if config.enrollment.enable_ascii_preview.unwrap_or(true) {
        auth.enroll_with_preview(username)
    } else {
        auth.enroll(username)
    }
}

pub fn authenticate_user_dev(username: &str, dev_mode: &DevMode) -> Result<bool> {
    let mut auth = FaceAuth::new_with_dev_mode(dev_mode.clone())?;
    auth.authenticate(username)
}

pub fn authenticate_user_system(username: &str, paths: &crate::paths::Paths) -> Result<bool> {
    let config = Config::load_from_path(&paths.config_file())?;
    
    // Try system store first, then user store
    let system_store = UserStore::new_with_paths(
        PathBuf::from("/var/lib/linuxsup/users"),
        PathBuf::from("/var/lib/linuxsup/enrollment"),
    )?;
    
    // Check if user exists in system store
    let store = if system_store.get_user(username).is_ok() {
        system_store
    } else {
        // Try user's home directory
        if let Some(home) = dirs::home_dir() {
            let user_store = UserStore::new_with_paths(
                home.join(".local/share/linuxsup/users"),
                home.join(".local/share/linuxsup/enrollment"),
            )?;
            
            // Check if user exists in user store
            if user_store.get_user(username).is_ok() {
                user_store
            } else {
                // Return system store (will fail with UserNotFound)
                system_store
            }
        } else {
            system_store
        }
    };
    
    let camera = Camera::new(&config)?;
    let detector = FaceDetector::new_with_model_path(&config, &paths.models_dir())?;
    let recognizer = FaceRecognizer::new_with_model_path(&config, &paths.models_dir())?;
    
    let mut auth = FaceAuth {
        camera,
        detector,
        recognizer,
        store,
        config,
        dev_mode: DevMode::new(false)?,
    };
    
    auth.authenticate(username)
}

// Helper function to average embeddings
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