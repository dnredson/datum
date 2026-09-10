"""Materialize a node bundle locally; never contact Docker or the control plane."""
import argparse
import json
import os
from pathlib import Path
import re
import secrets
import shlex

ROOT = Path(__file__).resolve().parent
NODES = {
    "fog-01": ("fog", ["mosquitto", "chirpstack-postgres", "chirpstack-redis", "chirpstack", "chirpstack-provisioner"]),
    "mist-01": ("mist", ["chirpstack-gateway-bridge"]),
}


def prepare(node, destination):
    stage, ids = NODES[node]
    destination = Path(destination).expanduser().absolute()
    # Refuse symlinks and repository destinations, including untracked directories.
    if any(p.is_symlink() for p in [destination, *destination.parents]):
        raise ValueError("destination must not traverse symlinks")
    destination = destination.resolve()
    if destination.is_relative_to(ROOT.parent):
        raise ValueError("generated bundles and secrets must stay outside the repository")
    files = {}
    secret_names = {"fog-01": ["POSTGRES_PASSWORD", "CHIRPSTACK_API_SECRET"], "mist-01": []}[node]
    for artifact_id in ids:
        document = json.loads((ROOT / "catalog" / f"{artifact_id}.json").read_text())
        files[f"catalog/{artifact_id}.json"] = (json.dumps(document, indent=2) + "\n").encode()
        for mount in document["runtime"]["container"].get("configuration_mounts", []):
            source = (ROOT / "config" / mount["source"]).resolve()
            if not source.is_relative_to(ROOT / "config") or not source.exists():
                raise ValueError("missing or unsafe configuration source")
            for item in ([source] if source.is_file() else sorted(source.rglob("*"))):
                if item.is_file():
                    files["config/" + item.relative_to(ROOT / "config").as_posix()] = item.read_bytes()
    profile = (ROOT.parent / "DATUM" / f"config.{stage}.linux").read_text()
    profile = profile.replace('"/opt/datum/governed-config"', json.dumps((destination / "config").as_posix()))
    files["agent.toml"] = profile.encode()
    files["manifest.json"] = (json.dumps({"node_id": node, "artifacts_in_dependency_order": ids,
        "secret_environment_names": secret_names, "deployment_performed": False}, indent=2) + "\n").encode()
    # Shell paths are quoted; secrets are hex and therefore safe to source.
    runner = '#!/bin/sh\nset -eu\ncd ' + shlex.quote(destination.as_posix()) + '\n'
    runner += 'set -a\n. ./node.env\nset +a\n'
    runner += 'exec "${DATUM_RECONCILER:?set the absolute SmartSentinel binary path}" --config ./agent.toml '
    runner += '--dserver-url "${DSERVER_URL:?set DSERVER_URL}" --project-id "${PROJECT_ID:?set PROJECT_ID}" '
    runner += '--node-id ' + shlex.quote(node) + ' "$@"\n'
    files["run-reconcile.sh"] = runner.encode()
    env_path = destination / "node.env"
    # Check every path before reading an existing secret or writing any content.
    for relative in [*files, "node.env"]:
        target = destination / relative
        if any(p.is_symlink() for p in [target, *target.parents]):
            raise ValueError("bundle paths must not traverse symlinks")
        if target.exists() and not target.is_file():
            raise ValueError(f"expected file: {relative}")
        if relative in files and target.exists() and target.read_bytes() != files[relative]:
            raise ValueError(f"refusing to overwrite changed file: {relative}")
    if env_path.exists():
        lines = env_path.read_text().splitlines()
        expected = {name for name in secret_names}
        if len(lines) != len(expected) or {line.partition("=")[0] for line in lines} != expected:
            raise ValueError("existing node.env has unexpected keys; preserve it and resolve manually")
        if any(not re.fullmatch(r"[A-Z_]+=[0-9a-f]{64}", line) for line in lines):
            raise ValueError("existing node.env must contain generated hex secrets only")
    else:
        files["node.env"] = ''.join(f'{name}={secrets.token_hex(32)}\n' for name in secret_names).encode()
    destination.mkdir(parents=True, exist_ok=True, mode=0o700)
    for relative, content in files.items():
        path = destination / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600 if relative == "node.env" else 0o644)
            with os.fdopen(fd, "wb") as out:
                out.write(content)
    if os.name == "posix":
        env_path.chmod(0o600)
        (destination / "run-reconcile.sh").chmod(0o700)
    return {"node_id": node, "destination": str(destination), "artifacts": ids, "deployment_performed": False}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--node', required=True, choices=NODES)
    parser.add_argument('--destination', required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.node, args.destination), indent=2))
