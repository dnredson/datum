# Phase 119F Audit Evidence Bundle

**Evidence subject:** the source-code claims made by the Phase 119F governed source-cleanup
signal, identity & authority audit.

**Source commit:** `97646f3da316137532622752f1bdfa51503c9b17`
(branch `agent/phase119-migration` at the time this bundle was produced). Every excerpt in
`source-evidence.txt` is retrieved with `git show 97646f3:<path>`, so it stays valid and
reproducible even if the working tree, or this branch's HEAD, later moves on.

**Audit document:**
[`documentation/reference/phase119f-governed-source-cleanup-authority-audit.md`](../../phase119f-governed-source-cleanup-authority-audit.md)

**Collection method:** read-only source inspection only — `git show <commit>:<path>` to retrieve
exact file contents at the pinned commit, combined with `sed`/`grep` line-range filtering, plus a
small number of `git grep <commit> -- <path>` searches for absence claims. No code was compiled or
executed, no live infrastructure (RPi, fog node, DServer instance) was used, and no test suite was
run to produce this evidence. See `commands.txt` for the exact, reproducible command list.

**No production code or test code was modified** to produce this bundle. **No secrets, passwords,
Firebase credentials, private SSH material, node-local files, or binaries are included** — every
file in this directory is plain-text, source-derived evidence and manifest metadata.

**Status of this evidence:** supporting material only. It documents what the cited source code
does at the pinned commit, in support of the Phase 119F audit's own conclusions. It is not a new
authority, does not supersede the audit document, and does not itself authorize or implement any
change — Phase 119F remains a design/audit deliverable, and any future implementation (Phase 119G
or later) is out of scope for this bundle.

## Evidence index

| Evidence ID | Phase119F claim | Source path(s) | Function/type | Evidence location in source-evidence.txt |
|---|---|---|---|---|
| E01 | OperationalDGraph node membership derives only from current placements; a node with zero placements disappears from the graph. | `dserver/src/core/ddeploy.rs` | `union_project_operational_graph` | `E01` block |
| E02 | A node missing from the graph cannot currently receive an empty slice automatically — it hard-errors. | `dserver/src/core/operational_dgraph.rs` | `build_operational_slice` | `E02` block |
| E03 | Empty `desired_services` on a present node is structurally legal. | `dserver/src/core/operational_dgraph.rs` | `OperationalNodeDesiredState`, `validate_operational_dgraph` | `E03` block |
| E04 | `desired_state` currently allows only `"running"`/`"completed"`, on both server and agent. | `dserver/src/core/operational_dgraph.rs`, `DATUM/src/agent/operational_reconciliation.rs` | `validate_desired_service`, `validate_context` | `E04` block |
| E05 | The agent acts only by iterating `context.desired_services`; an empty list drives zero per-service branches. | `DATUM/src/agent/operational_reconciliation.rs` | main dispatch loop (line 1361), `validate_context` loop (line 2722) | `E05` block |
| E06 | Existing rollback/M8 is per-service, explicitly requested, and Stage107A lease-gated. | `DATUM/src/agent/operational_reconciliation.rs`, `DATUM/src/agent/native_process_runtime.rs`, `DATUM/src/agent/stage107a_enforcement.rs` | `check_boundary`, `control_plane_authorized`, `rollback_trusted_container_intent`, `NativeReconciliationAction::Rollback`, `check_generation_and_projection` | `E06` block |
| E07 | Container identity evidence (datum.* Docker labels) already exists. | `DATUM/src/agent/operational_reconciliation.rs` | `ExecutionIdentityLabels`, `inspected_container_labels_match` | `E07` block |
| E08 | Native-process managed identity already persists strong evidence, keyed by a stable per-service key. | `DATUM/src/agent/native_process_state.rs`, `DATUM/src/agent/native_process_acquisition.rs` | `ManagedProcessRuntimeStateV1`, `stable_service_key` | `E08` block |
| E09 | Server authorization history proves issuance, not mutation success — no report-back channel exists. | `dserver/src/api/operational_dgraph.rs`, `dserver/src/core/operational_dgraph.rs`, `dserver/src/core/operational_dgraph_persistence.rs`, `dserver/src/main.rs` | `authorize_operational_reconciliation`, `OperationalReconciliationAuthorization`, `OperationalDGraphPersistedState`, route table | `E09` block |
| E10 | Current ServiceArtifact identity can differ from historical runtime identity — generation/digest are mutable and monotonic. | `dserver/src/core/service_artifact_registry.rs` | `execution_generation_high_water`, generation-bump logic | `E10` block |
| E11 | No generic missing-context delete-all/sweep/prune path exists. | `DATUM/src/**`, `dserver/src/**` (grep) | N/A (negative search, paired with E05/E06) | `E11` block |
| E12 | D-Engine remains derived only from current accepted placements; unaffected by any future cleanup directive. | `dserver/src/core/ddeploy.rs` | `derive_active_dengine_nodes` | `E12` block |
| E13 | Current acceptance history retains enough structure to derive previous-vs-current placement relationships; not pruned in production code. | `dserver/src/core/ddeploy.rs` | `DDeployAcceptanceV1`, `DDeployStateV1` | `E13` block |
| E14 | v1/v2 canonical reference parity is field-identical but achieved via manual normalization, not a shared type. | `dserver/src/core/canonical_dmap.rs`, `dserver/src/core/canonical_dmap_v2.rs`, `dserver/src/core/ddeploy_migration.rs` | `CanonicalContractReferenceV1`, `ContractRefV2`, `locate_acceptance` | `E14` block |

### Phase 119F-R1 supplemental evidence (source-evidence-r1.txt)

Added during the R1 independent design-correction pass. See
[`phase119f-r1-governed-source-cleanup-design-correction.md`](../../phase119f-r1-governed-source-cleanup-design-correction.md)
for the correction this evidence supports.

| Evidence ID | Claim | Source path(s) | Function/type | Evidence location |
|---|---|---|---|---|
| E15 | Operational authorization persistence is keyed by `(project_id, node_id)` only; a newer durable authorization overwrites the prior one — no durable history of old authorization snapshots is retained. | `dserver/src/core/operational_dgraph_persistence.rs`, `dserver/src/api/operational_dgraph.rs` | `authorization_key`, authorize-handler insert | `E15` block, `source-evidence-r1.txt` |
| E16 | ServiceArtifact registry retains only current artifact content plus a generation high-water counter, not complete historical snapshots. | `dserver/src/core/service_artifact_registry.rs` | `ServiceArtifactRegistryState`, artifact update path | `E16` block, `source-evidence-r1.txt` |
| E17 | DDeploy v2 mirrors v1's proposals/acceptances/active structure; a v1-to-v2 transition preserves old v1 acceptance records (only the active pointer is retargeted). | `dserver/src/core/ddeploy_v2.rs`, `dserver/src/core/ddeploy.rs` | `DDeployAcceptanceV2`/`DDeployStateV2`/`ActiveDMapV2`, v1-to-v2 transition | `E17` block, `source-evidence-r1.txt` |
| E18 | Container `datum.*` labels and the static `DispatchContainerMapping` config carry no project/application/service/dmap_ref identity. | `DATUM/src/agent/operational_reconciliation.rs`, `DATUM/src/config.rs` | `DATUM_LABEL_*`, `DispatchContainerMapping` | `E18` block, `source-evidence-r1.txt` |
| E19 | No server path binds a fresh artifact assignment to historical identity; every join resolves against the current registry, and a single unmatched service fails the whole node's context. | `dserver/src/core/artifact_reconciliation_context.rs` | `resolve_assignments_from_registry_with_authority` | `E19` block, `source-evidence-r1.txt` |
| E20 | M8 rollback already excludes image/target-identity verification and never checks physical container labels before `docker rm -f`; its safety rests on trusted `container_name` plus authorization/lease freshness. | `DATUM/src/agent/operational_reconciliation.rs` | `rollback_service_block_from` | `E20` block, `source-evidence-r1.txt` |

## Files in this bundle

- `README.md` — this file.
- `manifest.json` — machine-readable index of all 20 evidence entries (id, claim, source paths/symbols).
- `source-evidence.txt` — the actual source excerpts, commands, and hedged interpretations for E01–E14.
- `source-evidence-r1.txt` — the R1 supplemental source excerpts for E15–E20.
- `commands.txt` — every read-only command used to produce this bundle, in order.

## Honesty notes

Every "Interpretation" in `source-evidence.txt`/`source-evidence-r1.txt` is deliberately scoped to
state only what the quoted code proves, and explicitly names what it does not prove (e.g. E09,
E11, E17, and E19 all flag that a negative/scoped claim rests on the absence of a match in a
specific search/struct/function, not on an exhaustive proof of absence). No inconsistency was
found between the Phase 119F audit's claims about CURRENT code (E01–E14) and this freshly
re-derived source evidence; had one been found, this bundle would have stopped and reported it
rather than silently reconciling the two.

**Phase 119F-R1 update:** a later independent review (E15–E20) found that Phase 119F's evidence
about current code (E01–E14) remained accurate, but that its *recommended future design* (§22, §24
of the original audit) had two unresolved problems — see
[`phase119f-r1-governed-source-cleanup-design-correction.md`](../../phase119f-r1-governed-source-cleanup-design-correction.md)
for the full analysis and the corrected architecture. This is a correction to a forward-looking
design recommendation, not a retraction of any E01–E14 evidence about the codebase as it exists
today.
