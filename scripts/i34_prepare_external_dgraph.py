#!/usr/bin/env python3
import argparse
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path

WASM_MAGIC = b"\x00asm"

SPECS = [
    {
        "node_id": "fog-01",
        "node_kind": "fog",
        "module_id": "fog-scale-module",
        "wasm_module_id": "fog-scale-wasm",
        "function_name": "fog_scale",
        "artifact_id": "fog-scale-artifact",
        "file_name": "fog-scale.wasm",
        "assignment_id": "fog-scale",
    },
    {
        "node_id": "local-01",
        "node_kind": "local",
        "module_id": "local-aggregate-module",
        "wasm_module_id": "local-aggregate-wasm",
        "function_name": "local_aggregate",
        "artifact_id": "local-aggregate-artifact",
        "file_name": "local-aggregate.wasm",
        "assignment_id": "local-aggregate",
    },
    {
        "node_id": "cloud-01",
        "node_kind": "cloud",
        "module_id": "cloud-delta-module",
        "wasm_module_id": "cloud-delta-wasm",
        "function_name": "cloud_delta",
        "artifact_id": "cloud-delta-artifact",
        "file_name": "cloud-delta.wasm",
        "assignment_id": "cloud-delta",
    },
]


def digest_wasm(path: Path) -> tuple[str, int]:
    data = path.read_bytes()
    if not data.startswith(WASM_MAGIC):
        raise RuntimeError(f"{path} does not contain a WebAssembly binary magic header")
    return hashlib.sha256(data).hexdigest(), len(data)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Prepare an external I34 D-Graph from independently existing WASM artifacts"
    )
    parser.add_argument("--artifact-host", required=True)
    parser.add_argument("--artifact-port", type=int, default=8090)
    parser.add_argument("--artifacts-dir", default=".c67-live/artifacts")
    parser.add_argument("--out", default=".i34-live/i34-external-dgraph.json")
    args = parser.parse_args()

    artifacts_dir = Path(args.artifacts_dir).resolve()
    output = Path(args.out).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)

    nodes = []
    modules = []
    artifacts = []
    deployment_intents = []
    placements = []
    wasm_modules = []
    summaries = []

    for spec in SPECS:
        artifact_path = artifacts_dir / spec["file_name"]
        if not artifact_path.is_file():
            raise RuntimeError(
                f"missing artifact {artifact_path}; prepare the C67 artifacts first"
            )
        sha256, size_bytes = digest_wasm(artifact_path)
        artifact_uri = (
            f"http://{args.artifact_host}:{args.artifact_port}/artifacts/{spec['file_name']}"
        )

        nodes.append(
            {
                "node_id": spec["node_id"],
                "display_name": spec["node_id"],
                "node_kind": spec["node_kind"],
                "status": "available",
                "labels": ["i34-live", spec["node_kind"]],
                "metadata": {"source": "external-i34-fixture"},
            }
        )
        modules.append(
            {
                "module_id": spec["module_id"],
                "module_kind": "wasm",
                "desired_state": "executed",
                "observed_state": "unknown",
                "runtime": "wasmtime",
                "artifact_id": spec["artifact_id"],
                "metadata": {"source": "external-i34-fixture"},
            }
        )
        artifacts.append(
            {
                "artifact_id": spec["artifact_id"],
                "artifact_kind": "wasm",
                "artifact_ref": artifact_uri,
                "version": "i34-live",
                "metadata": {
                    "sha256": sha256,
                    "content_id": f"sha256:{sha256}",
                    "size_bytes": size_bytes,
                    "source": "external-artifact-repository",
                },
            }
        )
        deployment_intents.append(
            {
                "intent_id": f"intent:{spec['assignment_id']}",
                "target_kind": "module",
                "target_id": spec["module_id"],
                "desired_state": "executed",
                "priority": "normal",
                "metadata": {"target_node": spec["node_id"]},
            }
        )
        placements.append(
            {
                "placement_id": f"placement:{spec['assignment_id']}",
                "module_id": spec["module_id"],
                "selected_node_id": spec["node_id"],
                "selected_runtime_id": None,
                "reason": "external I34 placement decision",
                "status": "selected",
                "metadata": {"source": "external-i34-fixture"},
            }
        )
        wasm_modules.append(
            {
                "module_id": spec["module_id"],
                "wasm_module_id": spec["wasm_module_id"],
                "function_name": spec["function_name"],
                "metadata": {
                    "assignment_id": spec["assignment_id"],
                    "fuel": 1_000_000,
                    "timeout_ms": 1_000,
                },
            }
        )
        summaries.append(
            {
                "node_id": spec["node_id"],
                "module_id": spec["module_id"],
                "artifact_uri": artifact_uri,
                "bytes": size_bytes,
                "content_id": f"sha256:{sha256}",
            }
        )

    graph = {
        "project_id": "i34-live-project",
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "nodes": nodes,
        "runtimes": [],
        "modules": modules,
        "artifacts": artifacts,
        "flows": [],
        "capabilities": [],
        "health_reports": [],
        "deployment_intents": deployment_intents,
        "placements": placements,
        "reconfigurations": [],
        "wasm_modules": wasm_modules,
        "wasm_reports": [],
        "relationships": [],
        "metadata": {
            "dgraph_id": "i34-external-dgraph",
            "producer": "external-i34-fixture",
            "checkpoint": "I34/140",
            "note": "This source graph is created independently of the derived Deployment Manifest.",
        },
    }

    output.write_text(json.dumps(graph, indent=2) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "status": "prepared",
                "dgraph": str(output),
                "project_id": graph["project_id"],
                "dgraph_id": graph["metadata"]["dgraph_id"],
                "artifacts": summaries,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
