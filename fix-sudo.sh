#!/bin/bash
# Emergency script to restore sudo access
# Run this as root if you're locked out of sudo

echo "Emergency sudo restore script"
echo "============================="

# Create a proper sudo PAM config without face auth
cat > /etc/pam.d/sudo <<'EOF'
#%PAM-1.0
auth       include      system-auth
account    include      system-auth
password   include      system-auth
session    include      system-auth
EOF

echo "✅ Restored default sudo PAM configuration"
echo ""
echo "Now test sudo in a new terminal with: sudo ls"
echo ""
echo "Once sudo works again, you can carefully re-enable face auth by:"
echo "1. Making sure the service is running: systemctl status suplinux"
echo "2. Using the safe config with fallback: cp examples/pam.d/sudo-with-face-safe /etc/pam.d/sudo"