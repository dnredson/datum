# Scripts

## Contract Smoke Test

Prerequisites:
- `dserver` must already be running.
- `dserver` must have valid Firestore configuration available.
- `DATUM/config.toml` must point to the same server, project, and node that you want to validate.

Run from `PD`:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\smoke-client-server-contract.ps1
```

Optional parameters:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\smoke-client-server-contract.ps1 `
  -ServerUrl http://localhost:8080 `
  -ClientConfig ..\DATUM\config.toml `
  -ProjectId smart-city-campinas `
  -NodeId fog-01 `
  -VerboseOutput
```

What it validates:
- server health
- client capability publish
- server capability read-back
- client snapshot publish
- graph materialization
- analysis
- capability-aware action-plan
- gated dispatch
- node-scoped dispatch
- client dispatch consumption without execution
- client action result publish via `dispatch --publish-results`
- server node latest `action-results` read-back
- server project latest `action-results` listing
- server global `deployment-planning/latest` endpoint
- server node-scoped `deployment-planning/for-node/{node_id}` endpoint
- client `deployment-planning` and `deploy-plan` viewer contract
- server global `installation-plan/latest` endpoint
- server node-scoped `installation-plan/for-node/{node_id}` endpoint
- client `installation-plan` and `install-plan` viewer contract
- client `installation-tasks --materialize` contract
- server installation task list for node
- server installation task list for project
- server `installation_policy` metadata in installation tasks
- `safe_to_auto_execute` remains false in installation policy when present
- client `installation-tasks` output displays policy information when available
- optional server installation task get when tasks exist
- optional client installation task status update to `acknowledged` when tasks exist
- optional detection of `candidate_for_assisted_execution` tasks
- optional client approval submission via `installation-tasks --approve`
- optional server latest approval read-back for a candidate task
- optional server approval list for the node
- optional client latest approval viewer via `installation-tasks --approval`
- optional client approval list viewer via `installation-tasks --approvals`
- server installation-task readiness list for node
- optional server single-task readiness when tasks exist
- client readiness viewer via `installation-tasks --readiness`
- optional client single-task readiness viewer via `installation-tasks --task-id <task_id> --readiness`
- `safe_to_auto_execute` remains false in readiness responses
- optional detection of `ready_for_assisted_execution` tasks
- optional client assisted execution request submission via `installation-tasks --request-assisted-execution`
- optional server latest assisted execution request read-back for a ready task
- optional server assisted execution request list for the node
- optional client latest assisted execution request viewer via `installation-tasks --execution-request`
- optional client assisted execution request list viewer via `installation-tasks --execution-requests`
- `safe_to_auto_execute` remains false in assisted execution request responses
- optional detection of requested assisted execution requests when available
- optional client assisted execution claim submission via `installation-tasks --claim-assisted-execution`
- optional server latest assisted execution claim read-back for a requested task/request pair
- optional server assisted execution claim list for the node
- optional client latest assisted execution claim viewer via `installation-tasks --execution-claim`
- optional client assisted execution claim list viewer via `installation-tasks --execution-claims`
- `safe_to_auto_execute` remains false in assisted execution claim responses
- optional client assisted claim status update via `installation-tasks --claim-status released`
- optional server latest assisted claim read-back after status update
- optional client latest assisted claim viewer after status update
- `safe_to_auto_execute` remains false after assisted claim lifecycle updates
- optional client assisted execution attempt submission in `no_op` mode via `installation-tasks --attempt-assisted-execution --execution-mode no_op`
- optional server latest assisted execution attempt read-back for a claimed task/request/claim tuple
- optional server assisted execution attempt list for the node
- optional client latest assisted execution attempt viewer via `installation-tasks --execution-attempt`
- optional client assisted execution attempt list viewer via `installation-tasks --execution-attempts`
- `safe_to_auto_execute` remains false in assisted execution attempt responses
- `side_effects_performed` remains false in assisted execution attempt responses
- server assisted execution state list for node
- optional server single-task assisted execution state when a task exists
- client assisted execution state viewer via `installation-tasks --execution-state`
- optional client single-task assisted execution state viewer via `installation-tasks --task-id <task_id> --execution-state`
- server assisted execution state summaries enriched with local mapping context after local mapping preflight publish
- optional server single-task assisted execution state local mapping context check
- client `installation-tasks --execution-state` display of local mapping context
- optional client single-task `installation-tasks --task-id <task_id> --execution-state` display of local mapping context
- server node-wide and single-task state checks for `local_mapping_status`
- `safe_to_auto_execute` remains false in assisted execution state summaries
- `side_effects_performed` remains false in the current no-op/dry-run assisted execution state flow when present
- server project-wide `local-mapping-recommendations` endpoint
- server node-scoped `local-mapping-recommendations/for-node/{node_id}` endpoint
- client local mapping recommendation viewer via `installation-tasks --local-mapping-recommendations`
- server project-wide `local-mapping-patch-proposals` endpoint
- server node-scoped `local-mapping-patch-proposals/for-node/{node_id}` endpoint
- client local mapping patch proposal viewer via `installation-tasks --local-mapping-patch-proposals`
- client local mapping patch proposal file generation via `installation-tasks --local-mapping-patch-proposals --write-local-mapping-patch <path>`
- generated proposal file exists and is advisory only
- generated proposal file is not applied
- active client config hash/content remains unchanged after proposal file generation
- generated proposal files must not be committed
- proposed_config_snippet remains operator-review only
- client local mapping preflight via `installation-tasks --local-mapping-preflight`
- optional client single-task local mapping preflight via `installation-tasks --task-id <task_id> --local-mapping-preflight`
- client local mapping preflight publish via `installation-tasks --local-mapping-preflight --publish-local-mapping-preflight`
- server node latest `local-mapping-preflight/latest` endpoint
- server project latest `local-mapping-preflight/latest` endpoint
- optional client single-task local mapping preflight publish via `installation-tasks --task-id <task_id> --local-mapping-preflight --publish-local-mapping-preflight`
- `safe_to_auto_apply` remains false in local mapping recommendations when recommendations are present
- `safe_to_auto_apply` remains false in local mapping patch proposals when proposals are present
- `side_effects_performed` remains false in local mapping patch proposals when proposals are present
- `requires_operator` remains true in local mapping patch proposals when proposals are present
- `proposed_config_snippet` remains advisory only and is not applied
- `safe_to_auto_execute` remains false in local mapping preflight output
- `side_effects_performed` remains false in local mapping preflight output
- `safe_to_auto_execute` remains false in published local mapping preflight reports
- `side_effects_performed` remains false in published local mapping preflight reports

What it does not do:
- start `dserver`
- run `dispatch --execute`
- perform Docker/container side effects
- run Docker inspect
- run WASM
- perform migration
- transfer artifacts
- install anything
- write local config
- modify local mappings
- apply proposed config snippets
- apply generated patch files
- perform destructive operations

Notes:
- When at least one installation task exists for the node, the smoke test may update that server-side installation task record to `acknowledged`.
- That status update is part of the installation task lifecycle contract only; it does not execute deployment or installation work.
- Approval submission is metadata-only; it does not execute deployment or installation work.
- Readiness is informational only; it does not execute deployment or installation work, even when readiness is `ready_for_assisted_execution`.
- Assisted execution request submission is metadata-only; it does not execute deployment or installation work.
- Assisted execution claim submission is metadata-only; it does not execute deployment or installation work.
- Assisted claim lifecycle updates are metadata-only; they do not execute deployment or installation work.
- Assisted execution attempts are no-op/dry-run metadata only; they do not execute deployment or installation work.
- Assisted execution state summaries are read-only; they do not execute deployment or installation work.
- Local mapping recommendations are advisory only; they do not write config, modify mappings, or execute installation work.
- Local mapping patch proposals are operator-reviewed suggestions only; they do not write config, modify mappings, apply proposed config snippets, or execute installation work.
- Local mapping patch proposal file generation is explicit-only; it writes a separate operator-review file and must not change active config or active mappings.
- Local mapping preflight is advisory only; it does not write config, modify mappings, or execute installation work.
- Local mapping preflight publishing is advisory metadata only; it does not write config, modify mappings, or execute installation work.
- Local mapping context in assisted execution state is advisory only; it does not grant execution permission or execute deployment or installation work.
- Empty local mapping recommendation results are valid and do not fail the smoke test.
- Empty local mapping patch proposal results are valid and do not fail the smoke test.
- Empty proposal files are valid when there are no server proposals, as long as advisory wording is present and active config is unchanged.
- Empty local mapping preflight results are valid and do not fail the smoke test.
- Empty published local mapping preflight reports are valid and do not fail the smoke test.
- Patch proposals with `safe_to_auto_apply=true`, `side_effects_performed=true`, or `requires_operator=false` should fail the smoke test.
- Reports with `safe_to_auto_execute=true` or `side_effects_performed=true` should be rejected by the server.
- `local_mapping_status=already_mapped` is still advisory only and does not imply execution permission.
- `local_mapping_status=missing_mapping` or `local_mapping_status=ambiguous` requires operator/config review, not automatic remediation.
- If no `candidate_for_assisted_execution` task exists for the node, operator approval checks are skipped and the smoke test still succeeds.
- If no `ready_for_assisted_execution` task exists for the node, assisted execution request checks are skipped and the smoke test still succeeds.
- If no requested assisted execution request exists for the node, assisted execution claim checks are skipped and the smoke test still succeeds.
- If a requested assisted execution request is already claimed, the smoke test may validate the latest claim instead of failing.
- If no claimed assisted execution claim exists for the node, assisted claim lifecycle update checks are skipped and the smoke test still succeeds.
- Repeated smoke runs may encounter an already released claim and should not fail solely for that reason; the smoke test validates the latest claim instead of requiring a fresh update.
- If no claimed assisted execution claim exists for the node, assisted execution attempt checks are skipped and the smoke test still succeeds.
- If a claim was already released by a repeated smoke run, assisted execution attempt checks are skipped because attempts require a claimed claim.
- If no installation task exists, optional single-task local mapping preflight checks are skipped and the smoke test still succeeds.
- If no installation task exists, optional single-task local mapping preflight publish checks are skipped and the smoke test still succeeds.
- Repeated or partial smoke runs may legitimately have missing approval, request, claim, attempt, state-summary, recommendation, preflight, or published-report detail for some tasks; the smoke test tolerates that as long as the read-only endpoints respond with clean JSON and the safety invariants remain false.
- WebAssembly remains mandatory for the long-term project roadmap, but WASM execution is not part of this smoke milestone.
