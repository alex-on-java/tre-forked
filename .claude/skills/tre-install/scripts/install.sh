#!/usr/bin/env bash
set -Eeuo pipefail
trap 'rc=$?; cmd=$BASH_COMMAND; pcs=("${PIPESTATUS[@]}"); file="${BASH_SOURCE[1]:-${BASH_SOURCE[0]}}"; line="${BASH_LINENO[0]}"; [[ $line -eq 0 ]] && line="$LINENO"; echo "ERROR $rc at $file:$line: $cmd  PIPESTATUS=${pcs[*]}" >&2' ERR

# Local installation script for tre
# Builds from source and installs to /usr/local/bin and ~/.cargo/bin

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

echo "==> Building tre in release mode..."
cd "$PROJECT_ROOT"
cargo build --release

echo "==> Installing tre..."
cp target/release/tre /usr/local/bin/
cp target/release/tre ~/.cargo/bin/

echo "==> Verifying installation..."
tre --version

echo "==> Installation complete!"
