#!/usr/bin/env python3
import argparse
import json
import subprocess
import tempfile
import urllib.error
import urllib.request
from pathlib import Path


def get_json(url: str) -> dict:
    try:
        with urllib.request.urlopen(url, timeout=10) as response:
            return json.load(response)
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"GET {url} failed with HTTP {exc.code}: {body}") from exc


def post_json(url: str, payload: dict) -> dict:
    body = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.load(response)
    except urllib.error.HTTPError as exc:
        response_body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(
            f"POST {url} failed with HTTP {exc.code}: {response_body}"
        ) from exc


def main() -> None:
    parser = argparse.ArgumentParser(description="Run one C67 node against the local dserver")
    parser.add_argument("--dserver", required=True, help="dserver base URL, e.g. http://192.168.1.10:8080")
    parser.add_argument("--node-id", required=True)
    parser.add_argument("--assignment", required=True)
    parser.add_argument("--arg", action="append", default=[], dest="wasm_args")
    parser.add_argument("--invoked-by", default="c67-live-validation")
    parser.add_argument(
        "--datum-bin",
        help="optional prebuilt DATUM binary; default uses cargo run --manifest-path DATUM/Cargo.toml",
    )
    parser.add_argument("--repo", default=".", help="repository root")
    args = parser.parse_args()

    base = args.dserver.rstrip("/")
    slice_url = f"{base}/api/v1/controlled-wasm/deployment/for-node/{args.node_id}"
    report_url = f"{base}/api/v1/controlled-wasm/live-reports"

    node_view = get_json(slice_url)
    if node_view.get("node_id") != args.node_id:
        raise RuntimeError(
            f"dserver returned node_id={node_view.get('node_id')!r}, expected {args.node_id!r}"
        )
    assignments = node_view.get("assignments", [])
    selected = [item for item in assignments if item.get("assignment_id") == args.assignment]
    if len(selected) != 1:
        raise RuntimeError(
            f"node slice for {args.node_id} does not contain exactly one assignment {args.assignment!r}"
        )
    if any(item.get("target_node") != args.node_id for item in assignments):
        raise RuntimeError("node slice contains an assignment for another node")

    repo = Path(args.repo).resolve()
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".json", prefix=f"c67-{args.node_id}-", delete=False, encoding="utf-8"
    ) as temp:
        json.dump(node_view, temp, indent=2)
        temp.write("\n")
        deployment_path = Path(temp.name)

    try:
        if args.datum_bin:
            command = [str(Path(args.datum_bin).expanduser())]
        else:
            command = [
                "cargo",
                "run",
                "--quiet",
                "--manifest-path",
                str(repo / "DATUM" / "Cargo.toml"),
                "--",
            ]

        command += [
            "wasm-deploy-live",
            "--node-id",
            args.node_id,
            "--deployment",
            str(deployment_path),
            "--assignment",
            args.assignment,
        ]
        for value in args.wasm_args:
            command += ["--arg", value]
        command += [
            "--execute",
            "--invoked-by",
            args.invoked_by,
            "--allow-insecure-http",
            "--json",
        ]

        completed = subprocess.run(
            command,
            cwd=repo,
            text=True,
            capture_output=True,
            check=False,
        )
        if completed.returncode != 0:
            raise RuntimeError(
                "DATUM live execution failed\n"
                f"stdout:\n{completed.stdout}\n"
                f"stderr:\n{completed.stderr}"
            )

        stdout = completed.stdout.strip()
        try:
            report = json.loads(stdout)
        except json.JSONDecodeError as exc:
            raise RuntimeError(
                f"DATUM stdout was not a single JSON report:\n{stdout}\nstderr:\n{completed.stderr}"
            ) from exc

        accepted = post_json(report_url, report)
        print(
            json.dumps(
                {
                    "node_id": args.node_id,
                    "assignment_id": args.assignment,
                    "slice_url": slice_url,
                    "status": report.get("status"),
                    "artifact_content_id": report.get("artifact_content_id"),
                    "results": report.get("results"),
                    "fuel_consumed": report.get("fuel_consumed"),
                    "report_published": accepted.get("execution_id") == report.get("execution_id"),
                    "execution_id": report.get("execution_id"),
                },
                indent=2,
            )
        )
    finally:
        try:
            deployment_path.unlink()
        except FileNotFoundError:
            pass


if __name__ == "__main__":
    main()
