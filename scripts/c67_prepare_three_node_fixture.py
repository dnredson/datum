#!/usr/bin/env python3
import argparse
import hashlib
import json
from pathlib import Path


def uleb(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def section(section_id: int, payload: bytes) -> bytes:
    return bytes([section_id]) + uleb(len(payload)) + payload


def binary_i32_module(export_name: str, opcode: int) -> bytes:
    name = export_name.encode("utf-8")
    module = bytearray(b"\x00asm\x01\x00\x00\x00")

    type_payload = bytes([0x01, 0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F])
    module += section(1, type_payload)

    function_payload = bytes([0x01, 0x00])
    module += section(3, function_payload)

    export_payload = bytes([0x01]) + uleb(len(name)) + name + bytes([0x00, 0x00])
    module += section(7, export_payload)

    body = bytes([0x00, 0x20, 0x00, 0x20, 0x01, opcode, 0x0B])
    code_payload = bytes([0x01]) + uleb(len(body)) + body
    module += section(10, code_payload)
    return bytes(module)


def artifact_entry(base_url: str, file_name: str, artifact_id: str, data: bytes) -> dict:
    digest = hashlib.sha256(data).hexdigest()
    return {
        "artifact_id": artifact_id,
        "artifact_uri": f"{base_url}/artifacts/{file_name}",
        "sha256": digest,
        "content_id": f"sha256:{digest}",
    }


def assignment(
    assignment_id: str,
    module_id: str,
    function_name: str,
    artifact: dict,
    target_node: str,
    role: str,
) -> dict:
    return {
        "assignment_id": assignment_id,
        "module_id": module_id,
        "function_name": function_name,
        "artifact": artifact,
        "target_node": target_node,
        "limits": {"fuel": 1_000_000, "timeout_ms": 1_000},
        "metadata": {"c67_role": role},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Prepare C67 three-node distributed WASM fixture")
    parser.add_argument(
        "--artifact-host",
        required=True,
        help="IP or hostname of the local machine reachable from both VMs",
    )
    parser.add_argument("--artifact-port", type=int, default=8090)
    parser.add_argument("--dserver-port", type=int, default=8080)
    parser.add_argument("--out", default=".c67-live")
    args = parser.parse_args()

    out = Path(args.out).resolve()
    artifacts_dir = out / "artifacts"
    artifacts_dir.mkdir(parents=True, exist_ok=True)

    specs = [
        {
            "file": "fog-scale.wasm",
            "function": "fog_scale",
            "opcode": 0x6C,  # i32.mul
            "assignment": "fog-scale",
            "module": "fog-scale-module",
            "artifact": "fog-scale-artifact",
            "node": "fog-01",
            "role": "fog preprocessing: scale a sensor reading",
        },
        {
            "file": "local-aggregate.wasm",
            "function": "local_aggregate",
            "opcode": 0x6A,  # i32.add
            "assignment": "local-aggregate",
            "module": "local-aggregate-module",
            "artifact": "local-aggregate-artifact",
            "node": "local-01",
            "role": "local aggregation: combine the fog output with a second value",
        },
        {
            "file": "cloud-delta.wasm",
            "function": "cloud_delta",
            "opcode": 0x6B,  # i32.sub
            "assignment": "cloud-delta",
            "module": "cloud-delta-module",
            "artifact": "cloud-delta-artifact",
            "node": "cloud-01",
            "role": "cloud analysis: calculate a delta against a reference",
        },
    ]

    base_url = f"http://{args.artifact_host}:{args.artifact_port}"
    assignments = []
    artifact_summaries = []
    for spec in specs:
        data = binary_i32_module(spec["function"], spec["opcode"])
        path = artifacts_dir / spec["file"]
        path.write_bytes(data)
        artifact = artifact_entry(base_url, spec["file"], spec["artifact"], data)
        assignments.append(
            assignment(
                spec["assignment"],
                spec["module"],
                spec["function"],
                artifact,
                spec["node"],
                spec["role"],
            )
        )
        artifact_summaries.append(
            {
                "node": spec["node"],
                "assignment": spec["assignment"],
                "file": str(path),
                "bytes": len(data),
                "content_id": artifact["content_id"],
            }
        )

    manifest = {
        "manifest_version": "0.1.0",
        "manifest_id": "c67-live-three-node",
        "project_id": "c67-live-project",
        "strategy": "distributed",
        "graph_distribution": "node_slice",
        "source_dgraph_id": "c67-live-dgraph",
        "assignments": assignments,
        "execution_boundary": {
            "explicit_invocation_required": True,
            "automatic_execution_allowed": False,
            "governed_execution_authorized": False,
        },
        "metadata": {
            "checkpoint": "C67/150",
            "topology": ["local-01", "fog-01", "cloud-01"],
            "purpose": "live distributed controlled WASM deployment",
        },
    }

    manifest_path = out / "c67-deployment-manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

    dserver = f"http://{args.artifact_host}:{args.dserver_port}"
    print(json.dumps({"manifest": str(manifest_path), "artifacts": artifact_summaries}, indent=2))
    print()
    print("Artifact repository:")
    print(f"  python3 -m http.server {args.artifact_port} --bind 0.0.0.0 --directory {out}")
    print()
    print("Publish the global manifest to dserver:")
    print(
        f"  curl -sS -X POST {dserver}/api/v1/controlled-wasm/deployment-manifest "
        f"-H 'Content-Type: application/json' --data-binary '@{manifest_path}'"
    )
    print()
    print("Node slices:")
    for node in ("local-01", "fog-01", "cloud-01"):
        print(f"  curl -sS {dserver}/api/v1/controlled-wasm/deployment/for-node/{node}")


if __name__ == "__main__":
    main()
