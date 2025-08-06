use crate::{
    ascii_preview::{AsciiRenderer, clear_screen, check_for_escape},
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
            // Setup screen once at the start
            clear_screen().ok();
            
            // Hide cursor for cleaner display
            crossterm::execute!(
                io::stdout(),
                crossterm::cursor::Hide
            ).ok();
            
            // Keep track of last valid frame and faces
            let mut last_valid_frame = None;
            let mut last_faces = Vec::new();
            
            loop {
                // Capture new frame
                let frame = session.capture_frame()?;
                
                // Check if frame is too dark (likely invalid)
                let is_valid_frame = {
                    let gray = frame.to_luma8();
                    let (width, height) = gray.dimensions();
                    let mut total_brightness: u64 = 0;
                    let mut pixel_count = 0;
                    
                    // Sample pixels in a grid pattern for efficiency
                    let step = 20; // Sample every 20th pixel for speed
                    for y in (0..height).step_by(step) {
                        for x in (0..width).step_by(step) {
                            total_brightness += gray.get_pixel(x, y)[0] as u64;
                            pixel_count += 1;
                        }
                    }
                    
                    pixel_count > 0 && (total_brightness / pixel_count) > 15
                };
                
                // Skip dark/invalid frames entirely
                if !is_valid_frame && last_valid_frame.is_some() {
                    continue;
                }
                
                // Store valid frame immediately
                if is_valid_frame {
                    last_valid_frame = Some(frame.clone());
                }
                
                // Process face detection
                let detected_faces = self.detector.detect(&frame)?;
                
                // Update faces if we found any
                if !detected_faces.is_empty() {
                    last_faces = detected_faces.clone();
                }
                
                // Render the frame
                let ascii = renderer.render_frame_with_progress(
                    &frame,
                    &last_faces,  // Use last known faces if current detection failed
                    captured,
                    total_captures
                );
                
                // Update display
                crossterm::execute!(
                    io::stdout(),
                    crossterm::cursor::MoveTo(0, 0),
                    crossterm::style::Print(&ascii),
                    crossterm::cursor::MoveTo(0, (renderer.height() + 2) as u16),
                    crossterm::style::Print("Press ESC to cancel enrollment                    ")
                ).ok();
                
                // Check for ESC key
                if check_for_escape()
                    .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to check input: {}", e)))? {
                    return Err(FaceAuthError::Other(anyhow::anyhow!("Enrollment cancelled by user")));
                }
                
                // Auto-capture logic
                if !detected_faces.is_empty() && is_valid_frame {
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
                        }
                    }
                }
                
                // Check if we've captured enough images
                if captured >= total_captures {
                    break;
                }
            }
            
            // Clear the ASCII display before showing results
            clear_screen().ok();
            
            // Build the entire metrics display as a single string with \r\n
            let mut output = String::new();
            
            output.push_str("\r\n╔══════════════════════════════════════════════════════╗\r\n");
            output.push_str("║           ENROLLMENT COMPLETE - RESULTS              ║\r\n");
            output.push_str("╚══════════════════════════════════════════════════════╝\r\n\r\n");
            
            output.push_str(&format!("User: {}\r\n", username));
            output.push_str(&format!("Captures completed: {}/{}\r\n\r\n", captured, total_captures));
            
            // Display quality scores for each capture
            if !quality_scores.is_empty() {
                output.push_str("📊 Quality Scores per Capture:\r\n");
                let mut total_quality = 0.0;
                for (i, score) in quality_scores.iter().enumerate() {
                    let bar_length = (score * 20.0) as usize;
                    let bar = "█".repeat(bar_length);
                    let empty = "░".repeat(20_usize.saturating_sub(bar_length));
                    output.push_str(&format!("  Capture {}: [{}{}] {:.2}%\r\n", 
                             i + 1, bar, empty, score * 100.0));
                    total_quality += score;
                }
                
                let avg_quality = total_quality / quality_scores.len() as f32;
                output.push_str(&format!("\r\n📈 Average Quality Score: {:.1}%\r\n", avg_quality * 100.0));
                
                // Quality assessment
                let quality_rating = if avg_quality >= 0.8 {
                    "Excellent ⭐⭐⭐⭐⭐"
                } else if avg_quality >= 0.7 {
                    "Good ⭐⭐⭐⭐"
                } else if avg_quality >= 0.6 {
                    "Acceptable ⭐⭐⭐"
                } else {
                    "Poor ⭐⭐"
                };
                output.push_str(&format!("   Rating: {}\r\n", quality_rating));
            }
            
            // Check embedding diversity/robustness
            let consistency = calculate_embedding_consistency(&embeddings);
            output.push_str(&format!("\r\n🔄 Enrollment Robustness: {:.1}%\r\n", consistency * 100.0));
            
            let consistency_rating = if consistency >= 0.85 {
                "Excellent - Optimal variation for robust recognition"
            } else if consistency >= 0.75 {
                "Good - Good balance of consistency and variation"
            } else if consistency >= 0.65 {
                "Acceptable - Adequate for recognition"
            } else if consistency < 0.5 {
                "Too similar - Try moving head more between captures"
            } else {
                "Too different - Keep movements smaller"
            };
            output.push_str(&format!("   {}\r\n", consistency_rating));
            
            if consistency < 0.65 {
                output.push_str("\r\n⚠️  Warning: Enrollment could be more robust.\r\n");
                output.push_str("   Suggestions:\r\n");
                if consistency < 0.5 {
                    output.push_str("   • Move your head slightly between captures\r\n");
                    output.push_str("   • Try different subtle angles\r\n");
                    output.push_str("   • Vary your expression slightly\r\n");
                } else {
                    output.push_str("   • Keep movements more subtle\r\n");
                    output.push_str("   • Maintain consistent lighting\r\n");
                    output.push_str("   • Keep the same general expression\r\n");
                }
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
            
            output.push_str(&format!("\r\n✅ User '{}' enrolled successfully!\r\n", username));
            output.push_str(&format!("   • {} high-quality face captures saved\r\n", user_data.embeddings.len()));
            output.push_str("   • Images saved in enrollment directory\r\n");
            
            output.push_str("\r\n══════════════════════════════════════════════════════\r\n");
            
            // Print everything at once
            print!("{}", output);
            io::stdout().flush().ok();
            
            Ok(())
        })();
        
        // Restore cursor before restoring terminal
        crossterm::execute!(
            io::stdout(),
            crossterm::cursor::Show
        ).ok();
        
        // Restore terminal
        terminal::disable_raw_mode()
            .map_err(|e| FaceAuthError::Other(anyhow::anyhow!("Failed to disable raw mode: {}", e)))?;
        
        result
    }
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
    use crate::service_client::ServiceClient;
    
    // Always use the service now (unified path)
    let mut client = ServiceClient::new(dev_mode.is_enabled());
    client.enroll(username)
}

// Removed enroll_via_service - now using ServiceClient for both dev and production

pub fn authenticate_user_dev(username: &str, dev_mode: &DevMode) -> Result<bool> {
    use crate::service_client::ServiceClient;
    
    // Always use the service now (unified path)
    let mut client = ServiceClient::new(dev_mode.is_enabled());
    client.test_auth(username)
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