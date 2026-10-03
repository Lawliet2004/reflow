#!/usr/bin/env bash
# The shared helper runs Tauri once from the checkout root and checks real installers.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec node "$SCRIPT_DIR/package.cjs" --platform linux "$@"
