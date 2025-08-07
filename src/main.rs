use sup_linux::{
    auth,
    camera,
    dev_mode,
    storage,
    visualization,
};

use clap::{Parser, Subcommand};
use anyhow::Result;

#[derive(Parser)]
#[command(name = "supLinux")]
#[command(about = "Linux face authentication system")]
struct Cli {
    /// Enable development mode (saves data locally for testing)
    #[arg(long, global = true)]
    dev: bool,
    
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Test camera
    TestCamera,
    /// Test face detection
    TestDetection,
    /// Detect IR camera automatically
    DetectCamera,
    /// Enroll a new face
    Enroll {
        #[arg(short, long)]
        username: String,
    },
    /// Enhance existing enrollment with additional embeddings
    Enhance {
        #[arg(short, long)]
        username: String,
        /// Number of additional captures (default: 3)
        #[arg(short = 'c', long, default_value = "3")]
        additional_captures: u32,
        /// Replace weak embeddings instead of just appending
        #[arg(short = 'r', long)]
        replace_weak: bool,
    },
    /// Test authentication
    Test {
        #[arg(short, long)]
        username: String,
    },
    /// Visualize user data
    Visualize {
        #[arg(short, long)]
        username: String,
        #[command(subcommand)]
        command: Option<VisualizeCommands>,
    },
}

#[derive(Subcommand)]
enum VisualizeCommands {
    /// Generate similarity matrix
    Similarity,
    /// Generate embedding statistics
    Stats,
    /// Export embeddings to CSV
    Export,
    /// Generate all visualizations
    All,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Setup logging based on mode
    setup_logging(cli.dev);
    
    // Create dev mode context
    let dev_mode = dev_mode::DevMode::new(cli.dev)?;

    match cli.command {
        Commands::TestCamera => {
            println!("Testing camera...");
            auth::test_camera_dev(&dev_mode)?;
        }
        Commands::TestDetection => {
            println!("Testing face detection...");
            auth::test_detection_dev(&dev_mode)?;
        }
        Commands::DetectCamera => {
            println!("🔍 Detecting available cameras...\n");
            
            let cameras = camera::Camera::list_all_cameras()?;
            
            if cameras.is_empty() {
                println!("❌ No cameras found!");
                println!("\nTroubleshooting:");
                println!("  1. Check if cameras are connected");
                println!("  2. Ensure you have permission to access /dev/video*");
                println!("  3. Try: sudo chmod 666 /dev/video*");
                return Ok(());
            }
            
            // Find best candidate
            let mut selected_index = None;
            let mut ir_candidates = Vec::new();
            let mut other_candidates = Vec::new();
            
            for (index, name, features, likely_ir) in &cameras {
                println!("📷 /dev/video{}: {}", index, name);
                for feature in features {
                    println!("   - {}", feature);
                }
                
                if *likely_ir {
                    ir_candidates.push(*index);
                    if selected_index.is_none() && features.iter().any(|f| f.contains("Grayscale")) {
                        selected_index = Some(*index);
                    }
                } else if features.iter().any(|f| f.contains("VIDEO_CAPTURE")) {
                    other_candidates.push(*index);
                }
                println!();
            }
            
            // Auto-detection result
            println!("═══════════════════════════════════════════════════════");
            if let Some(idx) = selected_index {
                println!("✅ Auto-detected IR camera: /dev/video{}", idx);
                println!("\nThis will be used when device_index = 999 (auto-detect)");
            } else if !ir_candidates.is_empty() {
                println!("⚠️  Potential IR cameras found but no grayscale format detected:");
                for idx in &ir_candidates {
                    println!("   - /dev/video{}", idx);
                }
            } else {
                println!("⚠️  No IR camera detected. Will use default camera (video0)");
            }
            
            // Manual configuration instructions
            println!("\n📝 To manually set a camera, edit the configuration:");
            println!("   sudo nano /etc/suplinux/face-auth.toml");
            println!("\n   Change the device_index value:");
            println!("   [camera]");
            println!("   device_index = <NUMBER>  # Replace <NUMBER> with desired index");
            
            if !ir_candidates.is_empty() || !other_candidates.is_empty() {
                println!("\n💡 Suggested cameras to try:");
                for idx in ir_candidates.iter().chain(other_candidates.iter()).take(3) {
                    println!("   device_index = {}", idx);
                }
            }
            
            println!("\n🔧 After editing, test with:");
            println!("   suplinux test-camera");
        }
        Commands::Enroll { username } => {
            println!("Enrolling user: {}", username);
            auth::enroll_user_dev(&username, &dev_mode)?;
        }
        Commands::Enhance { username, additional_captures, replace_weak } => {
            println!("Enhancing enrollment for user: {}", username);
            auth::enhance_user_dev(&username, additional_captures, replace_weak, &dev_mode)?;
        }
        Commands::Test { username } => {
            println!("Testing authentication for: {}", username);
            let result = auth::authenticate_user_dev(&username, &dev_mode)?;
            println!("Authentication: {}", if result { "SUCCESS" } else { "FAILED" });
        }
        Commands::Visualize { username, command } => {
            let store = storage::UserStore::new_with_dev_mode(&dev_mode)?;
            let visualizer = visualization::Visualizer::new(&dev_mode)?;
            
            match command.unwrap_or(VisualizeCommands::All) {
                VisualizeCommands::Similarity => {
                    visualizer.generate_similarity_matrix(&username, &store)?;
                }
                VisualizeCommands::Stats => {
                    visualizer.generate_embedding_stats(&username, &store)?;
                }
                VisualizeCommands::Export => {
                    visualizer.export_embeddings_csv(&username, &store)?;
                }
                VisualizeCommands::All => {
                    visualizer.generate_similarity_matrix(&username, &store)?;
                    visualizer.generate_embedding_stats(&username, &store)?;
                    visualizer.export_embeddings_csv(&username, &store)?;
                }
            }
        }
    }

    Ok(())
}

fn setup_logging(dev_mode: bool) {
    if dev_mode {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_file(true)
            .with_line_number(true)
            .with_thread_ids(true)
            .init();
    } else {
        tracing_subscriber::fmt::init();
    }
}