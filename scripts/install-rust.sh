#!/usr/bin/env bash
set -euo pipefail

# DATUM reusable Rust/Cargo bootstrap for Debian/Ubuntu/Raspberry Pi OS.
#
# Installs the native build prerequisites used by the Rust workspaces and
# ensures a rustup-managed stable Rust toolchain is available for the current
# user. Safe to run more than once.

if [ "${EUID}" -eq 0 ]; then
  SUDO=()
elif command -v sudo >/dev/null 2>&1; then
  SUDO=(sudo)
else
  echo "error: this installer needs root privileges (run as root or install sudo)" >&2
  exit 1
fi

if ! command -v apt-get >/dev/null 2>&1; then
  echo "error: this installer currently supports Debian/Ubuntu/Raspberry Pi OS apt-based systems" >&2
  echo "install a C/C++ build toolchain, pkg-config, OpenSSL development headers, curl, CA certificates, Git, Python 3, tar, and rustup manually" >&2
  exit 2
fi

echo "==> Installing DATUM native build prerequisites"
"${SUDO[@]}" apt-get update
DEBIAN_FRONTEND=noninteractive "${SUDO[@]}" apt-get install -y \
  build-essential \
  pkg-config \
  libssl-dev \
  curl \
  ca-certificates \
  git \
  python3 \
  tar

if ! command -v rustup >/dev/null 2>&1; then
  echo "==> Installing rustup (minimal profile)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal
fi

# rustup updates shell startup files, but make Cargo available to this script
# immediately as well.
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "error: rustup was installed but is not available in this shell" >&2
  echo "try: source \"$HOME/.cargo/env\"" >&2
  exit 3
fi

echo "==> Ensuring the stable Rust toolchain is installed"
rustup toolchain install stable --profile minimal
rustup default stable

echo
echo "DATUM Rust toolchain ready"
echo "  host:  $(hostname)"
echo "  arch:  $(uname -m)"
echo "  rustc: $(rustc --version)"
echo "  cargo: $(cargo --version)"
echo "  git:   $(git --version)"
echo
echo "If cargo is not available in a newly opened shell, run:"
echo "  source \"$HOME/.cargo/env\""
