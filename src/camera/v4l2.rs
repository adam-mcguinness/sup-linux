use crate::common::{FaceAuthError, Result, Config};
use v4l::buffer::Type;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;
use v4l::{Device, FourCC};
use image::{DynamicImage, ImageBuffer, Luma};
use std::fs;

pub struct Camera {
    device: Device,
    config: Config,
}

// Helper to work around lifetime issues
pub struct CameraSession<'a> {
    camera: &'a mut Camera,
    stream: v4l::io::mmap::Stream<'a>,
    format: v4l::Format,
}

impl Camera {
    pub fn new(config: &Config) -> Result<Self> {
        let device_index = if config.camera.device_index == 999 {
            // Special value 999 means auto-detect
            Self::detect_ir_camera()?
        } else {
            config.camera.device_index
        };
        Self::new_with_device(device_index, config.clone())
    }
    
    /// List all available cameras with their capabilities
    pub fn list_all_cameras() -> Result<Vec<(u32, String, Vec<String>, bool)>> {
        let mut cameras = Vec::new();
        
        // Scan /dev/video* devices
        for entry in fs::read_dir("/dev")? {
            let entry = entry?;
            let path = entry.path();
            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
                
            if filename.starts_with("video") {
                if let Some(index_str) = filename.strip_prefix("video") {
                    if let Ok(index) = index_str.parse::<u32>() {
                        // Try to open the device
                        if let Ok(device) = Device::new(index as usize) {
                            if let Ok(caps) = device.query_caps() {
                                let mut features = Vec::new();
                                let mut likely_ir = false;
                                
                                // Check capabilities
                                if caps.capabilities.contains(v4l::capability::Flags::VIDEO_CAPTURE) {
                                    features.push("VIDEO_CAPTURE".to_string());
                                } else if caps.capabilities.contains(v4l::capability::Flags::META_CAPTURE) {
                                    features.push("METADATA_CAPTURE (may work for IR)".to_string());
                                }
                                
                                // Check supported formats
                                let formats = device.enum_formats().unwrap_or_default();
                                
                                for fmt in &formats {
                                    let fourcc_str = fmt.fourcc.str().unwrap_or("UNKNOWN");
                                    if fourcc_str == "GREY" || fourcc_str == "Y8" || fourcc_str == "Y16" {
                                        features.push(format!("Grayscale ({})", fourcc_str));
                                        likely_ir = true;
                                    } else if fourcc_str == "MJPG" || fourcc_str == "YUYV" {
                                        features.push(format!("Color ({})", fourcc_str));
                                    }
                                }
                                
                                // Check if name suggests IR
                                if caps.card.contains("BRIO") || caps.card.contains("IR") || caps.card.contains("Infrared") {
                                    likely_ir = true;
                                }
                                
                                cameras.push((index, caps.card.clone(), features, likely_ir));
                            }
                        }
                    }
                }
            }
        }
        
        // Sort by index
        cameras.sort_by_key(|c| c.0);
        Ok(cameras)
    }
    
    /// Auto-detect IR camera by looking for devices with grayscale format
    pub fn detect_ir_camera() -> Result<u32> {
        println!("Auto-detecting IR camera...");
        
        let mut candidates = Vec::new();
        
        // Scan /dev/video* devices
        for entry in fs::read_dir("/dev")? {
            let entry = entry?;
            let path = entry.path();
            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
                
            if filename.starts_with("video") {
                if let Some(index_str) = filename.strip_prefix("video") {
                    if let Ok(index) = index_str.parse::<u32>() {
                        // Try to open the device
                        if let Ok(device) = Device::new(index as usize) {
                            if let Ok(caps) = device.query_caps() {
                                // Check if it's a video capture device
                                // Some devices (like BRIO IR) report as metadata but have video capture capability
                                let has_video_cap = caps.capabilities.contains(v4l::capability::Flags::VIDEO_CAPTURE);
                                
                                if has_video_cap {
                                    // Check supported formats
                                    let formats = device.enum_formats()
                                        .unwrap_or_default();
                                    
                                    // Look for grayscale formats (typical for IR cameras)
                                    let has_grayscale = formats.iter().any(|fmt| {
                                        let fourcc_bytes = fmt.fourcc.repr;
                                        fourcc_bytes == *b"GREY" || 
                                        fourcc_bytes == *b"Y8  " ||
                                        fourcc_bytes == *b"Y16 "
                                    });
                                    
                                    if has_grayscale {
                                        println!("Found grayscale camera at /dev/video{}: {}", 
                                                index, caps.card);
                                        candidates.push((index, caps.card.clone(), 100)); // High priority
                                    } else if caps.card.contains("BRIO") || caps.card.contains("IR") {
                                        println!("Found potential IR camera at /dev/video{}: {}", 
                                                index, caps.card);
                                        candidates.push((index, caps.card.clone(), 50)); // Medium priority
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Sort by priority (highest first)
        candidates.sort_by(|a, b| b.2.cmp(&a.2));
        
        if let Some((index, name, _)) = candidates.first() {
            println!("Selected camera: /dev/video{} ({})", index, name);
            Ok(*index)
        } else {
            // No IR camera found, fall back to default camera
            println!("No IR camera detected, falling back to default camera (device 0)");
            println!("For better accuracy, consider using an IR camera or specifying device_index in config");
            Ok(0)
        }
    }

    pub fn new_with_device(index: u32, config: Config) -> Result<Self> {
        println!("Opening camera device {}...", index);
        
        let device = Device::new(index as usize)
            .map_err(|e| FaceAuthError::Camera(format!("Failed to open camera {}: {}", index, e)))?;

        // Check device capabilities
        let caps = device.query_caps()
            .map_err(|e| FaceAuthError::Camera(format!("Failed to query capabilities: {}", e)))?;
        
        println!("Device capabilities: {:?}", caps.capabilities);
        
        // Check if device supports video capture
        // Some devices like BRIO IR report as metadata but have video capture capability
        if !caps.capabilities.contains(v4l::capability::Flags::VIDEO_CAPTURE) {
            // Special case: Some devices report only metadata but still work for video
            // We'll warn but continue
            println!("Warning: Device {} may not support standard video capture", index);
            println!("Device capabilities: {:?}", caps.capabilities);
        }

        println!("Camera opened successfully. Getting current format...");
        
        let mut fmt = device.format()
            .map_err(|e| FaceAuthError::Camera(format!("Failed to get format: {}", e)))?;

        println!("Current format: {}x{} {}", fmt.width, fmt.height, fmt.fourcc.str().unwrap());
        
        // Try to set desired resolution
        fmt.width = config.camera.width;
        fmt.height = config.camera.height;

        // Keep GREY format for IR camera, otherwise use MJPG
        if fmt.fourcc.str().unwrap() != "GREY" {
            fmt.fourcc = FourCC::new(b"MJPG");
        }

        println!("Attempting to set format: {}x{} {}", fmt.width, fmt.height, fmt.fourcc.str().unwrap());
        
        // Try to set format, but don't fail if exact resolution isn't supported
        match device.set_format(&fmt) {
            Ok(_) => println!("Format set successfully"),
            Err(e) => println!("Warning: Could not set exact format: {}. Using device defaults.", e),
        }

        // Get the actual format that was set
        let final_fmt = device.format()
            .map_err(|e| FaceAuthError::Camera(format!("Failed to get final format: {}", e)))?;
        
        println!("Actual format: {}x{} {}", final_fmt.width, final_fmt.height, final_fmt.fourcc.str().unwrap());
        
        // Warn if resolution differs significantly from requested
        if final_fmt.width != config.camera.width || final_fmt.height != config.camera.height {
            println!("WARNING: Camera resolution {}x{} differs from requested {}x{}", 
                     final_fmt.width, final_fmt.height, 
                     config.camera.width, config.camera.height);
        }

        Ok(Self { device, config })
    }

    pub fn capture_frame(&mut self) -> Result<DynamicImage> {
        self.capture_frame_with_warmup(self.config.camera.warmup_frames)
    }

    pub fn capture_frame_with_warmup(&mut self, warmup_frames: u32) -> Result<DynamicImage> {
        let fmt = self.device.format()
            .map_err(|e| FaceAuthError::Camera(format!("Failed to get format: {}", e)))?;

        let mut stream = v4l::io::mmap::Stream::with_buffers(&mut self.device, Type::VideoCapture, 4)
            .map_err(|e| FaceAuthError::Camera(format!("Failed to create stream: {}", e)))?;

        // Warmup frames for IR emitter
        for _ in 0..warmup_frames {
            let (_buf, _meta) = stream.next()
                .map_err(|e| FaceAuthError::Camera(format!("Failed to capture warmup frame: {}", e)))?;
            std::thread::sleep(std::time::Duration::from_millis(self.config.camera.warmup_delay_ms));
        }

        let (buf, _meta) = stream.next()
            .map_err(|e| FaceAuthError::Camera(format!("Failed to capture: {}", e)))?;

        match fmt.fourcc.str().unwrap() {
            "GREY" => self.grey_to_image(&buf, fmt.width, fmt.height),
            _ => Err(FaceAuthError::Camera("Unsupported format".into())),
        }
    }
    
    // Start a streaming session for multiple captures
    pub fn start_session(&mut self) -> Result<CameraSession> {
        let fmt = self.device.format()
            .map_err(|e| FaceAuthError::Camera(format!("Failed to get format: {}", e)))?;
            
        let mut stream = v4l::io::mmap::Stream::with_buffers(&mut self.device, Type::VideoCapture, 8)
            .map_err(|e| FaceAuthError::Camera(format!("Failed to create stream: {}", e)))?;
            
        // Do warmup frames here when starting the session
        println!("Warming up camera...");
        for i in 0..self.config.camera.warmup_frames {
            let (_buf, _meta) = stream.next()
                .map_err(|e| FaceAuthError::Camera(format!("Failed to capture warmup frame {}: {}", i, e)))?;
            std::thread::sleep(std::time::Duration::from_millis(self.config.camera.warmup_delay_ms));
        }
        println!("Camera ready");
            
        Ok(CameraSession {
            camera: self,
            stream,
            format: fmt,
        })
    }

    fn grey_to_image(&self, data: &[u8], width: u32, height: u32) -> Result<DynamicImage> {
        let img_buffer = ImageBuffer::<Luma<u8>, _>::from_raw(width, height, data.to_vec())
            .ok_or_else(|| FaceAuthError::Camera("Failed to create grayscale image buffer".into()))?;

        Ok(DynamicImage::ImageLuma8(img_buffer))
    }
}

#[allow(dead_code)]
impl<'a> CameraSession<'a> {
    pub fn capture_frame(&mut self) -> Result<DynamicImage> {
        let (buf, _meta) = self.stream.next()
            .map_err(|e| FaceAuthError::Camera(format!("Failed to capture: {}", e)))?;

        match self.format.fourcc.str().unwrap() {
            "GREY" => self.camera.grey_to_image(&buf, self.format.width, self.format.height),
            _ => Err(FaceAuthError::Camera("Unsupported format".into())),
        }
    }
    
    pub fn capture_frame_with_warmup(&mut self, warmup_frames: u32) -> Result<DynamicImage> {
        // Warmup frames for IR emitter
        for _ in 0..warmup_frames {
            let (_buf, _meta) = self.stream.next()
                .map_err(|e| FaceAuthError::Camera(format!("Failed to capture warmup frame: {}", e)))?;
            std::thread::sleep(std::time::Duration::from_millis(self.camera.config.camera.warmup_delay_ms));
        }
        
        self.capture_frame()
    }
}