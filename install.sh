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
mkdir -p /var/lib/linuxsup/users
mkdir -p /var/lib/linuxsup/enrollment
mkdir -p /usr/share/linuxsup/models
mkdir -p /usr/local/bin

# Build the project
echo "Building LinuxSup..."
cargo build --release --all

# Copy binaries
echo "Installing binaries..."
cp target/release/linuxsup /usr/local/bin/
cp target/release/linuxsup-auth /usr/local/bin/
cp target/release/linuxsup-embedding-service /usr/local/bin/
cp linuxsup-pam-wrapper /usr/local/bin/
chmod 755 /usr/local/bin/linuxsup
chmod 755 /usr/local/bin/linuxsup-auth
chmod 755 /usr/local/bin/linuxsup-embedding-service
chmod 755 /usr/local/bin/linuxsup-pam-wrapper

# Copy configuration
echo "Installing configuration..."
if [ ! -f /etc/linuxsup/face-auth.toml ]; then
    cp configs/face-auth.toml /etc/linuxsup/
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
echo "Building PAM module..."
if [ -f "pam_module/target/release/libpam_linuxsup.so" ]; then
    echo "Installing PAM module..."
    cp pam_module/target/release/libpam_linuxsup.so /lib/security/pam_linuxsup.so
    chmod 644 /lib/security/pam_linuxsup.so
    echo "PAM module installed successfully"
else
    echo "⚠️  PAM module not built. Using pam_exec fallback."
    echo "   For production use, the native PAM module is recommended."
fi

# Create linuxsup user for service
if ! id -u linuxsup >/dev/null 2>&1; then
    echo "Creating linuxsup user..."
    useradd -r -s /bin/false -d /nonexistent -c "LinuxSup Service" linuxsup
    usermod -a -G video linuxsup
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
/usr/local/bin/linuxsup-auth
/usr/local/bin/linuxsup-embedding-service
/usr/local/bin/linuxsup-pam-wrapper
/lib/security/pam_linuxsup.so
/etc/systemd/system/linuxsup-embedding.service
/etc/linuxsup
/var/lib/linuxsup
/usr/share/linuxsup
EOF

# Set permissions
echo "Setting permissions..."
chmod 700 /var/lib/linuxsup
chmod 700 /var/lib/linuxsup/users
chmod 755 /var/lib/linuxsup/enrollment
chmod 755 /etc/linuxsup
chmod 755 /usr/share/linuxsup

# Create example PAM configurations
echo "Creating example PAM configurations..."
mkdir -p examples/pam.d

# Sudo configuration (Native PAM module - RECOMMENDED)
cat > examples/pam.d/sudo-with-face-native <<'EOF'
#%PAM-1.0
# LinuxSup face authentication for sudo (Native PAM module)
# Copy this file to /etc/pam.d/sudo to enable

# Face authentication (optional - falls back to password)
auth    sufficient    pam_linuxsup.so

# Default sudo authentication
@include common-auth
@include common-account
@include common-session-noninteractive
EOF

# Sudo configuration (pam_exec fallback)
cat > examples/pam.d/sudo-with-face <<'EOF'
#%PAM-1.0
# LinuxSup face authentication for sudo (pam_exec fallback)
# Copy this file to /etc/pam.d/sudo to enable

# Face authentication (optional - falls back to password)
auth    sufficient    pam_exec.so    quiet    stdout    /usr/local/bin/linuxsup-pam-wrapper

# Default sudo authentication
@include common-auth
@include common-account
@include common-session-noninteractive
EOF

# GDM configuration
cat > examples/pam.d/gdm-password-with-face <<'EOF'
#%PAM-1.0
# LinuxSup face authentication for GNOME login
# Copy this file to /etc/pam.d/gdm-password to enable

# Face authentication first
auth    sufficient    pam_exec.so    quiet    stdout    /usr/local/bin/linuxsup-pam-wrapper

# Fall back to standard authentication
auth    requisite     pam_nologin.so
auth    required      pam_succeed_if.so user != root quiet_success
@include common-auth
auth    optional      pam_gnome_keyring.so
@include common-account
@include common-session
session optional      pam_gnome_keyring.so auto_start
@include common-password
EOF

# SDDM configuration  
cat > examples/pam.d/sddm-with-face <<'EOF'
#%PAM-1.0
# LinuxSup face authentication for KDE/SDDM login
# Copy this file to /etc/pam.d/sddm to enable

# Face authentication first
auth    sufficient    pam_exec.so    quiet    stdout    /usr/local/bin/linuxsup-pam-wrapper

# Standard SDDM authentication
auth    include       common-auth
account include       common-account
password include      common-password
session include       common-session
EOF

# Console login configuration
cat > examples/pam.d/login-with-face <<'EOF'
#%PAM-1.0
# LinuxSup face authentication for console login
# Copy this file to /etc/pam.d/login to enable
# WARNING: Console may not have camera access!

# Face authentication (if camera available)
auth    sufficient    pam_exec.so    quiet    stdout    /usr/local/bin/linuxsup-pam-wrapper

# Standard login authentication
auth    requisite     pam_nologin.so
auth    include       common-auth
account include       common-account
session include       common-session
password include      common-password
EOF

echo
echo "Installation complete!"
echo
echo "Next steps:"
echo "1. Start the embedding service: sudo systemctl start linuxsup-embedding"
echo "2. Add current user to video group: sudo usermod -a -G video $USER"
echo "3. Log out and back in for group change to take effect"
echo "4. Enroll yourself: sudo linuxsup --system enroll -u $USER"
echo "5. Test authentication: sudo linuxsup --system test -u $USER"
echo
echo "To enable face authentication:"
if [ -f "/lib/security/pam_linuxsup.so" ]; then
    echo "- For sudo (RECOMMENDED): sudo cp examples/pam.d/sudo-with-face-native /etc/pam.d/sudo"
    echo "- For sudo (fallback): sudo cp examples/pam.d/sudo-with-face /etc/pam.d/sudo"
else
    echo "- For sudo: sudo cp examples/pam.d/sudo-with-face /etc/pam.d/sudo"
fi
echo "- For GNOME login: sudo cp examples/pam.d/gdm-password-with-face /etc/pam.d/gdm-password"
echo "- For KDE login: sudo cp examples/pam.d/sddm-with-face /etc/pam.d/sddm"
echo
echo "⚠️  IMPORTANT: Keep a root terminal open when modifying PAM!"
echo "⚠️  Test in a new terminal before closing the root terminal!"
echo
echo "To enable automatic startup:"
echo "  sudo systemctl enable linuxsup-embedding"
echo
echo "To uninstall, run: sudo ./uninstall.sh"
echo