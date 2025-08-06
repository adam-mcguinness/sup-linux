use crate::detector::FaceBox;
use crate::recognizer::Embedding;
use image::DynamicImage;

#[derive(Debug, Clone)]
pub struct QualityMetrics {
    pub detection_confidence: f32,
    pub face_size_ratio: f32,
    pub face_centering_score: f32,
    pub brightness_score: f32,
    pub contrast_score: f32,
    pub overall_score: f32,
}

impl QualityMetrics {
    /// Calculate quality metrics for a face detection
    pub fn calculate(image: &DynamicImage, face: &FaceBox) -> Self {
        let detection_confidence = face.confidence;
        
        // Calculate face size ratio (how much of the image the face occupies)
        let img_width = image.width() as f32;
        let img_height = image.height() as f32;
        let face_width = face.x2 - face.x1;
        let face_height = face.y2 - face.y1;
        let face_area = face_width * face_height;
        let image_area = img_width * img_height;
        let face_size_ratio = (face_area / image_area).min(1.0);
        
        // Calculate face centering score (how centered the face is)
        let face_center_x = (face.x1 + face.x2) / 2.0;
        let face_center_y = (face.y1 + face.y2) / 2.0;
        let img_center_x = img_width / 2.0;
        let img_center_y = img_height / 2.0;
        
        let x_offset = ((face_center_x - img_center_x).abs() / img_center_x).min(1.0);
        let y_offset = ((face_center_y - img_center_y).abs() / img_center_y).min(1.0);
        let face_centering_score = 1.0 - (x_offset + y_offset) / 2.0;
        
        // Calculate brightness and contrast for the face region
        let (brightness_score, contrast_score) = calculate_image_quality(image, face);
        
        // Calculate overall score (weighted average)
        let overall_score = detection_confidence * 0.3
            + face_size_ratio * 0.2
            + face_centering_score * 0.2
            + brightness_score * 0.15
            + contrast_score * 0.15;
        
        QualityMetrics {
            detection_confidence,
            face_size_ratio,
            face_centering_score,
            brightness_score,
            contrast_score,
            overall_score,
        }
    }
    
    /// Check if the quality meets minimum requirements
    pub fn meets_minimum_requirements(&self, min_quality: f32) -> bool {
        self.overall_score >= min_quality
    }
    
    /// Get a human-readable quality assessment
    pub fn get_quality_assessment(&self) -> String {
        let quality_level = if self.overall_score >= 0.8 {
            "Excellent"
        } else if self.overall_score >= 0.7 {
            "Good"
        } else if self.overall_score >= 0.6 {
            "Acceptable"
        } else if self.overall_score >= 0.5 {
            "Poor"
        } else {
            "Very Poor"
        };
        
        format!("Quality: {} (score: {:.2})", quality_level, self.overall_score)
    }
    
    /// Get detailed feedback for improvement
    pub fn get_improvement_suggestions(&self) -> Vec<String> {
        let mut suggestions = Vec::new();
        
        if self.detection_confidence < 0.7 {
            suggestions.push("Move closer to the camera for better face detection".to_string());
        }
        
        if self.face_size_ratio < 0.1 {
            suggestions.push("Face is too small - move closer to the camera".to_string());
        } else if self.face_size_ratio > 0.5 {
            suggestions.push("Face is too large - move back from the camera".to_string());
        }
        
        if self.face_centering_score < 0.7 {
            suggestions.push("Center your face in the camera view".to_string());
        }
        
        if self.brightness_score < 0.5 {
            suggestions.push("Increase lighting - the image is too dark".to_string());
        } else if self.brightness_score > 0.9 {
            suggestions.push("Reduce lighting - the image is too bright".to_string());
        }
        
        if self.contrast_score < 0.5 {
            suggestions.push("Improve lighting conditions for better contrast".to_string());
        }
        
        suggestions
    }
}

/// Calculate embedding diversity score for robustness
/// For real-world applications, we want controlled variation (not too similar, not too different)
pub fn calculate_embedding_consistency(embeddings: &[Embedding]) -> f32 {
    if embeddings.len() < 2 {
        return 0.8; // Default score for single embedding
    }
    
    let mut similarities = Vec::new();
    
    // Calculate pairwise similarities
    for i in 0..embeddings.len() {
        for j in i+1..embeddings.len() {
            similarities.push(cosine_similarity(&embeddings[i], &embeddings[j]));
        }
    }
    
    if similarities.is_empty() {
        return 0.8;
    }
    
    // Calculate average similarity
    let avg_similarity = similarities.iter().sum::<f32>() / similarities.len() as f32;
    
    // Calculate variance to measure diversity
    let variance = similarities.iter()
        .map(|s| (s - avg_similarity).powi(2))
        .sum::<f32>() / similarities.len() as f32;
    
    // Ideal range: 0.75-0.90 similarity (some variation but still the same person)
    // Penalize both too high similarity (no variation) and too low (too different)
    let ideal_similarity = 0.82;
    let ideal_variance = 0.005; // Small but present variation
    
    // Score based on how close we are to ideal values
    let similarity_score = 1.0 - (avg_similarity - ideal_similarity).abs() * 2.0;
    let variance_score = if variance < 0.001 {
        0.7 // Too similar, no variation
    } else if variance > 0.02 {
        0.7 // Too different
    } else {
        1.0 - (variance - ideal_variance).abs() * 10.0
    };
    
    // Combine scores (weighted average)
    let combined_score = (similarity_score * 0.7 + variance_score * 0.3).max(0.0).min(1.0);
    
    // Return the score (higher is better for robust enrollment)
    combined_score
}

// Helper function to calculate brightness and contrast scores
fn calculate_image_quality(image: &DynamicImage, face: &FaceBox) -> (f32, f32) {
    let gray = image.to_luma8();
    
    // Ensure face bounds are within image
    let x1 = face.x1.max(0.0) as u32;
    let y1 = face.y1.max(0.0) as u32;
    let x2 = face.x2.min(gray.width() as f32) as u32;
    let y2 = face.y2.min(gray.height() as f32) as u32;
    
    if x2 <= x1 || y2 <= y1 {
        return (0.5, 0.5); // Default values if face box is invalid
    }
    
    let mut sum = 0u64;
    let mut sum_sq = 0u64;
    let mut count = 0u32;
    
    // Calculate mean and variance for the face region
    for y in y1..y2 {
        for x in x1..x2 {
            let pixel = gray.get_pixel(x, y)[0] as u64;
            sum += pixel;
            sum_sq += pixel * pixel;
            count += 1;
        }
    }
    
    if count == 0 {
        return (0.5, 0.5);
    }
    
    let mean = sum as f32 / count as f32;
    let variance = (sum_sq as f32 / count as f32) - (mean * mean);
    let std_dev = variance.sqrt();
    
    // Normalize brightness score (ideal mean around 127.5 for 8-bit images)
    let brightness_score = 1.0 - ((mean - 127.5).abs() / 127.5).min(1.0);
    
    // Normalize contrast score (higher std dev = better contrast, up to a point)
    let contrast_score = (std_dev / 64.0).min(1.0); // 64 is a reasonable std dev for good contrast
    
    (brightness_score, contrast_score)
}

fn cosine_similarity(a: &Embedding, b: &Embedding) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    
    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    
    dot_product / (norm_a * norm_b)
}