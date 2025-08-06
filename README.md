# LinuxSup - Face Authentication for Linux

A secure face authentication system for Linux, inspired by Windows Hello. Uses infrared cameras for enhanced security and integrates with Linux PAM for system-wide authentication.

## Features

- 🎥 IR camera support for better security
- 🔐 Secure biometric data storage with encryption
- 🚀 Fast authentication using ONNX models
- 🐧 PAM integration for sudo, login, and lock screen
- 🛡️ Anti-spoofing with liveness detection

## Development Guide

### Prerequisites

- Rust 1.70+
- V4L2 compatible camera (preferably with IR support)
- Linux with PAM support

### Quick Start

1. **Clone and build:**
```bash
git clone https://github.com/yourusername/linuxSup.git
cd linuxSup
cargo build --release
```

2. **Download required models:**
```bash
cd models
# Run the commands below to download the models
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
cargo run -- test-camera

# Development mode (saves to ./dev_data/captures/)
cargo run -- --dev test-camera
```

### Testing Face Detection

1. **Basic detection test:**
```bash
# Normal mode
cargo run -- test-detection

# Development mode with debug output
cargo run -- --dev test-detection
```

2. **Enrollment:**
```bash
# Normal mode (saves to system directories)
cargo run -- enroll --username testuser

# Development mode (saves to ./dev_data/)
cargo run -- --dev enroll --username testuser
```

3. **Authentication test:**
```bash
# Test authentication
cargo run -- --dev test --username testuser
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
cargo run -- --dev test-camera
cargo run -- --dev test-detection
cargo run -- --dev enroll --username alice
cargo run -- --dev test --username alice
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

### Face Detection - UltraFace
Lightweight face detection model optimized for speed.
- Model: `ultraface_640.onnx`
- Input: 640x480 RGB image
- Output: Face bounding boxes with confidence

### Face Recognition - ArcFace
State-of-the-art face recognition model.
- Model: `arcface_r100.onnx` 
- Input: 112x112 face crop
- Output: 512-dimensional embedding

### Download Models

```bash
cd models

# Clean up any existing models
rm -f *.onnx

# Download UltraFace detector
echo "Downloading UltraFace model..."
curl -L -o ultraface_640.onnx https://github.com/onnx/models/raw/main/validated/vision/body_analysis/ultraface/models/version-RFB-640.onnx

# Download ArcFace recognizer (INT8 version for better performance)
echo "Downloading ArcFace INT8 model..."
curl -L -o arcface_r100.onnx https://github.com/onnx/models/raw/main/validated/vision/body_analysis/arcface/model/arcfaceresnet100-11-int8.onnx

# Verify downloads (should show "data" type and file sizes)
echo "Verifying downloads..."
file *.onnx
ls -lh *.onnx

cd ..
```


https://vcipl-okstate.org/pbvs/bench/Data/07/download.html

## Security Notice

⚠️ **Important:** Face authentication should never be the sole authentication method. This system is designed to work alongside traditional passwords as an additional convenience factor.

## Implementation Status

See [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) for detailed development roadmap.

Current phase: **Phase 1 - Local Development Mode**

## License

[Your License Here]