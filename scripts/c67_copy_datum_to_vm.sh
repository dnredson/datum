#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "usage: $0 <ssh-target> [remote-dir]" >&2
  echo "example: $0 dener@192.168.10.103 ~/datum-c67" >&2
  exit 2
fi

SSH_TARGET="$1"
REMOTE_DIR="${2:-~/datum-c67}"

if [ ! -f "DATUM/Cargo.toml" ] || [ ! -f "scripts/c67_run_distributed_node.py" ]; then
  echo "error: run this helper from the datum repository root" >&2
  exit 1
fi

# Copy only the DATUM execution side plus the C67 node runner. This deliberately
# excludes dserver configuration, Firebase material, .git, build output, and
# local experiment artifacts.
tar \
  --exclude='DATUM/target' \
  --exclude='DATUM/.cargo' \
  -czf - \
  DATUM \
  scripts/c67_run_distributed_node.py \
| ssh "$SSH_TARGET" "rm -rf '$REMOTE_DIR' && mkdir -p '$REMOTE_DIR' && tar -xzf - -C '$REMOTE_DIR'"

echo "C67 DATUM payload copied"
echo "  target: $SSH_TARGET"
echo "  remote_dir: $REMOTE_DIR"
echo "Next on the VM:"
echo "  cd $REMOTE_DIR"
echo "  cargo build --manifest-path DATUM/Cargo.toml"
