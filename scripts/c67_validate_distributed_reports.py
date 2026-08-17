#!/usr/bin/env python3
import argparse
import json
import urllib.error
import urllib.request


EXPECTED = {
    "fog-01": {"assignment": "fog-scale", "result": 24},
    "local-01": {"assignment": "local-aggregate", "result": 30},
    "cloud-01": {"assignment": "cloud-delta", "result": 20},
}


def get_json(url: str) -> dict:
    try:
        with urllib.request.urlopen(url, timeout=10) as response:
            return json.load(response)
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"GET {url} failed with HTTP {exc.code}: {body}") from exc


def scalar_i32(report: dict):
    results = report.get("results", [])
    if len(results) != 1:
        return None
    result = results[0]
    if result.get("type") != "i32":
        return None
    return result.get("value")


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate C67 three-node live evidence")
    parser.add_argument("--dserver", required=True)
    args = parser.parse_args()

    url = args.dserver.rstrip("/") + "/api/v1/controlled-wasm/live-reports"
    payload = get_json(url)
    reports = payload.get("reports", [])

    errors = []
    if payload.get("manifest_id") != "c67-live-three-node":
        errors.append(f"unexpected manifest_id: {payload.get('manifest_id')!r}")
    if payload.get("project_id") != "c67-live-project":
        errors.append(f"unexpected project_id: {payload.get('project_id')!r}")
    if payload.get("count") != 3 or len(reports) != 3:
        errors.append(f"expected exactly 3 reports, received count={payload.get('count')} len={len(reports)}")

    by_node = {report.get("node_id"): report for report in reports}
    if set(by_node) != set(EXPECTED):
        errors.append(f"expected nodes {sorted(EXPECTED)}, received {sorted(str(x) for x in by_node)}")

    content_ids = {report.get("artifact_content_id") for report in reports}
    if len(content_ids) != 3 or None in content_ids:
        errors.append("expected three distinct non-empty artifact content IDs")

    execution_ids = {report.get("execution_id") for report in reports}
    if len(execution_ids) != 3 or None in execution_ids:
        errors.append("expected three distinct non-empty execution IDs")

    for node_id, expected in EXPECTED.items():
        report = by_node.get(node_id)
        if not report:
            continue
        if report.get("assignment_id") != expected["assignment"]:
            errors.append(
                f"{node_id}: expected assignment {expected['assignment']!r}, received {report.get('assignment_id')!r}"
            )
        if report.get("status") != "executed":
            errors.append(f"{node_id}: status is not executed")
        if scalar_i32(report) != expected["result"]:
            errors.append(
                f"{node_id}: expected i32 result {expected['result']}, received {report.get('results')!r}"
            )
        for field in (
            "integrity_verified",
            "wasmtime_compiled",
            "wasmtime_instantiated",
            "guest_execution_performed",
        ):
            if report.get(field) is not True:
                errors.append(f"{node_id}: {field} is not true")
        for field in (
            "wasm_guest_side_effects",
            "governed_execution_performed",
            "host_mutation_performed",
            "safe_to_auto_execute",
            "timeout_enforced",
        ):
            if report.get(field) is not False:
                errors.append(f"{node_id}: {field} is not false")
        acquisition = report.get("acquisition", {})
        if acquisition.get("artifact_delivery_performed") is not True:
            errors.append(f"{node_id}: network artifact delivery was not recorded")
        if acquisition.get("host_network_fetch_performed") is not True:
            errors.append(f"{node_id}: host network artifact fetch was not recorded")
        if acquisition.get("persistent_write_performed") is not False:
            errors.append(f"{node_id}: artifact persistent write must remain false")

    if errors:
        print(json.dumps({"status": "FAIL", "errors": errors, "payload": payload}, indent=2))
        raise SystemExit(1)

    print(
        json.dumps(
            {
                "status": "PASS",
                "manifest_id": payload["manifest_id"],
                "project_id": payload["project_id"],
                "report_count": 3,
                "nodes": {
                    node: {
                        "assignment_id": by_node[node]["assignment_id"],
                        "artifact_content_id": by_node[node]["artifact_content_id"],
                        "result_i32": scalar_i32(by_node[node]),
                        "fuel_consumed": by_node[node].get("fuel_consumed"),
                    }
                    for node in ("fog-01", "local-01", "cloud-01")
                },
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
