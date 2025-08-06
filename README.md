# LinuxSup - Face Authentication for Linux

A secure face authentication system for Linux, inspired by Windows Hello. Uses infrared cameras for enhanced security and integrates with Linux PAM for system-wide authentication.

## Current Status

**Phase 3 - Secure PAM Integration** - Native PAM module with privilege separation. The system supports:
- K-of-N authentication (require K successful matches out of N attempts)
- Rolling embedding buffer for improved accuracy
- Both system-wide and per-user enrollment
- PAM integration for sudo, GDM, SDDM
- Automatic camera detection with IR camera support

## Features

### Implemented
- ✅ Native PAM module with challenge-response protocol
- ✅ Privilege separation architecture (PAM + embedding service)
- ✅ IR camera support with auto-detection
- ✅ K-of-N matching strategy for robust authentication
- ✅ Rolling buffer with embedding fusion
- ✅ High-quality enrollment with multiple captures
- ✅ INT8 model quantization for ~16+ FPS performance
- ✅ Development mode for safe testing
- ✅ Backward compatibility with pam_exec

### Planned (Phase 2)
- 🔒 AES-256-GCM encryption for embeddings
- 🔑 Secure key storage in kernel keyring
- 🛡️ Enhanced anti-spoofing measures
- 📊 Rate limiting and audit logging

## Development Guide

### Prerequisites

- Rust 1.70+ with workspace support
- V4L2 compatible camera (preferably with IR support)
- Linux with PAM support
- ONNX Runtime compatible system
- Models: `detect.onnx` and `compare.onnx` (see Models section below)

### Complete Setup Guide

#### Development Testing (Recommended First)

1. **Clone and setup:**
```bash
git clone https://github.com/yourusername/linuxSup.git
cd linuxSup
```

2. **Verify models (REQUIRED):**
```bash
# Ensure you have the correct models
ls models/
# Should show: detect.onnx compare.onnx
# If missing, contact repository maintainer for model access
```

3. **Build workspace:**
```bash
# Build all components (main CLI, PAM module, embedding service)
cargo build --release --all
```

4. **Test camera and detection:**
```bash
# Test camera access
cargo run --bin linuxsup -- --dev test-camera

# Test face detection
cargo run --bin linuxsup -- --dev test-detection
```

5. **Test enrollment and authentication:**
```bash
# Enroll yourself (saves to ./dev_data/)
cargo run --bin linuxsup -- --dev enroll --username testuser

# Test authentication
cargo run --bin linuxsup -- --dev test --username testuser
```

#### Production Installation

Only proceed after development testing works:

```bash
# Install system-wide (requires models in place)
sudo ./install.sh

# Follow the installation prompts for PAM setup
```

### Camera Configuration

1. **Find your camera device:**
```bash
# List all video devices
v4l2-ctl --list-devices

# For Logitech BRIO IR camera, look for "Video Capture 4" or similar
# Common IR camera indices: 2, 4, 51
```

2. **Update configuration:**
Edit `configs/face-auth.toml`:
```toml
[camera]
device_index = 0  # Change to your camera index
width = 640
height = 480
```

3. **Test camera capture:**
```bash
# Normal mode (saves to current directory)
cargo run --bin linuxsup -- test-camera

# Development mode (saves to ./dev_data/captures/)
cargo run --bin linuxsup -- --dev test-camera
```

### Testing Face Detection

1. **Basic detection test:**
```bash
# Normal mode
cargo run --bin linuxsup -- test-detection

# Development mode with debug output
cargo run --bin linuxsup -- --dev test-detection
```

2. **Enrollment:**
```bash
# Normal mode (saves to system directories)
cargo run --bin linuxsup -- enroll --username testuser

# Development mode (saves to ./dev_data/)
cargo run --bin linuxsup -- --dev enroll --username testuser
```

3. **Authentication test:**
```bash
# Test authentication
cargo run --bin linuxsup -- --dev test --username testuser
```

### Development Mode

The `--dev` flag enables development mode for safe testing:

**Features:**
- ✅ All data saved to `./dev_data/` directory
- ✅ Enhanced debug logging with file/line numbers
- ✅ No system permissions required
- ✅ Timestamped capture files
- ✅ Easy cleanup: `rm -rf ./dev_data/`

**Directory Structure:**
```
./dev_data/
├── users/          # User embeddings
├── enrollment/     # Enrollment images by user
├── captures/       # Test captures with timestamps
├── logs/           # Debug logs
└── config/         # Dev-specific configs
```

**Example Commands:**
```bash
# All commands support --dev flag
cargo run --bin linuxsup -- --dev test-camera
cargo run --bin linuxsup -- --dev test-detection
cargo run --bin linuxsup -- --dev enroll --username alice
cargo run --bin linuxsup -- --dev test --username alice
```

### Troubleshooting

**Camera not found:**
- Check device permissions: `sudo chmod 666 /dev/video*`
- Verify camera index in config
- Try different indices (0, 2, 4, etc.)

**Face not detected:**
- Ensure good lighting
- Check camera focus
- Verify models downloaded correctly
- Lower detection confidence in config

**IR Camera specific:**
- Some IR cameras need warmup frames
- Adjust `warmup_frames` in config (try 3-10)
- Check if camera outputs GREY format

## Models

### Required Models

The system requires two ONNX models in the `models/` directory:

1. **Face Detection Model** (`detect.onnx`)
   - YOLOv8-based face detection
   - Input: 640x640 RGB image
   - Output: Face bounding boxes with confidence

2. **Face Recognition Model** (`compare.onnx`)
   - ArcFace-based face recognition
   - Input: 112x112 face crop
   - Output: 512-dimensional embedding

### Model Setup

```bash
# Verify you have the required models
ls -la models/
# Should show:
# -rw-r--r-- 1 user user 12251037 detect.onnx
# -rw-r--r-- 1 user user  4397715 compare.onnx

# Test model loading
cargo run --bin linuxsup -- --dev test-detection
```

**Note**: These are proprietary models. Contact the repository maintainer for access. The system will not work without these models.

## Security Notice

⚠️ **Important:** Face authentication should never be the sole authentication method. This system is designed to work alongside traditional passwords as an additional convenience factor.

## Installation

### Quick Install (System-wide)

```bash
# Build and install
sudo ./install.sh

# Start the embedding service
sudo systemctl start linuxsup-embedding
sudo systemctl enable linuxsup-embedding  # For automatic startup

# Enroll yourself
linuxsup --system enroll --username $USER

# Test authentication
linuxsup --system test --username $USER

# Enable for sudo (choose one):
# Option 1: Native PAM module (RECOMMENDED - more secure)
sudo cp examples/pam.d/sudo-with-face-native /etc/pam.d/sudo

# Option 2: pam_exec fallback (for testing)
sudo cp examples/pam.d/sudo-with-face /etc/pam.d/sudo
```

### Configuration

The system uses a TOML configuration file at `/etc/linuxsup/face-auth.toml`:

```toml
[auth]
similarity_threshold = 0.6     # Face matching threshold
k_required_matches = 2         # Require 2 successful matches
n_total_attempts = 3          # Out of 3 total attempts
embedding_buffer_size = 3     # Rolling buffer size
use_embedding_fusion = true   # Enable temporal fusion

[camera]
device_index = 51            # 999 for auto-detect
warmup_frames = 3           # IR camera warmup
```

## Architecture

LinuxSup now uses a secure architecture with privilege separation:

```
┌─────────────┐     ┌────────────────┐     ┌──────────────────┐
│ PAM Module  │────▶│  Unix Socket   │────▶│ Embedding Service│
│(pam_linuxsup)     │ /run/linuxsup/ │     │ (unprivileged)  │
└─────────────┘     └────────────────┘     └──────────────────┘
        │                                            │
        ▼                                            ▼
┌─────────────┐                              ┌──────────────┐
│ User Data   │                              │ Camera/Models│
│ (privileged)│                              │ (read-only)  │
└─────────────┘                              └──────────────┘
```

- **PAM Module**: Handles authentication decisions and user data access
- **Embedding Service**: Only captures faces and generates embeddings
- **Challenge-Response**: Prevents replay attacks and service spoofing

## Implementation Status

Current phase: **Phase 3 - Secure PAM Integration**

- ✅ Native PAM module with challenge-response protocol
- ✅ Privilege separation between authentication and face capture
- ✅ Systemd service for embedding generation
- 🔄 Backward compatible with pam_exec during transition
- ⏳ Phase 2 encryption features pending

## License

[Your License Here]