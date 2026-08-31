#!/usr/bin/env python3
import argparse
import json
import urllib.error
import urllib.request

EXPECTED = {
    "fog-scale-module": {"node_id": "fog-01", "result": 24},
    "local-aggregate-module": {"node_id": "local-01", "result": 30},
    "cloud-delta-module": {"node_id": "cloud-01", "result": 20},
}


def get_json(url: str) -> dict:
    try:
        with urllib.request.urlopen(url, timeout=10) as response:
            return json.load(response)
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"GET {url} failed with HTTP {exc.code}: {body}") from exc


def result_i32(report: dict):
    metadata = report.get("metadata") or {}
    results = metadata.get("results") or []
    if len(results) != 1:
        return None
    result = results[0]
    if result.get("type") != "i32":
        return None
    return result.get("value")


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate I34 materialized observed D-Graph")
    parser.add_argument("--dserver", required=True)
    args = parser.parse_args()

    base = args.dserver.rstrip("/")
    payload = get_json(base + "/api/v1/controlled-wasm/observed-dgraph")
    graph = payload.get("graph") or {}
    errors = []

    if payload.get("source_dgraph_id") != "i34-external-dgraph":
        errors.append(f"unexpected source_dgraph_id: {payload.get('source_dgraph_id')!r}")
    if payload.get("manifest_id") != "manifest:i34-external-dgraph":
        errors.append(f"unexpected manifest_id: {payload.get('manifest_id')!r}")
    if payload.get("project_id") != "i34-live-project":
        errors.append(f"unexpected project_id: {payload.get('project_id')!r}")
    if payload.get("report_count") != 3:
        errors.append(f"expected report_count=3, received {payload.get('report_count')!r}")
    if payload.get("side_effects_performed") is not False:
        errors.append("side_effects_performed must remain false")
    if payload.get("validation_errors") != []:
        errors.append(f"observed D-Graph validation errors: {payload.get('validation_errors')!r}")

    nodes = {node.get("node_id") for node in graph.get("nodes", [])}
    expected_nodes = {value["node_id"] for value in EXPECTED.values()}
    if nodes != expected_nodes:
        errors.append(f"expected nodes {sorted(expected_nodes)}, received {sorted(str(x) for x in nodes)}")

    modules = {module.get("module_id"): module for module in graph.get("modules", [])}
    reports = graph.get("wasm_reports", [])
    if len(reports) != 3:
        errors.append(f"expected exactly 3 graph wasm_reports, received {len(reports)}")

    report_by_module = {}
    for report in reports:
        metadata = report.get("metadata") or {}
        module_id = metadata.get("module_id")
        if module_id:
            report_by_module[module_id] = report
        if report.get("status") != "executed":
            errors.append(f"report {report.get('report_id')!r}: status is not executed")
        if metadata.get("integrity_verified") is not True:
            errors.append(f"report {report.get('report_id')!r}: integrity_verified is not true")
        content_id = metadata.get("artifact_content_id")
        if not isinstance(content_id, str) or not content_id.startswith("sha256:"):
            errors.append(f"report {report.get('report_id')!r}: invalid artifact_content_id")

    summaries = {}
    for module_id, expected in EXPECTED.items():
        module = modules.get(module_id)
        if module is None:
            errors.append(f"missing module {module_id!r}")
            continue
        if module.get("observed_state") != "executed":
            errors.append(f"{module_id}: observed_state is not executed")
        report = report_by_module.get(module_id)
        if report is None:
            errors.append(f"{module_id}: no observed WASM report materialized")
            continue
        if report.get("node_id") != expected["node_id"]:
            errors.append(
                f"{module_id}: expected node {expected['node_id']!r}, received {report.get('node_id')!r}"
            )
        result = result_i32(report)
        if result != expected["result"]:
            errors.append(
                f"{module_id}: expected result {expected['result']}, received {result!r}"
            )
        metadata = report.get("metadata") or {}
        summaries[module_id] = {
            "node_id": report.get("node_id"),
            "report_id": report.get("report_id"),
            "result_i32": result,
            "artifact_content_id": metadata.get("artifact_content_id"),
            "fuel_consumed": metadata.get("fuel_consumed"),
        }

    report_links = [
        rel
        for rel in graph.get("relationships", [])
        if rel.get("from_kind") == "WasmRuntimeReport" and rel.get("relation") == "OBSERVES"
    ]
    if len(report_links) != 6:
        errors.append(f"expected 6 WasmRuntimeReport OBSERVES relationships, received {len(report_links)}")

    if errors:
        print(json.dumps({"status": "FAIL", "errors": errors, "payload": payload}, indent=2))
        raise SystemExit(1)

    print(
        json.dumps(
            {
                "status": "PASS",
                "source_dgraph_id": payload["source_dgraph_id"],
                "manifest_id": payload["manifest_id"],
                "project_id": payload["project_id"],
                "report_count": payload["report_count"],
                "wasm_report_observes_relationships": len(report_links),
                "modules": summaries,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
