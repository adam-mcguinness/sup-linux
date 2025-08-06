# Linux Face Authentication Implementation Plan

## Overview
This document outlines the phased approach to implementing a secure face authentication system for Linux using PAM integration.

## Phase 1: Local Development Mode ✅ COMPLETED

### 1.1 Basic Functionality
- [x] Camera capture with V4L2
- [x] Face detection with ONNX models
- [x] Face recognition with embeddings
- [x] Basic enrollment and authentication
- [x] Configuration system

### 1.2 Development Testing
- [x] Add `--dev` flag for development mode
- [x] Save all data to `./dev_data/` directory
- [x] Enhanced debug logging
- [x] Model quantization (INT8) for faster inference
- [x] Performance profiling and optimization
- [x] Camera device auto-detection
- [x] Test data visualization tools
- [x] Rolling buffer authentication testing
- [x] Enhanced enrollment quality metrics

### 1.3 Initial Security (Deferred to Phase 2)
- [ ] Basic embedding encryption (for testing)
- [ ] File permission checks
- [ ] Input validation

### 1.4 Advanced Authentication Features ✅ COMPLETED
- [x] K-of-N matching implementation
- [x] Rolling buffer for embedding fusion during auth
- [x] Enhanced enrollment with embedding averaging
- [x] Quality-based enrollment selection
- [x] INT8 model quantization for performance

### 1.9 Minimal Viable Product (MVP) ✅ COMPLETED
- [x] Add --system flag to use system paths
- [x] Create paths module for system vs dev mode
- [x] Update all components to support custom paths
- [x] PAM-compatible authentication
  - [x] Read PAM_USER environment variable
  - [x] Handle missing camera gracefully
  - [x] Timeout for remote sessions
  - [x] Proper exit codes for PAM
- [x] PAM wrapper script for environment setup
- [x] Example PAM configurations
  - [x] sudo authentication
  - [x] GDM/GNOME login
  - [x] SDDM/KDE login  
  - [x] Console login
- [x] User enrollment without root
  - [x] Store in ~/.local/share/linuxsup for regular users
  - [x] System auth checks both system and user directories
- [x] Install/uninstall scripts with PAM examples
- [x] Basic logging to stderr (syslog-compatible)

## Phase 2: Security Hardening ← NEXT PHASE

### 2.1 Encryption Implementation
- [ ] AES-256-GCM for embedding storage
- [ ] PBKDF2 key derivation
- [ ] Secure key storage in kernel keyring
- [ ] Memory zeroing for sensitive data

### 2.2 Anti-Spoofing
- [ ] Liveness detection with IR camera
- [x] Multiple angle enrollment (minimum 3)
- [x] K-of-N matching: Require K successful matches out of N attempts
  - [x] Sliding window implementation
  - [x] Rolling embedding buffer for improved accuracy
  - [x] Progress feedback during authentication
- [x] Configurable K and N values in TOML config
- [x] Embedding fusion during authentication
  - [x] Maintain buffer of last M embeddings
  - [x] Compare both individual and averaged embeddings
- [ ] Confidence threshold tuning per user
- [ ] Attack detection and logging
- [ ] Time-based challenge variations

### 2.3 Access Control
- [ ] User permission verification
- [ ] Rate limiting for attempts (partially addressed in Phase 3.3)
- [ ] Temporary lockout mechanism
- [ ] Audit logging (foundation laid with systemd journal)

## Phase 3: PAM Integration ← IN PROGRESS

### 3.1 PAM Module Development ✅ MOSTLY COMPLETE
- [x] Create `pam_linuxsup` module in Rust using `pamsm` crate
  - [x] Implement challenge-response authentication
  - [x] Generate random challenges in PAM module
  - [x] Verify embedding signatures (SHA256 - upgrade to HMAC pending)
  - [x] Perform similarity comparison in PAM module
- [ ] Implement PAM conversation functions for user feedback
- [x] Unix socket communication with embedding service
- [x] Proper PAM return codes (SUCCESS, AUTH_ERR, SERVICE_ERR)

### 3.2 Service Architecture (Secure Design) ✅ COMPLETED
- [x] Create embedding service (unprivileged)
  - [x] Camera access and face detection only
  - [x] Return embeddings with challenge signature
  - [x] No authentication decisions
  - [x] No access to stored user data
- [x] Unix socket at `/run/linuxsup/embedding.sock`
- [x] Service runs as dedicated `linuxsup` user
- [x] Strict socket permissions (0600)
- [x] systemd service unit with security restrictions

### 3.3 Security Protocol ✅ MOSTLY COMPLETE
- [x] Challenge-Response Implementation:
  ```
  PAM → Generate Challenge → Service
  Service → Capture Face → Generate Embedding → Sign(Embedding + Challenge)
  PAM ← Embedding + Signature ← Service
  PAM → Verify Signature → Compare Embeddings → Auth Decision
  ```
- [x] Embedding storage access only in PAM module
- [x] Time-bound challenges (5 second validity)
- [ ] Rate limiting in PAM module (pending)

### 3.4 Configuration Integration ✅ MOSTLY COMPLETE
- [x] Native module available (can replace pam_exec)
- [x] Update PAM configuration examples (both native and pam_exec)
- [ ] Full integration testing with common PAM stacks
- [x] Fallback authentication flows (maintains pam_exec compatibility)

### 3.5 Implementation Notes
- **Architecture Decision**: Separate crate (`pam_module/`) for PAM module due to `pamsm` macro requirements
- **Security Status**:
  - ✅ Privilege separation achieved through Unix socket IPC
  - ✅ Service has no access to user data or auth decisions
  - ⚠️ Currently using SHA256 for signatures (should upgrade to HMAC with shared secret)
  - ⚠️ Rate limiting not yet implemented (from Phase 2.3)
- **Dual-Mode Operation**: System supports both pam_exec (legacy) and native module for smooth transition
- **Files Created**:
  - `pam_module/` - Separate Rust crate for PAM module
  - `src/bin/embedding_service.rs` - Unprivileged embedding service
  - `systemd/linuxsup-embedding.service` - systemd service configuration

## Phase 4: System Integration

### 4.1 Desktop Environment Support
- [ ] GDM integration for GNOME
- [ ] SDDM integration for KDE
- [ ] LightDM support
- [ ] Screen lock integration

### 4.2 Security Policies
- [ ] SELinux policy module
- [ ] AppArmor profile
- [ ] Polkit rules for GUI auth
- [ ] Secure installation scripts

### 4.3 User Experience
- [ ] GUI enrollment tool
- [ ] Status indicator applet
- [ ] Configuration GUI
- [ ] Troubleshooting tools

## Phase 5: Production Readiness

### 5.1 Performance Optimization
- [x] Model optimization for speed
  - [x] INT8 quantization support
  - [x] ONNX optimization level 3
  - [x] Streaming camera session
- [ ] Caching strategies
  - [ ] Embedding cache for rolling buffer
  - [ ] Model warmup on startup
- [ ] Resource usage limits
- [ ] Background processing

### 5.2 Reliability
- [ ] Comprehensive error handling
- [ ] Graceful degradation
- [ ] Automatic recovery
- [ ] Health monitoring

### 5.3 Distribution
- [ ] Package for major distributions (deb, rpm, AUR)
- [ ] Installation documentation
- [ ] Migration tools
- [ ] Uninstall procedures

## Security Considerations

### Always Remember:
1. **Face authentication is NEVER the sole authentication method**
2. **Biometric data cannot be changed if compromised**
3. **Regular security audits are mandatory**
4. **User privacy must be protected**

### Enhanced Security Features:
1. **K-of-N Matching Strategy**
   - Require K successful face matches out of N consecutive attempts
   - Prevents single lucky match from granting access
   - Example: K=2, N=3 means 2 out of 3 attempts must succeed
   - Configurable per deployment or per user

2. **Adaptive Thresholds**
   - Base similarity threshold in config
   - Per-user threshold adjustments based on enrollment quality
   - Temporal threshold adjustments (stricter at night)
   - Environmental adjustments (IR illumination quality)

3. **Embedding Fusion and Quality Enhancement**
   - Enrollment improvements:
     - Store both individual and averaged embeddings
     - Quality metrics for each enrollment capture
     - Weighted averaging based on quality scores
   - Authentication improvements:
     - Rolling buffer of recent embeddings
     - Dynamic embedding fusion during auth
     - Compare against both stored individuals and average

4. **Performance Optimizations**
   - INT8 model quantization for 2-4x speedup
   - Optimized image preprocessing pipeline
   - Persistent camera streaming session
   - ~16-20 FPS authentication capability

### Configuration Hierarchy:
1. System-wide: `/etc/linuxsup/config.toml`
2. User-specific: `~/.config/linuxsup/config.toml`
3. Development: `./dev_data/config.toml`

### Example Security Configuration:
```toml
[auth]
# Base similarity threshold
similarity_threshold = 0.6

# K-of-N matching
k_required_matches = 2      # Require 2 successful matches
n_total_attempts = 3        # Out of 3 total attempts

# Rolling buffer for embedding fusion
embedding_buffer_size = 3   # Keep last 3 embeddings for averaging
use_embedding_fusion = true # Enable dynamic embedding fusion

# Timeouts and limits
timeout_seconds = 5         # Total session timeout
lost_face_timeout = 3       # Timeout when face not detected
lockout_duration = 300      # 5 minute lockout after failure

# Enhanced security
require_blink_detection = true
min_face_size = 80         # Minimum face size in pixels
max_face_distance = 1.5    # Maximum distance factor from enrollment

[performance]
# Model optimization
enable_quantization = true  # Use INT8 quantization
optimization_level = 3      # ONNX optimization level

[enrollment]
# Enhanced enrollment
store_averaged_embedding = true
capture_quality_metrics = true
min_enrollment_quality = 0.7

[security]
# Anti-spoofing
require_ir_camera = true
check_image_quality = true
min_image_brightness = 30
max_image_brightness = 220

# Logging
log_failed_attempts = true
log_successful_auth = true
audit_file = "/var/log/linuxsup/audit.log"
```

## Testing Checklist

### Before Each Phase:
- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Security scan clean
- [ ] Documentation updated

### Development Commands:
```bash
# Run in development mode
cargo run -- --dev test-camera

# Test face detection
cargo run -- --dev test-detection

# Enroll user (dev mode)
cargo run -- --dev enroll --username testuser

# Test authentication (dev mode)
cargo run -- --dev test --username testuser
```

## Success Criteria

### Phase 1 Complete When:
- Camera reliably captures frames at 15+ FPS
- Face detection works consistently with INT8 quantization
- Enrollment stores both individual and averaged embeddings
- Authentication uses K-of-N matching with rolling buffer
- Performance: ~50ms per detection, 16+ attempts per second
- All data stays in project directory

### Final Success:
- Seamless integration with Linux desktop
- Sub-second authentication time
- Zero security vulnerabilities
- Positive user feedback
- Active community adoption

## Development Notes for Next Session

### Current Status (Phase 3 - PAM Integration In Progress)
- ✅ All Phase 1 features implemented and tested
- ✅ K-of-N authentication working successfully (tested by user)
- ✅ Basic PAM integration complete (pam_exec MVP)
- ✅ Native PAM module implemented with challenge-response
- ✅ Privilege separation architecture completed
- ✅ System can handle both root and non-root user enrollment
- ✅ Performance optimized: ~50ms per detection, 16+ FPS
- 🔄 Dual-mode operation: pam_exec (stable) + native module (testing)

### Key Implementation Details
1. **Authentication Flow**: 
   - Uses rolling buffer with K-of-N matching
   - Compares against both individual and averaged embeddings
   - 5-second timeout with face loss detection

2. **Data Storage**:
   - System mode: `/var/lib/linuxsup/` (requires root)
   - User mode: `~/.local/share/linuxsup/` 
   - Development mode: `./dev_data/`
   - Authentication checks both system and user stores

3. **PAM Integration**:
   - **Current (MVP)**: Uses pam_exec with wrapper script
     - Binary: `linuxsup-auth` reads PAM_USER env var
     - Wrapper script: `linuxsup-pam-wrapper` sets up environment
     - Skips face auth for SSH/remote sessions
     - Example configs for sudo, GDM, SDDM included
   - **Target (Phase 3)**: Native PAM module with challenge-response
     - `pam_linuxsup.so` replaces pam_exec approach
     - Embedding service provides faces → embeddings only
     - PAM module handles all authentication decisions
     - Enhanced security through privilege separation

4. **Performance Notes**:
   - INT8 quantization enabled by default
   - Persistent camera streaming session implemented
   - Remove old 100ms sleep between attempts
   - Avoid creating/destroying V4L2 streams per frame

### Known Issues Fixed
- ✅ Serialization issue with UserData (removed skip_serializing_if)
- ✅ Channel mismatch (YOLOv8 needs 3ch, ArcFace needs 1ch)
- ✅ Performance bottlenecks resolved

### Next Steps (Complete Phase 3 & Begin Phase 2)
Phase 3 Remaining:
- [ ] Implement rate limiting in PAM module
- [ ] Add PAM conversation functions for user feedback
- [ ] Full integration testing with GDM/SDDM/LightDM
- [ ] Upgrade from SHA256 to HMAC for signatures

Phase 2 Security (Priority Items):
- [ ] Implement AES-256-GCM encryption for stored embeddings
- [ ] Add secure key storage (kernel keyring or TPM)
- [ ] Complete audit logging implementation
- [ ] Add lockout mechanism after failed attempts

### Testing Commands
```bash
# Install system-wide (requires root)
sudo ./install.sh

# Start embedding service (for native PAM module)
sudo systemctl start linuxsup-embedding
sudo systemctl enable linuxsup-embedding

# Test enrollment (as regular user)
linuxsup enroll --username $USER

# Test authentication
linuxsup test --username $USER

# Test with sudo - Option 1: Native PAM module (recommended)
sudo cp examples/pam.d/sudo-with-face-native /etc/pam.d/sudo
sudo -k  # Clear sudo cache
sudo ls  # Should trigger face auth

# Test with sudo - Option 2: pam_exec fallback
sudo cp examples/pam.d/sudo-with-face /etc/pam.d/sudo
sudo -k  # Clear sudo cache
sudo ls  # Should trigger face auth

# Check service logs
sudo journalctl -u linuxsup-embedding -f

# Uninstall
sudo ./uninstall.sh
```