use crate::error::Result;
use crate::storage::UserStore;
use crate::recognizer::Embedding;
use crate::dev_mode::DevMode;
use std::fs;
use std::path::PathBuf;

pub struct Visualizer {
    output_dir: PathBuf,
}

impl Visualizer {
    pub fn new(dev_mode: &DevMode) -> Result<Self> {
        let output_dir = if dev_mode.is_enabled() {
            dev_mode.data_dir().join("visualizations")
        } else {
            PathBuf::from("./visualizations")
        };
        
        fs::create_dir_all(&output_dir)?;
        
        Ok(Self { output_dir })
    }
    
    /// Generate a simple text-based similarity matrix for a user's embeddings
    pub fn generate_similarity_matrix(&self, username: &str, store: &UserStore) -> Result<()> {
        let user_data = store.get_user(username)?;
        let output_file = self.output_dir.join(format!("{}_similarity_matrix.txt", username));
        
        let mut content = String::new();
        content.push_str(&format!("Similarity Matrix for user: {}\n", username));
        content.push_str(&format!("Number of embeddings: {}\n", user_data.embeddings.len()));
        if user_data.averaged_embedding.is_some() {
            content.push_str("Has averaged embedding: Yes\n");
        }
        content.push_str("\n");
        
        // Calculate similarity between all pairs of embeddings
        content.push_str("Pairwise similarities:\n");
        for i in 0..user_data.embeddings.len() {
            for j in i+1..user_data.embeddings.len() {
                let similarity = cosine_similarity(&user_data.embeddings[i], &user_data.embeddings[j]);
                content.push_str(&format!("Embedding {} vs {}: {:.3}\n", i, j, similarity));
            }
        }
        
        // If averaged embedding exists, compare it with individual embeddings
        if let Some(ref avg_embedding) = user_data.averaged_embedding {
            content.push_str("\nSimilarities with averaged embedding:\n");
            for (i, embedding) in user_data.embeddings.iter().enumerate() {
                let similarity = cosine_similarity(embedding, avg_embedding);
                content.push_str(&format!("Embedding {} vs Averaged: {:.3}\n", i, similarity));
            }
        }
        
        fs::write(output_file, content)?;
        println!("Saved similarity matrix to visualizations/{}_similarity_matrix.txt", username);
        
        Ok(())
    }
    
    /// Generate embedding statistics
    pub fn generate_embedding_stats(&self, username: &str, store: &UserStore) -> Result<()> {
        let user_data = store.get_user(username)?;
        let output_file = self.output_dir.join(format!("{}_embedding_stats.txt", username));
        
        let mut content = String::new();
        content.push_str(&format!("Embedding Statistics for user: {}\n", username));
        content.push_str(&format!("Number of embeddings: {}\n", user_data.embeddings.len()));
        content.push_str(&format!("Embedding dimension: {}\n", 
                                user_data.embeddings.first()
                                    .map(|e| e.len())
                                    .unwrap_or(0)));
        content.push_str("\n");
        
        // Calculate statistics for each embedding
        for (i, embedding) in user_data.embeddings.iter().enumerate() {
            let mean = embedding.iter().sum::<f32>() / embedding.len() as f32;
            let variance = embedding.iter()
                .map(|x| (x - mean).powi(2))
                .sum::<f32>() / embedding.len() as f32;
            let std_dev = variance.sqrt();
            let min = embedding.iter().cloned().fold(f32::INFINITY, f32::min);
            let max = embedding.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            
            content.push_str(&format!("Embedding {}:\n", i));
            content.push_str(&format!("  Mean: {:.6}\n", mean));
            content.push_str(&format!("  Std Dev: {:.6}\n", std_dev));
            content.push_str(&format!("  Min: {:.6}\n", min));
            content.push_str(&format!("  Max: {:.6}\n", max));
            content.push_str(&format!("  L2 Norm: {:.6}\n", l2_norm(embedding)));
            content.push_str("\n");
        }
        
        fs::write(output_file, content)?;
        println!("Saved embedding statistics to visualizations/{}_embedding_stats.txt", username);
        
        Ok(())
    }
    
    /// Generate a CSV file of embeddings for external visualization
    pub fn export_embeddings_csv(&self, username: &str, store: &UserStore) -> Result<()> {
        let user_data = store.get_user(username)?;
        let output_file = self.output_dir.join(format!("{}_embeddings.csv", username));
        
        let mut content = String::new();
        
        // Header
        if let Some(first_embedding) = user_data.embeddings.first() {
            let headers: Vec<String> = (0..first_embedding.len())
                .map(|i| format!("dim_{}", i))
                .collect();
            content.push_str("embedding_id,");
            content.push_str(&headers.join(","));
            content.push_str("\n");
            
            // Data
            for (i, embedding) in user_data.embeddings.iter().enumerate() {
                content.push_str(&format!("{}", i));
                for value in embedding {
                    content.push_str(&format!(",{}", value));
                }
                content.push_str("\n");
            }
            
            // Add averaged embedding if it exists
            if let Some(ref avg_embedding) = user_data.averaged_embedding {
                content.push_str("averaged");
                for value in avg_embedding {
                    content.push_str(&format!(",{}", value));
                }
                content.push_str("\n");
            }
        }
        
        fs::write(output_file, content)?;
        println!("Exported embeddings to visualizations/{}_embeddings.csv", username);
        println!("You can visualize this data using Python, R, or any data visualization tool");
        
        Ok(())
    }
}

// Helper functions
fn cosine_similarity(a: &Embedding, b: &Embedding) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    
    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a = l2_norm(a);
    let norm_b = l2_norm(b);
    
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    
    dot_product / (norm_a * norm_b)
}

fn l2_norm(embedding: &Embedding) -> f32 {
    embedding.iter().map(|x| x * x).sum::<f32>().sqrt()
}