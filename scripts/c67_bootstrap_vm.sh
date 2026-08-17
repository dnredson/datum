#!/usr/bin/env bash
set -euo pipefail

if ! command -v sudo >/dev/null 2>&1; then
  echo "error: sudo is required for the C67 VM bootstrap" >&2
  exit 1
fi

if command -v apt-get >/dev/null 2>&1; then
  sudo apt-get update
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    curl \
    ca-certificates \
    python3 \
    tar
else
  echo "error: this helper currently supports Debian/Ubuntu apt-based VMs only" >&2
  echo "install a C/C++ build toolchain, pkg-config, OpenSSL development headers, curl, CA certificates, Python 3, tar, and Rust manually" >&2
  exit 2
fi

if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi

# rustup modifies shell startup files, but this script must also see Cargo immediately.
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

rustup default stable

echo "C67 VM bootstrap ready"
echo "  host: $(hostname)"
echo "  kernel: $(uname -srmo)"
echo "  rustc: $(rustc --version)"
echo "  cargo: $(cargo --version)"
echo "  python: $(python3 --version)"
