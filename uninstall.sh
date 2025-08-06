#!/bin/bash
set -e

echo "LinuxSup Face Authentication - Uninstall Script"
echo "=============================================="
echo
read -p "This will remove LinuxSup from your system. Continue? (y/N) " -n 1 -r
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

echo "Removing LinuxSup..."

# Stop and disable systemd service if running
if systemctl is-active --quiet linuxsup-embedding; then
    echo "Stopping embedding service..."
    systemctl stop linuxsup-embedding
fi

if systemctl is-enabled --quiet linuxsup-embedding 2>/dev/null; then
    echo "Disabling embedding service..."
    systemctl disable linuxsup-embedding
fi

# Remove systemd service file
if [ -f /etc/systemd/system/linuxsup-embedding.service ]; then
    echo "Removing systemd service..."
    rm -f /etc/systemd/system/linuxsup-embedding.service
    systemctl daemon-reload
fi

# Remove binaries
echo "Removing binaries..."
rm -f /usr/local/bin/linuxsup
rm -f /usr/local/bin/linuxsup-auth
rm -f /usr/local/bin/linuxsup-embedding-service
rm -f /usr/local/bin/linuxsup-pam-wrapper

# Remove PAM module if it exists
if [ -f /lib/security/pam_linuxsup.so ]; then
    echo "Removing PAM module..."
    rm -f /lib/security/pam_linuxsup.so
fi

# Remove linuxsup user if it exists
if id -u linuxsup >/dev/null 2>&1; then
    echo "Removing linuxsup service user..."
    userdel linuxsup
fi

# Ask about removing data
read -p "Remove all user data and configurations? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "Removing data directories..."
    rm -rf /var/lib/linuxsup
    rm -rf /etc/linuxsup
    rm -rf /usr/share/linuxsup
    rm -rf /run/linuxsup
else
    echo "Keeping user data and configurations."
    # Just remove the installation manifest
    rm -f /var/lib/linuxsup/.installed_files
fi

# Check if PAM was modified
echo
echo "⚠️  IMPORTANT: Check your PAM configuration!"
echo "   If you modified /etc/pam.d/sudo or other PAM files,"
echo "   you must manually remove the LinuxSup entries."
echo
echo "   Look for lines containing:"
echo "   - pam_linuxsup.so"
echo "   - pam_exec.so.*linuxsup-pam-wrapper"
echo

echo "Uninstall complete!"