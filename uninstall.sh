#!/bin/bash
set -e

echo "SupLinux Face Authentication - Uninstall Script"
echo "=============================================="
echo
read -p "This will remove SupLinux from your system. Continue? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Uninstall cancelled."
    exit 1
fi

# Check if running as root
if [ "$EUID" -ne 0 ]; then 
    echo "This script must be run as root (use sudo)"
    exit 1
fi

echo "Removing SupLinux..."

# Stop and disable systemd service if running
if systemctl is-active --quiet suplinux; then
    echo "Stopping service..."
    systemctl stop suplinux
fi

if systemctl is-enabled --quiet suplinux 2>/dev/null; then
    echo "Disabling service..."
    systemctl disable suplinux
fi

# Remove systemd service file
if [ -f /etc/systemd/system/suplinux.service ]; then
    echo "Removing systemd service..."
    rm -f /etc/systemd/system/suplinux.service
    systemctl daemon-reload
fi

# Remove binaries and wrappers
echo "Removing binaries..."
rm -f /usr/local/bin/suplinux
rm -f /usr/local/bin/suplinux.bin
rm -f /usr/local/bin/suplinux-service
rm -f /usr/local/bin/suplinux-service.bin

# Remove PAM module if it exists
if [ -f /lib/security/pam_suplinux.so ]; then
    echo "Removing PAM module..."
    rm -f /lib/security/pam_suplinux.so
fi

# Remove PAM profile and update pam-auth-update
if [ -f /usr/share/pam-configs/suplinux ]; then
    echo "Removing PAM profile..."
    rm -f /usr/share/pam-configs/suplinux
    # Regenerate PAM configuration without our module
    if command -v pam-auth-update &> /dev/null; then
        pam-auth-update --package --remove suplinux 2>/dev/null || true
    fi
fi

# Remove ONNX Runtime libraries from SupLinux directory
if [ -d /usr/local/lib/suplinux ]; then
    echo "Removing ONNX Runtime libraries..."
    rm -rf /usr/local/lib/suplinux
fi

# Remove suplinux user if it exists
if id -u suplinux >/dev/null 2>&1; then
    echo "Removing suplinux service user..."
    userdel suplinux
fi

# Ask about removing data
read -p "Remove all user data and configurations? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "Removing data directories..."
    rm -rf /var/lib/suplinux
    rm -rf /etc/suplinux
    rm -rf /usr/share/suplinux
    rm -rf /run/suplinux
else
    echo "Keeping user data and configurations."
    # Just remove the installation manifest
    rm -f /var/lib/suplinux/.installed_files
fi

# PAM configuration is automatically cleaned up by pam-auth-update
echo

echo "Uninstall complete!"