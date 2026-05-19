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

What it does not do:
- start `dserver`
- run `dispatch --execute`
- perform Docker/container side effects
- run WASM
- perform migration
- transfer artifacts
- install anything
- modify local mappings
- perform destructive operations

Notes:
- When at least one installation task exists for the node, the smoke test may update that server-side installation task record to `acknowledged`.
- That status update is part of the installation task lifecycle contract only; it does not execute deployment or installation work.
- Approval submission is metadata-only; it does not execute deployment or installation work.
- Readiness is informational only; it does not execute deployment or installation work, even when readiness is `ready_for_assisted_execution`.
- Assisted execution request submission is metadata-only; it does not execute deployment or installation work.
- Assisted execution claim submission is metadata-only; it does not execute deployment or installation work.
- Assisted claim lifecycle updates are metadata-only; they do not execute deployment or installation work.
- If no `candidate_for_assisted_execution` task exists for the node, operator approval checks are skipped and the smoke test still succeeds.
- If no `ready_for_assisted_execution` task exists for the node, assisted execution request checks are skipped and the smoke test still succeeds.
- If no requested assisted execution request exists for the node, assisted execution claim checks are skipped and the smoke test still succeeds.
- If a requested assisted execution request is already claimed, the smoke test may validate the latest claim instead of failing.
- If no claimed assisted execution claim exists for the node, assisted claim lifecycle update checks are skipped and the smoke test still succeeds.
- Repeated smoke runs may encounter an already released claim and should not fail solely for that reason; the smoke test validates the latest claim instead of requiring a fresh update.
