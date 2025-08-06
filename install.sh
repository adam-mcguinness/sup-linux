#!/bin/bash
set -e

echo "LinuxSup Face Authentication - Installation Script"
echo "================================================="
echo
echo "⚠️  WARNING: This is a TEST VERSION - NOT SECURE"
echo "⚠️  Do not use in production environments!"
echo
read -p "Continue with installation? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Installation cancelled."
    exit 1
fi

# Check if running as root
if [ "$EUID" -ne 0 ]; then 
    echo "This script must be run as root (use sudo)"
    exit 1
fi

# Create directories
echo "Creating system directories..."
mkdir -p /etc/linuxsup
mkdir -p /var/lib/linuxsup
mkdir -p /var/lib/linuxsup/users
mkdir -p /var/lib/linuxsup/enrollment
mkdir -p /usr/share/linuxsup/models
mkdir -p /usr/local/bin
mkdir -p /usr/local/lib/linuxsup

# Check if binaries are built
echo "Checking for required binaries..."
REQUIRED_BINARIES=(
    "target/release/linuxsup"
    "target/release/linuxsup-embedding-service"
    "target/release/libpam_linuxsup.so"
)

MISSING_BINARIES=()
for binary in "${REQUIRED_BINARIES[@]}"; do
    if [ ! -f "$binary" ]; then
        MISSING_BINARIES+=("$binary")
    fi
done

if [ ${#MISSING_BINARIES[@]} -ne 0 ]; then
    echo "❌ ERROR: Required binaries not found!"
    echo "   Missing:"
    for binary in "${MISSING_BINARIES[@]}"; do
        echo "   - $binary"
    done
    echo
    echo "   Please run './build.sh' first to build the project."
    echo "   Then run 'sudo ./install.sh' to install."
    exit 1
fi
echo "✅ All required binaries found"

# Validate models
echo "Checking for required models..."
if [ ! -f "models/detect.onnx" ] || [ ! -f "models/compare.onnx" ]; then
    echo "❌ ERROR: Missing required models!"
    echo "   Required files: models/detect.onnx, models/compare.onnx"
    echo "   Contact repository maintainer for model access."
    exit 1
fi
echo "✅ Models found: detect.onnx, compare.onnx"

# Copy binaries
echo "Installing binaries..."
cp target/release/linuxsup /usr/local/bin/
cp target/release/linuxsup-embedding-service /usr/local/bin/
chmod 755 /usr/local/bin/linuxsup
chmod 755 /usr/local/bin/linuxsup-embedding-service

# Install ONNX Runtime library to LinuxSup directory
echo "Installing ONNX Runtime library..."
if [ -f "target/release/libonnxruntime.so" ]; then
    cp target/release/libonnxruntime.so* /usr/local/lib/linuxsup/
    # Create wrapper scripts that set LD_LIBRARY_PATH
    cat > /usr/local/bin/linuxsup.tmp << 'EOF'
#!/bin/bash
export LD_LIBRARY_PATH="/usr/local/lib/linuxsup:$LD_LIBRARY_PATH"
exec /usr/local/bin/linuxsup.bin "$@"
EOF
    cat > /usr/local/bin/linuxsup-embedding-service.tmp << 'EOF'
#!/bin/bash
export LD_LIBRARY_PATH="/usr/local/lib/linuxsup:$LD_LIBRARY_PATH"
exec /usr/local/bin/linuxsup-embedding-service.bin "$@"
EOF
    # Move binaries to .bin and wrappers to main names
    mv /usr/local/bin/linuxsup /usr/local/bin/linuxsup.bin
    mv /usr/local/bin/linuxsup-embedding-service /usr/local/bin/linuxsup-embedding-service.bin
    mv /usr/local/bin/linuxsup.tmp /usr/local/bin/linuxsup
    mv /usr/local/bin/linuxsup-embedding-service.tmp /usr/local/bin/linuxsup-embedding-service
    chmod 755 /usr/local/bin/linuxsup
    chmod 755 /usr/local/bin/linuxsup-embedding-service
    echo "✅ ONNX Runtime installed to /usr/local/lib/linuxsup/"
else
    echo "⚠️  ONNX Runtime not found in build directory"
    echo "   The application may not work correctly"
fi

# Copy configuration (use system version with absolute paths)
echo "Installing configuration..."
if [ ! -f /etc/linuxsup/face-auth.toml ]; then
    # Use system config with absolute paths for models
    if [ -f configs/face-auth-system.toml ]; then
        cp configs/face-auth-system.toml /etc/linuxsup/face-auth.toml
    else
        # Fall back to regular config if system version doesn't exist
        cp configs/face-auth.toml /etc/linuxsup/face-auth.toml
    fi
    chmod 644 /etc/linuxsup/face-auth.toml
else
    echo "Config file already exists, skipping..."
fi

# Copy models
echo "Installing models..."
if [ -d "models" ]; then
    cp models/*.onnx /usr/share/linuxsup/models/ 2>/dev/null || true
    chmod 644 /usr/share/linuxsup/models/*.onnx 2>/dev/null || true
else
    echo "⚠️  No models found in ./models directory"
    echo "   Please copy your ONNX models to /usr/share/linuxsup/models/"
fi

# Create PAM module directory if it doesn't exist
mkdir -p /lib/security

# Build and install PAM module
echo "Installing PAM module..."
if [ -f "target/release/libpam_linuxsup.so" ]; then
    echo "Installing native PAM module..."
    cp target/release/libpam_linuxsup.so /lib/security/pam_linuxsup.so
    chmod 644 /lib/security/pam_linuxsup.so
    echo "✅ Native PAM module installed successfully"
else
    echo "❌ ERROR: Native PAM module not found!"
    echo "   The PAM module is required for authentication."
    echo "   Run 'cargo build --release --all' to build all components."
    exit 1
fi

# Create linuxsup user for service (no group needed since service handles everything)
if ! id -u linuxsup >/dev/null 2>&1; then
    echo "Creating linuxsup service user..."
    useradd -r -s /bin/false -d /nonexistent -c "LinuxSup Service" linuxsup
    usermod -a -G video linuxsup  # Need video group for camera access
fi

# Install systemd service
if [ -d "/etc/systemd/system" ]; then
    echo "Installing systemd service..."
    cp systemd/linuxsup-embedding.service /etc/systemd/system/
    systemctl daemon-reload
    echo "To enable the service: systemctl enable linuxsup-embedding"
    echo "To start the service: systemctl start linuxsup-embedding"
fi

# Create tracking file for uninstall
echo "Creating installation manifest..."
cat > /var/lib/linuxsup/.installed_files <<EOF
/usr/local/bin/linuxsup
/usr/local/bin/linuxsup.bin
/usr/local/bin/linuxsup-embedding-service
/usr/local/bin/linuxsup-embedding-service.bin
/usr/local/lib/linuxsup
/lib/security/pam_linuxsup.so
/etc/systemd/system/linuxsup-embedding.service
/etc/linuxsup
/var/lib/linuxsup
/usr/share/linuxsup
EOF

# Set permissions for LinuxSup directories
echo "Setting permissions..."
# All data directories owned exclusively by service user
chown -R linuxsup:linuxsup /var/lib/linuxsup
chmod 700 /var/lib/linuxsup  # Only service can access
chmod 700 /var/lib/linuxsup/users
chmod 700 /var/lib/linuxsup/enrollment

# Config directory - readable by all
chmod 755 /etc/linuxsup

# Models directory - readable by all
chmod 755 /usr/share/linuxsup

# Note: PAM configurations are already in examples/pam.d/
echo "PAM configuration examples available in examples/pam.d/"

echo
echo "Installation complete!"
echo
echo "Next steps:"
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "STEP 1: Start the Service"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "1. Start the embedding service:"
echo "   sudo systemctl start linuxsup-embedding"
echo "   sudo systemctl enable linuxsup-embedding  # (optional: auto-start)"
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "STEP 2: Enroll and Test (No logout required!)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "2. Enroll yourself (service handles everything):"
echo "   linuxsup enroll --username $USER"
echo
echo "3. Test authentication:"
echo "   linuxsup test --username $USER"
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "STEP 3: Enable PAM Integration (OPTIONAL - BE CAREFUL)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "⚠️  BEFORE modifying PAM, open a root shell as backup:"
echo "   sudo -i  # Keep this terminal open!"
echo
echo "Then in ANOTHER terminal, enable face auth:"
echo "- For sudo: sudo cp examples/pam.d/sudo-with-face /etc/pam.d/sudo"
echo "- For GNOME: sudo cp examples/pam.d/gdm-with-face /etc/pam.d/gdm-password"
echo "- For KDE: sudo cp examples/pam.d/sddm-with-face /etc/pam.d/sddm"
echo
echo "Test sudo in a NEW terminal before closing the root shell."
echo "All configs include password fallback for safety."
echo
echo "To enable automatic startup:"
echo "  sudo systemctl enable linuxsup-embedding"
echo
echo "To uninstall, run: sudo ./uninstall.sh"
echo