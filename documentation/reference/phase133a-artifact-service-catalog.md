# Phase 133A — Artifact & Service Catalog foundation

Final raw-source hardening rejects repository URL whitespace/control characters
before parsing, without trimming, normalizing or persisting a repaired variant.
RepositoryCommand argv is modeling metadata and MUST NOT carry plaintext credentials.
Use typed secret references instead. The catalog does not inspect arbitrary strings
for secret-like content and is not "secret-proof". Future execution requires typed
secret injection, authorization and policy; stored argv is not execution permission.

Status: Phase 133A frozen after independent review, user visual approval and final
raw repository URL hardening. Phase 133B is a separate non-authoritative modeling layer.

## Existing-model audit

- `service_artifact_registry.rs` / `artifacts/catalog/*.json`: durable, project-scoped
  operational execution descriptors, generation high-water marks and projection
  digests. Existing create/update/pinning authority is retained, not duplicated.
- ADR-0011 / execution_projection / Phase 107A: JCS immutable content identity,
  config hashes, secret binding/version semantics, execution generations.
- ADR-0013/0014 / Phase 107B: server-resolved OCI platform manifest/config digests;
  mutable tags are not sufficient. Catalog import requires an already resolved
  identity and reuses the existing validator; it performs no registry resolution.
- ADR-0015 / Phase 108: container/native_process are the operational dispatch
  vocabulary. Phase 110 canonical DServ is host-independent `datum-dnode/0`;
  it is not an operational artifact or catalog definition.
- ADR-0021 / Phase 111 and Phase 117G: D-Code descriptors have their own exact
  digest-addressed revision authority. Management WASM is separate again.
  Neither is aliased to native/container. D-Code import is not included in this
  first catalog adapter; this does not imply D-Code execution is unsupported.
- `DATUM/src/agent/service_artifact.rs` implements HTTP/TCP health probing;
  native process and invocation DMonitor producers report evidence. Phase 125
  scheduler cadence is not a per-definition deployment authority.
- Secret resolver and ServiceArtifactSecretBinding use opaque reference/version/
  environment-target contracts, never control-plane secret values.
- No reusable, global ServiceDefinition authority or tenancy contract existed.
  The new catalog supplies that missing modeling authority only.

## Authority and scope

Artifact != ServiceDefinition != logical DServ != ServiceInstance != runtime
evidence != placement. Definitions are global reusable metadata; artifact source
project is provenance, not catalog ownership or availability authorization.
There is no new application registry, project usage, graph integration or instance
creation. A global definition does not authorize deployment for every project.

The Atomic adapter deliberately reuses pinned existing container/native artifacts. It
captures a narrow immutable ArtifactSnapshot (source project/ID/generation/digest,
resolved OCI identity or native executable hash), deduplicated by JCS digest.
Multiple definitions can reference one snapshot. No operational artifact is
updated. Commands, arguments, networking, mounts, config hashes and lifecycle
configuration remain owned by the original artifact contract; the catalog does
not make a new execution configuration schema or imply it can deploy a snapshot.
No arbitrary legacy metadata/environment values are copied into the new store.

## ServiceDefinition and revisions

Stable definition ID plus immutable monotonically increasing revision and digest.
Creation requires expected_revision=0; edits append with expected_previous_revision.
SQLite IMMEDIATE transactions provide cross-process compare-and-swap. Historical
revisions retain original artifact identity even when source artifacts change.
Saving against changed artifact generation/digest conflicts rather than substituting.
There is no PUT overwrite or DELETE. Definitions cannot alter accepted applications.

Optional sections: CPU millicores, memory/storage MB; network-interface/device/P4
port/GPU capability requirements; HTTP/TCP health declaration; typed realization
monitoring intent with cadence_seconds 1–86400; versioned secret references;
definition dependencies. Numeric bounds, unique identities, supported mechanisms
and strict unknown-field decoding are validated. Requirements are not allocations;
exclusive capability intent never reserves a named resource or physical node.
Health timeouts are 1–60000 ms and ports are 1–65535. Health and DMonitor intent
remain distinct. Saving does not start probes, wire producers or configure a scheduler.

## Tenancy and examples

- Dedicated: one intended application/project tenant, not exclusive node ownership.
- SharedTenantAware: explicit tenant isolation capability, optional domain/namespace/
  vhost/schema/organization unit. No provider-specific provisioning is implemented.
- SharedStateless: sharing capability, not an automatic sharing policy.
- SystemGlobal: infrastructure/platform-oriented capability, not automatic assignment.

Test-only P4 Sniffer is Atomic with pinned native identity, dedicated tenancy,
exclusive network-interface requirement and 30-second monitoring intent. Test-only
Magistrala is Composite/Discovered with an immutable repository-source declaration
and SharedTenantAware/domain, not a pretend single container. It exists once;
no per-project copies, tenant domains or shared instances are created. Fixtures
are not production catalog seeds.

Future ServiceInstance, TenantBinding, ProjectResourceBinding and ResourceAllocation
remain independent relationships. Catalog scope is not authorization; DServer's
existing lack of application-layer authentication remains a deployment limitation.
Catalog writes assume a trusted/local control-plane boundary. DServer's default
`0.0.0.0:8080` binding is NOT safe for untrusted clients. Before any untrusted
network exposure, application authentication/authorization or an enforced external
trusted access boundary is a security prerequisite, not an optional improvement.

## Persistence/API

Set `DATUM_SERVICE_CATALOG_DB_PATH` to an absolute, standalone SQLite path. Unset
or inaccessible means safe 503, never in-memory fallback. Explicit startup initializes
only a nonexistent catalog database. Existing databases are validated, never
repaired: version 133, exact required columns, nullability, primary-key order and
unique primary-key indexes must be present. Version-0 existing files are not
first use. Bootstrap uses an atomic schema transaction and a same-process mutex;
multi-process bootstrap coordination is not certified (a competing creator may
fail safely). Ordinary reads open READ_ONLY; writes open READ_WRITE without CREATE.
Every ordinary connection validates schema. Removing the file after initialization
fails safely without recreating it. Missing tables fail as corruption, never empty
catalog success. Catalog
SQL operations run inside capacity-bounded owned spawn_blocking tasks. History and
artifact digests are verified on read. Errors do not disclose SQL/path/stack details.

- GET/POST `/api/v1/catalog/services`
- GET `/api/v1/catalog/services/:id`
- GET `/api/v1/catalog/services/:id/revisions/:revision`
- POST `/api/v1/catalog/services/:id/revisions`
- GET `/api/v1/catalog/artifacts` — up to 100 eligible pinned source references,
  not all artifacts or complete project inventory; has_more is explicit.
- GET `/api/v1/catalog/artifacts/:snapshot` — immutable captured identity.

List responses are bounded to 100; no pagination promise is made. JSON request
body limit is 64 KiB. Writes are only catalog rows. Source pinning/resolution and
operational execution authority are unchanged.

## Console

Global `/catalog` keeps catalog scope distinct from project Services usage; project
Services navigation remains future/planned. The manual form selects eligible pinned
artifact identity and shows inherited runtime kind, tenancy, optional requirements,
health, cadence, dependencies and secret references. Review/save create revisions;
historical forms are disabled/read-only. Artifact inspection retrieves the exact
stored `artifact_snapshot_id`, independently of the live artifact registry. Full
source projection digest, native executable hash, OCI repository/platform/manifest/
config/execution-reference pins are selectable read-only text. Historical forms
have no replacement selector. Create/new-revision selection exposes the selected
pin before saving; no current/live artifact substitutes for a historical snapshot.
No plaintext-secret input, deployment button
or graph entity insertion exists. No templates or chatbot are added.

The compact foundation editor edits one capability and one secret reference at a
time, preserving additional API-defined entries on revision. This is a limitation,
not a claim to a full modeling tool. Forms use labels, descriptions, error summary,
keyboard-native controls and a single-column layout at 768px.

Phase 133B owns guided/chatbot modeling; 136 owns placement/allocation; 138 owns
deploy/reconciliation. Operator definition saving is not any of those actions.

## Validation

Run `npm test`, `npm run build`, the optional built-app DOM/freeze gate, and
`cargo test --manifest-path dserver/Cargo.toml --bin DServer service_catalog`.
Full DServer tests/build and diff checks are required before independent review.

Validation evidence before final hardening: 91 lightweight frontend tests including
six catalog tests; 84 built-app DOM/freeze tests; 16 focused catalog tests.
Hardening adds seven store/API regressions, four pin-validation/client tests and
two DOM pin/history regressions. Full DServer regression and Rust build
passed. Strict TypeScript/Vite build passed. Real isolated DServer through Vite
returned HTTP 200 with a valid empty configured catalog. Populated catalog behavior
uses test-only fixtures; no graphical certification is claimed for this phase.
There are no dependency changes. Existing Rust warnings and the Vite >500 kB chunk
warning remain. Phase 133A is uncommitted; Phase 133B has not started.

Local manual launch (PowerShell, repository root):

```powershell
$catalogDirectory = Join-Path $env:LOCALAPPDATA 'DATUM'
New-Item -ItemType Directory -Force -Path $catalogDirectory | Out-Null
$env:DATUM_SERVICE_CATALOG_DB_PATH = Join-Path $catalogDirectory 'service-catalog.db'
cargo run --manifest-path dserver/Cargo.toml
```

In another terminal, `cd console; npm run dev`, then open
`http://127.0.0.1:5173/catalog`. Existing artifacts must already be registered and
pinned through their existing governed workflow. No production demo data is inserted.

## Final hardening and user visual review

F1 persistence failure handling and F2 exact artifact pin inspection are implemented.
Deferred: F3 stored revision metadata validation; F4 deeper multi-entry/history
regressions; F5 authentication/authorization (security prerequisite before untrusted
exposure); F6 dependency revision resolution; F8 editor usability. F7 narrow consumed
artifact pin validation is addressed with F2; this is not a backend schema mirror.

No enabled graphical browsers/apps were exposed to this agent. Headless DOM is not
graphical certification. Phase 133A remains uncommitted pending user visual approval.

Final hardening validation: 95 lightweight frontend tests (10 catalog tests),
86 built-app DOM/freeze tests, 23 focused catalog model/store/API tests;
full DServer 2277 passed, 1 ignored; strict frontend and Rust builds passed;
npm audit 0 vulnerabilities; diff check passed. The initial Rust build encountered
a Windows executable lock from the owned smoke server; it passed after that server
was stopped. Existing Rust warnings and the Vite >500 kB warning remain.

Populated TEST-ONLY fixture (no DServer or production catalog involved):

```powershell
cd console
node --experimental-strip-types tests/catalog-visual-fixture.mjs
```

In another PowerShell terminal from `console`:

```powershell
$env:DATUM_DEV_API_TARGET = 'http://127.0.0.1:8133'
npm run dev
```

Open `http://127.0.0.1:5173/catalog`. Stop another Vite instance using that port
first. The fixture listens only on localhost; simulated saves live only in memory
and disappear on restart. It is not production authority or backend validation.

Inspect P4 Sniffer latest revision 2 (native pin B); enter historical revision 1
to inspect native pin A, deliberately absent from live sources. Select the separate
container artifact reference to inspect platform/manifest/config pins. Magistrala
now exposes composite repository-source, command and discovery declarations. Create a
definition or new revision to inspect selected pin details, tenancy, cadence and
requirements before Save. Confirm historical fields are read-only and have no
replacement artifact selector. Inspect at desktop and approximately 768px widths.

## Phase 133A — Composite Services & Dependency Semantics Hardening

This is final hardening of the still-uncommitted Phase 133A, not a new roadmap
phase. Earlier Catalog UI graphical approval applies to the previous presentation;
this material contract extension is ready for a new targeted independent review,
not automatically frozen by that approval.

### Atomic vs Composite ServiceDefinition

`service_shape` is `atomic` or `composite`. Atomic requires an existing pinned
artifact and compatible runtime kind; composition/provisioning are absent.
Composite requires a composition declaration and deliberately has no root artifact,
runtime kind or artifact_snapshot_id. Its operational components have their own
future identities/artifacts. This avoids equating a multi-container application
with one container. Existing atomic snapshots and exact pin inspection are retained.

### Composition semantics

Static composition has 1–64 component references: unique component `key` plus exact
`definition_id` and `revision`. Every referenced revision must already exist and
validate in the catalog. Multiple aliases may reference the same software revision
as distinct structural roles. Direct self-definition references are rejected.
Static composition has no repository-command provisioning contract in this foundation.

Discovered composition declares `docker_compose` discovery and 1–16 unique,
repository-relative Compose file paths (no absolute or traversal paths). It carries
no hardcoded observed container inventory. It requires repository-command metadata.
No Docker/Compose invocation, automatic parsing or discovery is performed here.
Composite definitions may independently declare external required capabilities.

### Provisioning contract and immutable source

The audit found legacy free-form acquisition labels (`git`, `command_bundle`) in
ServiceArtifactRegistry, but no typed composite/member discovery or immutable
repository provisioning contract. The new `repository_command` is metadata only,
not a competing artifact resolver/executor. It stores credential-free HTTPS
repository URL (max 512 bytes), full lowercase 40-hex Git commit, and argv (1–32
bounded strings). Working directory is implicitly the checked-out repository root.
Branches/tags/latest are not accepted instead of a commit; the catalog validates
syntax, not remote commit existence. Example fixture commits are synthetic test pins.

Magistrala's [official repository](https://github.com/absmach/magistrala) describes
its repository/Make workflow; the [Makefile](https://github.com/absmach/magistrala/blob/main/Makefile)
uses Docker Compose for `run_latest`. The fixture declares `['make','run_latest']`
against a pinned source commit without claiming current prerequisites are complete.
The mutable image choices implied by `run_latest` are NOT reproducible execution
authority. Future deployment must capture effective Compose configuration, resolve
component image digests, verify source and preserve execution/artifact provenance.
Pinning Git alone is not sufficient to claim reproducible resulting containers.

### Expected vs observed components

Expected members may eventually come from `docker compose config`; observed members
may come from Docker runtime/`docker compose ps` and admitted DMonitor evidence.
Those are separate authorities. Future ServiceInstance may contain ComponentInstance
identities, each with independent realization, health/monitoring and alert/event
evidence. Composite degradation must not hide those behind an opaque health boolean.
Neither instance type nor aggregate health is implemented in this phase.

### Exact dependencies and revision-pinned history

`exact_dependencies` replaces the intermediate `dependencies: string[]` contract.
Each reference is an existing `definition_id` plus exact revision (1–1,000,000),
max 64 unique pairs. No implicit latest; direct self-definition references fail.
Creating B@5 does not change A@2 referencing B@4. Referential checks occur before
commit; referenced rows cannot be updated/deleted through catalog APIs.

Dependencies are software relationships, NOT startup or deployment order. Peer
relationships between stable IDs are allowed through previously committed revisions
(A@2 → B@1 → A@1); no forward/dangling references or direct self-reference are
permitted. This preserves finite immutable revision relationships without imposing
a topological startup algorithm. Planning/order semantics remain future work.

### Required and provided service capabilities

`requires: [{capability}]` declares unresolved requirements; `provides: [id]`
declares provider capabilities. Each list has max 64 unique identifiers, max 128
ASCII bytes: dot-separated lowercase segments with digits/hyphens. Examples are
`database.postgresql`, `messaging.mqtt`, `api.smart-coffee`; no global taxonomy is
hardcoded. Fine-grained constraint expressions are not included yet.
A gateway may require and provide the same capability, intentionally allowed.
These lists remain separate from resource requirements such as network_interface,
GPU, P4 port, millicores and memory. Neither list proves runtime availability.

Test-only Smart Coffee API is Atomic, provides `api.smart-coffee`, requires
`database.postgresql` and `messaging.mqtt`. Test-only PostgreSQL provides
`database.postgresql`. Saving them creates no relationship automatically.
Future RequirementBinding/ServiceDependencyBinding may bind a requirement to an
existing dedicated/shared ServiceInstance or a proposed new instance. Existing
provider reuse, tenant policy and allocation are future planner/proposal decisions.

### Revision, digest and editor

All structure, source, discovery, exact reference and capability fields are inside
the JCS-digested immutable definition body. Every edit appends a revision; Atomic
to Composite and dependency/capability changes cannot retroactively alter history.
The uncommitted intermediate DTO is intentionally not wire-compatible; old draft
catalog files are not silently migrated or repaired. Use a fresh test catalog for
this contract. Committed Phase 132/earlier APIs are unchanged.

The manual editor separates Service structure, Exact dependencies, Required and
Provided service capabilities. It edits the first static component/exact dependency
while retaining additional stored entries, and clearly pins component/dependency
revision fields. Discovered mode exposes source commit, argv and Compose-file JSON
arrays. Historical views disable all fields. No comma-separated dependency-ID field
remains. Artifact selector/pin details apply only to Atomic definitions.

### Future graph and Phase 133B authority

CONTAINS/COMPONENT means structural composition; DEPENDS_ON means an exact software
revision relation; REQUIRES means unresolved service capability. Future BOUND_TO
means a selected concrete provider. No catalog relationship becomes an accepted
DGraph/DCall edge in Phase 131/132; those graphs remain untouched.
Phase 133B may inspect operator intent/supported repository metadata and suggest
composite structure, provisioning/discovery and capabilities as a candidate draft.
The same validator/store remains authoritative only after operator review/save.
AI output is never authority. No AI, repository agent, binding, instance, allocation,
deployment, scheduler registration or execution is implemented now.

Extension validation: 98 lightweight tests including 13 catalog tests; 88 built-app
DOM/freeze tests; 30 focused catalog model/store/API tests; full DServer 2284 passed,
1 ignored, 0 failed; strict frontend and Rust builds passed; npm audit 0 vulnerabilities;
diff check passed. The populated fixture remains test-only. No commit/push occurred.
Recommendation: READY FOR FINAL PHASE 133A INDEPENDENT REVIEW.

Final freeze gate: independent review ACCEPT WITH MINOR FIXES; required R1 fixed.
99 lightweight tests, 88 built-app DOM/freeze tests, 31 catalog model/store/API
tests and full DServer 2285 passed/1 ignored/0 failed. TypeScript/Vite and Rust
builds passed; npm audit zero vulnerabilities. User graphical approval of the
Catalog is recorded; no recipe, repository clone, Docker or service deployment
was executed. Existing warnings remain. Catalog writes remain trusted/local only.
