#!/bin/bash
set -e

echo "SupLinux Face Authentication - Build Script"
echo "==========================================="
echo

# Check if running as root
if [ "$EUID" -eq 0 ]; then 
    echo "⚠️  Warning: Running as root. It's recommended to build as a normal user."
    echo "   The install script should be run with sudo after building."
    read -p "Continue anyway? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Build cancelled."
        exit 1
    fi
fi

# Check for Rust/Cargo
if ! command -v cargo &> /dev/null; then
    echo "❌ ERROR: Cargo not found!"
    echo "   Please install Rust: https://rustup.rs/"
    exit 1
fi

echo "Rust version:"
rustc --version
cargo --version
echo

# Validate models first
echo "Checking for required models..."
if [ ! -f "models/detect.onnx" ] || [ ! -f "models/compare.onnx" ]; then
    echo "❌ ERROR: Missing required models!"
    echo "   Required files:"
    echo "   - models/detect.onnx"
    echo "   - models/compare.onnx"
    echo "   Contact repository maintainer for model access."
    exit 1
fi
echo "✅ Models found: detect.onnx, compare.onnx"
echo

# Clean previous builds (optional)
if [ -d "target/release" ]; then
    read -p "Clean previous build? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo "Cleaning previous build..."
        cargo clean --release
    fi
fi

# Build the project
echo "Building SupLinux (this may take a few minutes)..."
if ! cargo build --release --all; then
    echo "❌ Build failed. Please check error messages above."
    exit 1
fi

echo
echo "Verifying build artifacts..."

# Check all required binaries
REQUIRED_BINARIES=(
    "target/release/suplinux"
    "target/release/suplinux-service"
    "target/release/libpam_suplinux.so"
)

MISSING_BINARIES=()
for binary in "${REQUIRED_BINARIES[@]}"; do
    if [ ! -f "$binary" ]; then
        MISSING_BINARIES+=("$binary")
    else
        echo "✅ Found: $binary"
    fi
done

if [ ${#MISSING_BINARIES[@]} -ne 0 ]; then
    echo
    echo "❌ ERROR: Missing required binaries:"
    for binary in "${MISSING_BINARIES[@]}"; do
        echo "   - $binary"
    done
    echo
    echo "Build may have partially failed. Check error messages above."
    exit 1
fi

echo
echo "✅ Build successful!"
echo
echo "All required components built:"
echo "  - suplinux (main CLI)"
echo "  - suplinux-service (systemd service)"
echo "  - libpam_suplinux.so (PAM module)"
echo
echo "Next steps:"
echo "  1. Run the installation script: sudo ./install.sh"
echo "  2. Follow the post-installation instructions"
echo
echo "Note: The install script will copy these binaries to system directories."