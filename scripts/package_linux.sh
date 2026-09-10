#!/usr/bin/env bash
# Reflow Linux Packaging Script (Debian / Ubuntu / Fedora / Arch)
# Pre-flight check, platform dependency inspection, frontend compilation, and Tauri bundle generation.

set -euo pipefail

echo "========================================="
echo " Reflow Linux Packaging Script "
echo "========================================="

# 1. Environment pre-flight
echo -e "\n[1/5] Checking build environment..."

command -v node >/dev/null 2>&1 || { echo "Node.js is required but not installed."; exit 1; }
echo "  ? Node.js detected: $(node --version)"

command -v cargo >/dev/null 2>&1 || { echo "Cargo/Rust is required but not installed."; exit 1; }
echo "  ? Cargo detected: $(cargo --version)"

command -v python3 >/dev/null 2>&1 || { echo "Python 3 is required but not installed."; exit 1; }
echo "  ? Python 3 detected: $(python3 --version)"

# Check model runtime script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNTIME_SCRIPT="$SCRIPT_DIR/../model-runtime/qwen3_asr_runtime.py"
if [ ! -f "$RUNTIME_SCRIPT" ]; then
    echo "Error: Runtime script $RUNTIME_SCRIPT not found!"
    exit 1
fi
echo "  ? Runtime script found: $RUNTIME_SCRIPT"

# 2. Check Linux library dependencies (Debian/Ubuntu hint)
echo -e "\n[2/5] Checking required platform libraries..."
PKGS=("libwebkit2gtk-4.1-dev" "libayatana-appindicator3-dev" "librsvg2-dev" "libasound2-dev" "libxdo-dev")
if command -v dpkg-query >/dev/null 2>&1; then
    for pkg in "${PKGS[@]}"; do
        if dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q "install ok installed"; then
            echo "  ? $pkg is installed"
        else
            echo "  ! $pkg not detected via dpkg (install with: sudo apt-get install $pkg)"
        fi
    done
fi

# 3. Frontend Build
echo -e "\n[3/5] Building frontend application bundle..."
cd "$SCRIPT_DIR/.."
npm run build
echo "  ? Frontend assets generated in dist/"

# 4. Cargo Tauri Release Compilation
echo -e "\n[4/5] Compiling Tauri release binary..."
cd "$SCRIPT_DIR/../src-tauri"
cargo build --release
echo "  ? Release binary compiled: target/release/reflow"

# 5. Checksum Generation
echo -e "\n[5/5] Generating SHA-256 Checksums..."
if [ -f "target/release/reflow" ]; then
    sha256sum target/release/reflow > target/release/reflow.sha256
    echo "  ? SHA-256 Checksum saved to target/release/reflow.sha256"
fi

echo -e "\n========================================="
echo " Linux Packaging Completed Successfully! "
echo "========================================="
