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
