use crate::common::{FaceAuthError, Result, Config};
use ort::{Environment, Session, SessionBuilder, Value, GraphOptimizationLevel};
use std::sync::Arc;
use image::{DynamicImage, imageops::FilterType};
use ndarray::{Array4, CowArray};

#[derive(Debug, Clone)]
pub struct FaceBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub confidence: f32,
}

pub struct FaceDetector {
    session: Session,
    _environment: Arc<Environment>,
    config: Config,
}

impl FaceDetector {
    #[allow(dead_code)]
    pub fn new_with_model_path(config: &Config, models_base: &std::path::Path) -> Result<Self> {
        let mut model_path = config.models.detector_path.clone();
        if model_path.is_relative() {
            model_path = models_base.join(&model_path);
        }
        
        let environment = Arc::new(
            Environment::builder()
                .with_name("face_detector")
                .build()
                .map_err(|e| FaceAuthError::Model(format!("Failed to create environment: {}", e)))?
        );
        
        if !model_path.exists() {
            return Err(FaceAuthError::Model(
                format!("Detector model not found at: {:?}", model_path)
            ));
        }
        
        let mut session_builder = SessionBuilder::new(&environment)?;
        
        // Apply optimization level from config
        let opt_level = match config.performance.optimization_level {
            0 => GraphOptimizationLevel::Disable,
            1 => GraphOptimizationLevel::Level1,
            2 => GraphOptimizationLevel::Level2,
            _ => GraphOptimizationLevel::Level3,
        };
        session_builder = session_builder.with_optimization_level(opt_level)?;
        
        let session = session_builder.with_model_from_file(model_path)?;
        
        Ok(Self {
            session,
            _environment: environment,
            config: config.clone(),
        })
    }
    
    pub fn new(config: &Config) -> Result<Self> {
        let environment = Arc::new(
            Environment::builder()
                .with_name("face_detector")
                .build()
                .map_err(|e| FaceAuthError::Model(format!("Failed to create environment: {}", e)))?
        );

        let model_path = &config.models.detector_path;
        if !model_path.exists() {
            return Err(FaceAuthError::Model(
                format!("Detector model not found at: {:?}", model_path)
            ));
        }

        let mut session_builder = SessionBuilder::new(&environment)?;
        
        // Apply optimization level from config
        let opt_level = match config.performance.optimization_level {
            0 => GraphOptimizationLevel::Disable,
            1 => GraphOptimizationLevel::Level1,
            2 => GraphOptimizationLevel::Level2,
            _ => GraphOptimizationLevel::Level3,
        };
        session_builder = session_builder.with_optimization_level(opt_level)?;
        
        // Note: INT8 quantization requires specific ONNX Runtime builds and providers
        // For now, we'll use the optimization level which provides good speedup
        
        let session = session_builder.with_model_from_file(model_path)?;

        Ok(Self {
            session,
            _environment: environment,
            config: config.clone(),
        })
    }

    pub fn detect(&self, image: &DynamicImage) -> Result<Vec<FaceBox>> {
        // Store original image dimensions for coordinate scaling
        let orig_width = image.width() as f32;
        let orig_height = image.height() as f32;
        
        // Process directly from grayscale if possible
        let img_array = if orig_width as u32 == self.config.detector.input_width 
            && orig_height as u32 == self.config.detector.input_height {
            // No resize needed - process directly
            self.image_to_array(image)?
        } else {
            // Resize needed
            let resized = image.resize_exact(
                self.config.detector.input_width, 
                self.config.detector.input_height, 
                FilterType::Nearest  // Fastest resize algorithm
            );
            self.image_to_array(&resized)?
        };

        let cow_array = CowArray::from(img_array.into_dyn());
        let input_tensor = Value::from_array(self.session.allocator(), &cow_array)?;
        let outputs = self.session.run(vec![input_tensor])?;

        let mut faces = self.parse_detections(&outputs)?;
        
        // Scale coordinates back to original image dimensions
        let scale_x = orig_width / self.config.detector.input_width as f32;
        let scale_y = orig_height / self.config.detector.input_height as f32;
        
        for face in &mut faces {
            face.x1 *= scale_x;
            face.x2 *= scale_x;
            face.y1 *= scale_y;
            face.y2 *= scale_y;
        }
        
        Ok(faces)
    }
    
    pub fn detect_debug(&self, image: &DynamicImage) -> Result<(Vec<FaceBox>, Vec<FaceBox>)> {
        // Store original image dimensions for coordinate scaling
        let orig_width = image.width() as f32;
        let orig_height = image.height() as f32;
        
        // Process directly from grayscale if possible
        let img_array = if orig_width as u32 == self.config.detector.input_width 
            && orig_height as u32 == self.config.detector.input_height {
            // No resize needed - process directly
            self.image_to_array(image)?
        } else {
            // Resize needed
            let resized = image.resize_exact(
                self.config.detector.input_width, 
                self.config.detector.input_height, 
                FilterType::Nearest  // Fastest resize algorithm
            );
            self.image_to_array(&resized)?
        };

        let cow_array = CowArray::from(img_array.into_dyn());
        let input_tensor = Value::from_array(self.session.allocator(), &cow_array)?;
        let outputs = self.session.run(vec![input_tensor])?;

        // Get all detections and filtered detections
        let (mut all_faces, mut filtered_faces) = self.parse_detections_debug(&outputs)?;
        
        // Scale coordinates back to original image dimensions
        let scale_x = orig_width / self.config.detector.input_width as f32;
        let scale_y = orig_height / self.config.detector.input_height as f32;
        
        for face in &mut all_faces {
            face.x1 *= scale_x;
            face.x2 *= scale_x;
            face.y1 *= scale_y;
            face.y2 *= scale_y;
        }
        
        for face in &mut filtered_faces {
            face.x1 *= scale_x;
            face.x2 *= scale_x;
            face.y1 *= scale_y;
            face.y2 *= scale_y;
        }
        
        Ok((all_faces, filtered_faces))
    }

    fn image_to_array(&self, img: &DynamicImage) -> Result<Array4<f32>> {
        // Optimized for YOLOv8 with NIR images
        let gray = match img {
            DynamicImage::ImageLuma8(gray) => gray.as_raw(),
            _ => {
                // Only convert if not already grayscale
                let converted = img.to_luma8();
                return self.image_to_array(&DynamicImage::ImageLuma8(converted));
            }
        };
        
        let width = img.width() as usize;
        let height = img.height() as usize;
        let mut array = Array4::<f32>::zeros((1, 3, height, width));

        // Vectorized normalization and channel replication
        let norm_factor = 1.0 / 255.0;
        
        // Process in chunks for better cache locality
        for y in 0..height {
            let row_offset = y * width;
            for x in 0..width {
                let idx = row_offset + x;
                let pixel_value = gray[idx] as f32 * norm_factor;
                
                // Set all 3 channels at once
                array[[0, 0, y, x]] = pixel_value;
                array[[0, 1, y, x]] = pixel_value;
                array[[0, 2, y, x]] = pixel_value;
            }
        }

        Ok(array)
    }

    fn parse_detections(&self, outputs: &Vec<Value>) -> Result<Vec<FaceBox>> {
        let mut faces = Vec::new();

        // YOLOv8 output format: [1, 8400, num_classes + 4] OR [1, num_classes + 4, 8400] (transposed)
        // Where each detection is [x_center, y_center, width, height, class_scores...]
        if outputs.len() >= 1 {
            let output = outputs[0].try_extract::<f32>()?.view().to_owned();
            let output_array = output.as_slice().unwrap();
            
            // Get dimensions
            let shape = output.shape();
            // tracing::debug!("YOLOv8 output shape: {:?}", shape);
            
            // Check if output is transposed
            let (num_predictions, prediction_length, is_transposed) = if shape.len() >= 3 {
                if shape[2] > shape[1] && shape[1] <= 10 {
                    // Likely transposed format [1, 5, 8400]
                    // Detected transposed output format
                    (shape[2], shape[1], true)
                } else {
                    // Standard format [1, 8400, 5]
                    (shape[1], shape[2], false)
                }
            } else if shape.len() == 2 {
                // Handle 2D output [8400, 5]
                (shape[0], shape[1], false)
            } else {
                tracing::warn!("Unexpected output shape: {:?}", shape);
                return Ok(faces);
            };
            
            // Processing predictions
            
            // Only log first few predictions for debugging
            let _debug_limit = 5.min(num_predictions);
            
            for i in 0..num_predictions {
                // Calculate index based on whether output is transposed
                let (x_center_raw, y_center_raw, width_raw, height_raw, confidence) = if is_transposed {
                    // Transposed format: [1, 5, 8400]
                    let base_idx = i;
                    (
                        output_array[base_idx],                    // x_center at [0, i]
                        output_array[8400 + base_idx],            // y_center at [1, i]
                        output_array[2 * 8400 + base_idx],       // width at [2, i]
                        output_array[3 * 8400 + base_idx],       // height at [3, i]
                        if prediction_length > 4 { 
                            output_array[4 * 8400 + base_idx]    // confidence at [4, i]
                        } else { 0.0 }
                    )
                } else {
                    // Standard format: [1, 8400, 5]
                    let base_idx = i * prediction_length;
                    (
                        output_array[base_idx],
                        output_array[base_idx + 1],
                        output_array[base_idx + 2],
                        output_array[base_idx + 3],
                        if prediction_length > 4 { output_array[base_idx + 4] } else { 0.0 }
                    )
                };
                
                // Check if coordinates are already in pixel space or normalized
                let scale_factor = if x_center_raw > 1.0 || y_center_raw > 1.0 || width_raw > 1.0 || height_raw > 1.0 {
                    // Already in pixel coordinates
                    1.0
                } else {
                    // Normalized coordinates, need to scale
                    self.config.detector.input_width as f32
                };
                
                let x_center = x_center_raw * scale_factor;
                let y_center = y_center_raw * scale_factor;
                let width = width_raw * scale_factor;
                let height = height_raw * scale_factor;
                
                // Skip debug logging
                
                // Apply minimal early filtering - just skip zero confidence
                if confidence > 0.001 {  // Very low threshold to catch all real detections
                    // Convert from center coordinates to corner coordinates
                    let x1 = (x_center - width / 2.0).max(0.0);
                    let y1 = (y_center - height / 2.0).max(0.0);
                    let x2 = (x_center + width / 2.0).min(self.config.detector.input_width as f32);
                    let y2 = (y_center + height / 2.0).min(self.config.detector.input_height as f32);
                    
                    // Skip invalid boxes (too small or inverted)
                    if x2 > x1 && y2 > y1 && (x2 - x1) > 10.0 && (y2 - y1) > 10.0 {
                        faces.push(FaceBox {
                            x1,
                            y1,
                            x2,
                            y2,
                            confidence,
                        });
                    }
                }
            }
        }

        // Pre-filtered boxes
        
        // Apply NMS FIRST on all boxes with low confidence threshold
        // This removes duplicates before we filter by the actual confidence threshold
        faces = self.apply_nms(faces, 0.45);
        // Applied NMS
        
        // THEN filter by the actual detection confidence threshold
        faces.retain(|face| face.confidence >= self.config.auth.detection_confidence);
        // Filtered by confidence
        
        // Sort by confidence and limit results
        faces.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        faces.truncate(5);

        Ok(faces)
    }
    
    fn apply_nms(&self, mut boxes: Vec<FaceBox>, iou_threshold: f32) -> Vec<FaceBox> {
        if boxes.is_empty() {
            return boxes;
        }
        
        // NMS processing
        
        // Sort by confidence
        boxes.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        
        let mut keep = Vec::new();
        let mut indices: Vec<usize> = (0..boxes.len()).collect();
        
        while !indices.is_empty() {
            let i = indices[0];
            keep.push(boxes[i].clone());
            
            let remaining_before = indices.len();
            
            indices = indices[1..].iter()
                .filter(|&&j| {
                    let iou = self.calculate_iou(&boxes[i], &boxes[j]);
                    let keep_box = iou < iou_threshold;
                        // Box overlap check
                    keep_box
                })
                .copied()
                .collect();
                
            let _removed = remaining_before - indices.len() - 1;
            // NMS iteration complete
        }
        
        // NMS complete
        keep
    }
    
    fn calculate_iou(&self, box1: &FaceBox, box2: &FaceBox) -> f32 {
        let x1 = box1.x1.max(box2.x1);
        let y1 = box1.y1.max(box2.y1);
        let x2 = box1.x2.min(box2.x2);
        let y2 = box1.y2.min(box2.y2);
        
        let intersection = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
        let area1 = (box1.x2 - box1.x1) * (box1.y2 - box1.y1);
        let area2 = (box2.x2 - box2.x1) * (box2.y2 - box2.y1);
        let union = area1 + area2 - intersection;
        
        if union > 0.0 {
            intersection / union
        } else {
            0.0
        }
    }
    
    fn parse_detections_debug(&self, outputs: &Vec<Value>) -> Result<(Vec<FaceBox>, Vec<FaceBox>)> {
        let mut all_faces = Vec::new();

        // YOLOv8 output format: [1, 8400, num_classes + 4]
        if outputs.len() >= 1 {
            let output = outputs[0].try_extract::<f32>()?.view().to_owned();
            let output_array = output.as_slice().unwrap();
            
            // Get dimensions
            let shape = output.shape();
            // tracing::debug!("YOLOv8 output shape: {:?}", shape);
            
            // Use same parsing logic as main function
            let (num_predictions, prediction_length, is_transposed) = if shape.len() >= 3 {
                if shape[2] > shape[1] && shape[1] <= 10 {
                    (shape[2], shape[1], true)
                } else {
                    (shape[1], shape[2], false)
                }
            } else if shape.len() == 2 {
                (shape[0], shape[1], false)
            } else {
                return Ok((all_faces, vec![]));
            };
            
            // Debug: Processing predictions
            
            for i in 0..num_predictions {
                // Calculate index based on whether output is transposed
                let (x_center_raw, y_center_raw, width_raw, height_raw, confidence) = if is_transposed {
                    let base_idx = i;
                    let stride = num_predictions;
                    (
                        output_array[base_idx],
                        output_array[stride + base_idx],
                        output_array[2 * stride + base_idx],
                        output_array[3 * stride + base_idx],
                        if prediction_length > 4 { output_array[4 * stride + base_idx] } else { 0.0 }
                    )
                } else {
                    let base_idx = i * prediction_length;
                    (
                        output_array[base_idx],
                        output_array[base_idx + 1],
                        output_array[base_idx + 2],
                        output_array[base_idx + 3],
                        if prediction_length > 4 { output_array[base_idx + 4] } else { 0.0 }
                    )
                };
                
                // Check if coordinates are already in pixel space or normalized
                let scale_factor = if x_center_raw > 1.0 || y_center_raw > 1.0 || width_raw > 1.0 || height_raw > 1.0 {
                    1.0
                } else {
                    self.config.detector.input_width as f32
                };
                
                let x_center = x_center_raw * scale_factor;
                let y_center = y_center_raw * scale_factor;
                let width = width_raw * scale_factor;
                let height = height_raw * scale_factor;
                
                // Convert from center coordinates to corner coordinates
                let x1 = (x_center - width / 2.0).max(0.0);
                let y1 = (y_center - height / 2.0).max(0.0);
                let x2 = (x_center + width / 2.0).min(self.config.detector.input_width as f32);
                let y2 = (y_center + height / 2.0).min(self.config.detector.input_height as f32);
                
                let face_box = FaceBox {
                    x1,
                    y1,
                    x2,
                    y2,
                    confidence,
                };
                
                // Add all boxes for debugging
                all_faces.push(face_box);
            }
        }

        // Sort by confidence
        all_faces.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        
        // Get filtered faces above threshold
        let mut filtered_faces: Vec<FaceBox> = all_faces
            .iter()
            .filter(|f| f.confidence > self.config.auth.detection_confidence)
            .cloned()
            .collect();
        
        // Apply NMS to filtered faces
        filtered_faces = self.apply_nms(filtered_faces, 0.5);
        
        // Limit for debug display
        all_faces.truncate(20);
        filtered_faces.truncate(5);

        Ok((all_faces, filtered_faces))
    }
}