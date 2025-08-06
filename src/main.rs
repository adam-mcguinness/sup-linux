mod ascii_preview;
mod auth;
mod camera;
mod config;
mod detector;
mod dev_mode;
mod error;
mod protocol;
mod quality;
mod recognizer;
mod service_client;
mod storage;
mod visualization;


use clap::{Parser, Subcommand};
use anyhow::Result;

#[derive(Parser)]
#[command(name = "linuxSup")]
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
            println!("Detecting IR camera...");
            camera::Camera::detect_ir_camera()?;
        }
        Commands::Enroll { username } => {
            println!("Enrolling user: {}", username);
            auth::enroll_user_dev(&username, &dev_mode)?;
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