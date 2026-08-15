#!/usr/bin/env python3
import argparse
import hashlib
import json
from pathlib import Path

ADD_WASM_HEX = (
    "0061736d01000000"
    "01070160027f7f017f"
    "03020100"
    "070701036164640000"
    "0a09010700200020016a0b"
)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Prepare the reproducible C66 single-node WASM live-validation fixture."
    )
    parser.add_argument("--node-id", default="fog-01")
    parser.add_argument("--project-id", default="c66-live-project")
    parser.add_argument("--out-dir", default=".c66-live")
    args = parser.parse_args()

    node_id = args.node_id.strip()
    if not node_id:
        raise SystemExit("--node-id must not be empty")

    out_dir = Path(args.out_dir).expanduser().resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    wasm_bytes = bytes.fromhex(ADD_WASM_HEX)
    wasm_path = out_dir / "c66-add.wasm"
    wasm_path.write_bytes(wasm_bytes)
    digest = hashlib.sha256(wasm_bytes).hexdigest()

    deployment = {
        "manifest_version": "0.1.0",
        "manifest_id": "c66-live-single-node",
        "project_id": args.project_id,
        "node_id": node_id,
        "delivery_scope": "node_slice",
        "assignments": [
            {
                "assignment_id": "c66-add-assignment",
                "module_id": "c66-add-module",
                "function_name": "add",
                "artifact": {
                    "artifact_id": "c66-add-artifact",
                    "artifact_uri": wasm_path.as_uri(),
                    "sha256": digest,
                    "content_id": f"sha256:{digest}",
                },
                "target_node": node_id,
                "limits": {"fuel": 1000000, "timeout_ms": 1000},
            }
        ],
        "execution_boundary": {
            "explicit_invocation_required": True,
            "automatic_execution_allowed": False,
            "governed_execution_authorized": False,
        },
    }

    deployment_path = out_dir / "c66-node-deployment.json"
    deployment_path.write_text(json.dumps(deployment, indent=2) + "\n", encoding="utf-8")

    print(f"artifact={wasm_path}")
    print(f"sha256={digest}")
    print(f"deployment={deployment_path}")
    print()
    print("Run:")
    print(
        "cargo run --manifest-path DATUM/Cargo.toml -- "
        "wasm-deploy-live "
        f"--node-id {node_id} "
        f"--deployment {deployment_path} "
        "--assignment c66-add-assignment "
        "--arg i32:2 --arg i32:3 "
        "--execute --invoked-by c66-live-validation --json"
    )


if __name__ == "__main__":
    main()
