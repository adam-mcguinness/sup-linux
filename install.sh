#!/bin/bash
set -e

echo "SupLinux Face Authentication - Installation Script"
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
mkdir -p /etc/suplinux
mkdir -p /var/lib/suplinux
mkdir -p /var/lib/suplinux/users
mkdir -p /var/lib/suplinux/enrollment
mkdir -p /usr/share/suplinux/models
mkdir -p /usr/local/bin
mkdir -p /usr/local/lib/suplinux

# Check if binaries are built
echo "Checking for required binaries..."
REQUIRED_BINARIES=(
    "target/release/suplinux"
    "target/release/suplinux-service"
    "target/release/libpam_suplinux.so"
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
cp target/release/suplinux /usr/local/bin/
cp target/release/suplinux-service /usr/local/bin/
chmod 755 /usr/local/bin/suplinux
chmod 755 /usr/local/bin/suplinux-service

# Install ONNX Runtime library to SupLinux directory
echo "Installing ONNX Runtime library..."
if [ -f "target/release/libonnxruntime.so" ]; then
    cp target/release/libonnxruntime.so* /usr/local/lib/suplinux/
    # Create wrapper scripts that set LD_LIBRARY_PATH
    cat > /usr/local/bin/suplinux.tmp << 'EOF'
#!/bin/bash
export LD_LIBRARY_PATH="/usr/local/lib/suplinux:$LD_LIBRARY_PATH"
exec /usr/local/bin/suplinux.bin "$@"
EOF
    cat > /usr/local/bin/suplinux-service.tmp << 'EOF'
#!/bin/bash
export LD_LIBRARY_PATH="/usr/local/lib/suplinux:$LD_LIBRARY_PATH"
exec /usr/local/bin/suplinux-service.bin "$@"
EOF
    # Move binaries to .bin and wrappers to main names
    mv /usr/local/bin/suplinux /usr/local/bin/suplinux.bin
    mv /usr/local/bin/suplinux-service /usr/local/bin/suplinux-service.bin
    mv /usr/local/bin/suplinux.tmp /usr/local/bin/suplinux
    mv /usr/local/bin/suplinux-service.tmp /usr/local/bin/suplinux-service
    chmod 755 /usr/local/bin/suplinux
    chmod 755 /usr/local/bin/suplinux-service
    echo "✅ ONNX Runtime installed to /usr/local/lib/suplinux/"
else
    echo "⚠️  ONNX Runtime not found in build directory"
    echo "   The application may not work correctly"
fi

# Copy configuration (use system version with absolute paths)
echo "Installing configuration..."
if [ ! -f /etc/suplinux/face-auth.toml ]; then
    # Use system config with absolute paths for models
    if [ -f configs/face-auth-system.toml ]; then
        cp configs/face-auth-system.toml /etc/suplinux/face-auth.toml
    else
        # Fall back to regular config if system version doesn't exist
        cp configs/face-auth.toml /etc/suplinux/face-auth.toml
    fi
    chmod 644 /etc/suplinux/face-auth.toml
else
    echo "Config file already exists, skipping..."
fi

# Copy models
echo "Installing models..."
if [ -d "models" ]; then
    cp models/*.onnx /usr/share/suplinux/models/ 2>/dev/null || true
    chmod 644 /usr/share/suplinux/models/*.onnx 2>/dev/null || true
else
    echo "⚠️  No models found in ./models directory"
    echo "   Please copy your ONNX models to /usr/share/suplinux/models/"
fi

# Create PAM module directory if it doesn't exist
mkdir -p /lib/security

# Build and install PAM module
echo "Installing PAM module..."
if [ -f "target/release/libpam_suplinux.so" ]; then
    echo "Installing native PAM module..."
    cp target/release/libpam_suplinux.so /lib/security/pam_suplinux.so
    chmod 644 /lib/security/pam_suplinux.so
    echo "✅ Native PAM module installed successfully"
else
    echo "❌ ERROR: Native PAM module not found!"
    echo "   The PAM module is required for authentication."
    echo "   Run 'cargo build --release --all' to build all components."
    exit 1
fi

# Create suplinux user for service (no group needed since service handles everything)
if ! id -u suplinux >/dev/null 2>&1; then
    echo "Creating suplinux service user..."
    useradd -r -s /bin/false -d /nonexistent -c "SupLinux Service" suplinux
    usermod -a -G video suplinux  # Need video group for camera access
fi

# Install systemd service
if [ -d "/etc/systemd/system" ]; then
    echo "Installing systemd service..."
    cp systemd/suplinux.service /etc/systemd/system/
    systemctl daemon-reload
    echo "To enable the service: systemctl enable suplinux"
    echo "To start the service: systemctl start suplinux"
fi

# Create tracking file for uninstall
echo "Creating installation manifest..."
cat > /var/lib/suplinux/.installed_files <<EOF
/usr/local/bin/suplinux
/usr/local/bin/suplinux.bin
/usr/local/bin/suplinux-service
/usr/local/bin/suplinux-service.bin
/usr/local/lib/suplinux
/lib/security/pam_suplinux.so
/etc/systemd/system/suplinux.service
/etc/suplinux
/var/lib/suplinux
/usr/share/suplinux
EOF

# Set permissions for SupLinux directories
echo "Setting permissions..."
# All data directories owned exclusively by service user
chown -R suplinux:suplinux /var/lib/suplinux
chmod 700 /var/lib/suplinux  # Only service can access
chmod 700 /var/lib/suplinux/users
chmod 700 /var/lib/suplinux/enrollment

# Config directory - readable by all
chmod 755 /etc/suplinux

# Models directory - readable by all
chmod 755 /usr/share/suplinux

# Install PAM profile for pam-auth-update
if [ -d "/usr/share/pam-configs" ]; then
    echo "Installing PAM profile..."
    cp pam-configs/suplinux /usr/share/pam-configs/
    chmod 644 /usr/share/pam-configs/suplinux
    
    # Register with pam-auth-update (without enabling by default)
    if command -v pam-auth-update &> /dev/null; then
        pam-auth-update --package
    fi
fi

echo
echo "Installation complete!"
echo
echo "Next steps:"
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "STEP 1: Start the Service"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "1. Start the service:"
echo "   sudo systemctl start suplinux"
echo "   sudo systemctl enable suplinux  # (optional: auto-start)"
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "STEP 2: Enroll and Test (No logout required!)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "2. Enroll yourself (service handles everything):"
echo "   suplinux enroll --username $USER"
echo
echo "3. Test authentication:"
echo "   suplinux test --username $USER"
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "STEP 3: Enable Face Authentication System-Wide (OPTIONAL)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "To enable face authentication for sudo, login, and desktop:"
echo
echo "1. Run the PAM configuration tool:"
echo "   sudo pam-auth-update"
echo
echo "2. Select 'Face authentication (SupLinux)' with spacebar"
echo "3. Press Enter to apply"
echo
echo "Face auth will then work automatically with:"
echo "- sudo commands"
echo "- System login (GDM/SDDM)"
echo "- Lock screen"
echo "- Any PAM-aware application"
echo
echo "To disable later, run 'sudo pam-auth-update' and deselect it."
echo
echo "To enable automatic startup:"
echo "  sudo systemctl enable suplinux"
echo
echo "To uninstall, run: sudo ./uninstall.sh"
echo