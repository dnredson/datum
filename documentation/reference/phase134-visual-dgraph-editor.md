# Phase 134 — Visual DGraph Editor

## Phase 134C — paginated catalog discovery

Add DServ now uses server-side paginated discovery on the EXISTING catalog list
endpoint: `GET /api/v1/catalog/services?limit=25&offset=0&search=...`.
The legacy request without query parameters still returns its original bounded
100-entry list, unchanged for Catalog and modeling-assistant consumers.

Discovery response schema `datum.catalog-discovery/1` carries `items`, `limit`,
`offset`, `total`, `has_more`, and the exact `search` query. Default page size is
25; valid limits are 1–100. Offset is a non-negative integer at most 1,000,000.
Search is at most 256 UTF-8 bytes, without control characters. Invalid/negative/
oversized/unknown parameters are rejected with bounded safe HTTP 400 errors.
Responses are bounded to 2 MB; no server schema/store/index migration is added.

SQL selects each definition's highest numeric stored revision, filters across
the FULL catalog, then orders IDs with SQLite BINARY collation before LIMIT/OFFSET.
Search is literal substring matching of definition ID, display name or provided
capability IDs (ASCII case-insensitive; non-ASCII case folding is not claimed).
`%` and `_` have no wildcard meaning. Count/page selection share one SQLite
READ_ONLY snapshot; returned immutable exact revisions undergo existing catalog
validation. No full catalog is loaded into the browser or Rust model vector.
Pages are live reads, not a catalog-wide frozen browsing session: concurrent new
IDs can shift offsets/counts. This never changes an already selected exact ref.

The picker shows Previous/Next and Page X/Y, keeps search visible, resets to page
one after search changes (200 ms debounce), and ignores stale async responses.
Shape/runtime presentation grouping applies within each returned page only.
Loading, empty catalog, zero search results and safe page failure/retry are distinct.
Pagination/search never alter graph draft payload/dirty state. Selecting a row
still fetches its EXACT revision, compares its digest, and closes the picker into
the explicit graph-local-ID/resource form. No latest reference, save or acceptance
is introduced. Reopening follows the existing convention: fresh first page.

The deterministic visual fixture contains 100 valid-shaped test-only definitions
across Atomic container/native and Composite services. Shared captured fixture
snapshots retain their synthetic pinned identities; definition digests are computed
for each body. Search `EXAMPLE.DISCOVERY-090` or `Discovery Service 090` to find
`service-090@3`, which is outside page one. Existing launch commands below apply.
Review Page 1/4, Next/Previous, last-page Next disabled, search reset to Page 1/1,
selection of exact revision/digest, a different graph-local ID, explicit resources,
DCall/diff/JSON and Validate Draft without save. Zero-results: `no-such-software`.

## Phase 134B — catalog-backed authoring and explicit draft semantics

The approved extension uses `datum.dgraph/2`. The historical Rust type names
`CanonicalDGraphV1` / `CanonicalDServV1` now read both wire versions in DServer
and the independent DATUM agent mirror; names do not select the wire version.
Existing v1 graphs retain their schema and digest, with absent references omitted.
Adding a catalog-backed DServ explicitly upgrades the LOCAL successor draft to v2;
the accepted base remains unchanged. V1 graphs carrying catalog references fail
validation. V2 rejects unknown fields at root, DServ, execution/resource requirement,
DCall and exact-reference levels; exact-reference objects also reject
unknown fields, missing fields and malformed values. Frozen v1 root annotation
handling remains compatible with existing cross-crate fixtures.

`service_id` is graph-local logical identity, independent of the reusable catalog
definition ID. Optional `service_definition` contains exactly `definition_id`,
positive exact `revision` (1–1,000,000), and `digest` (`sha256:<64 lowercase hex>`).
All three participate in canonical serialization/JCS digest. No floating latest,
name match or ID-equality linking occurs. Absence means Unlinked / legacy DServ.
Unlinked DServs remain editable; manual linking is not inferred or implemented.

This reference is MODELING-ONLY. It is not an artifact execution binding, ABI
compatibility assertion, instance/binding, deployment or placement permission.
Container/native catalog entries do not establish `datum-dnode/0` compatibility.
Composite selection records one logical reference only: it does not expand children,
derive aggregate resource feasibility, copy runtime kinds or authorize a recipe.

The original 134B Add DServ flow read the first 100 entries; Phase 134C above
supersedes that limitation with server-side paginated search. It groups
by Atomic/Composite and runtime kind for presentation only. Selection fetches the
EXACT listed revision and checks its digest before presenting the form. Stored
means canonically validated catalog revision; there is no new approval state.
Empty/unavailable catalogs do not silently fall back to arbitrary creation.
Advanced / Manual DServ remains available for legacy/non-catalog compiler workflows.
Register new service opens the existing `/catalog` in a separate tab, preserving
the in-memory graph draft. No duplicate service-definition editor is introduced.

Catalog resources (optional millicores, memory MB, storage, device capabilities)
are reference-only. Graph resources (whole CPU cores/exact bytes) remain explicit:
catalog-backed creation leaves both input fields blank. No unit conversion,
capability satisfaction, exclusive reservation or ABI inference is performed.

Validate Draft performs structural canonical validation, checks the exact accepted
base, then resolves each optional catalog reference through the existing
`CatalogStore.get(id, Some(revision))` READ_ONLY path. Exact records and snapshots
must pass catalog integrity validation and digests must match. Missing revisions
produce bounded findings; missing/corrupt persistence fails closed. Unlinked graphs
do not require a configured catalog. Catalog initialization is reused from server
startup, never triggered by validation. No graph/catalog rows are written.

The pure server/agent canonical validator remains storage-independent; catalog
resolution is DServer editor validation, not an agent-side catalog store. Existing
DDeploy/DMap/DCall/D-Code consumers preserve the extended canonical snapshot and
digest; they do not resolve or execute the reference. Phase 137 must retain a
fresh reference verification gate in any future editor acceptance flow; a previous
editor validation is neither acceptance nor a transferable execution permission.
The DCompile v1 constructor explicitly emits unlinked services. Other touched
consumer constructor changes are test fixtures adding `service_definition: None`;
no placement, monitoring or execution behavior was changed.

Visible copy now states: **Validation checks this local draft only. It does not save
or accept a new DGraph revision.** Draft state remains memory-only; there is no
browser/server persistence or hidden save. Acceptance remains Phase 137.

The visual fixture exposes a TEST-ONLY in-memory catalog: Smart Coffee API
(Atomic container @3), Magistrala (Composite @1), and P4 Sniffer (Atomic native @1).
Phase 134C expands those seeds to 100 definitions. Definition/snapshot digests are computed from deterministic canonical fixture
content; captured OCI/native pin identities are synthetic test-only values, never
claimed to be resolved production software. No production catalog is seeded.
Fixture launch commands below use the tested alternate port 18136 to avoid a
previous fixture still running on 8136. Review: Add DServ, search,
choose an exact revision, use a DIFFERENT graph-local ID, explicitly fill CPU/bytes,
inspect v2 JSON/ref/diff, connect DCall, Validate and confirm no save. Load the
legacy base and verify every original DServ remains unlinked. Empty catalog and
stale-base behavior are covered by built-app DOM regression tests.

Future Catalog Artifact Portability may export/import validated snapshots with
collision preview and explicit operator confirmation, without secrets or execution.
It is not implemented. Phase 133B's known non-blocking
`composition.discovery.compose_files:invalid_compose_path` issue is unchanged.

## Authority and audit

The editor models `CanonicalDGraphV1` (`datum.dgraph/1` and `/2`) from
`dserver/src/core/canonical_dgraph.rs`, not the older controlled composite
`DGraph` or edge-free operational deployment document. The current application
contract has ONLY DServ vertices and directed DCalls. It has no node category enum,
display labels, physical resources, runtime entities or health. V2 adds only the
optional modeling-only catalog reference described above.
Therefore the palette exposes DServ only; edges expose DCall only. Infrastructure,
runtime, data and IoT cannot truthfully appear in this editor's canonical fixture.
ServiceDefinition is not automatically a DServ.

DServ has stable service_id, host-independent datum-dnode/0 ABI, whole CPU cores
and exact memory bytes. DCall has stable call_id and exact source/target service IDs.
Cycles, reciprocal calls, self-calls and separately identified parallel calls are
legal under the canonical validator. Duplicate IDs and dangling endpoints are not.
Application ID differs from project scope. Existing IDs are read-only in edit forms;
changing identity means removing/adding an element, not silently renaming it.

DDeploy's active v1/v2 accepted application snapshots are the base source.
Read acquisition uses read_domains_only (READ_ONLY SQLite transaction), full existing
validate_persisted_state_integrity, active_placement_authority and exact recomputed
graph digest. No operational document, event or observation becomes a graph base.
Unavailable/unconfigured/corrupt sources fail explicitly, never become empty success.
An available empty source allows a new application draft. There is no independent
project registry to prove that every empty project identifier exists; empty source
does not claim project existence or ownership. Requests are explicitly project-path
scoped and only accepted graphs from that project can validate against the base.

## Console and local state

Route: `/projects/{encoded-project}/graph/edit`. Project Graph links to the editor.
The accepted base remains an immutable clone. Local draft initially uses revision
N+1 (new graph: revision 1); base stores application_id, dgraph_id, exact decimal
revision and digest. Choosing another base or starting/reloading/navigating away
requires dirty-state confirmation. Browser reload uses beforeunload. Declined
browser Back restores the prior URL with pushState; no draft is rebased or discarded.
State is memory-only: there is no server/browser draft store, hidden auto-save or
acceptance action. Closing/reloading after confirming discard loses the local draft.

Node/edge creation, edit and removal use typed forms and accessible buttons, not
drag-only controls or raw JSON editing. Removing a DServ confirms the exact incident
DCall IDs/count and removes them from the draft atomically. CPU/memory are declared
requirements, not allocations. The schema has no editable display label: IDs label
the canvas. Selecting nodes/edges opens a draft-only inspector. Accessible lists
announce category and source→target direction, with selected aria-pressed state.
Errors link to the validation summary; results focus it. No color-only controls.

Diff is deterministically ordered added/modified/removed service and call IDs/counts.
Accepted/base vs draft is textual, not health colors. Canonical payload preview and
Copy Draft JSON reflect exactly graph_json submitted to validation. All new semantics
are ordinary canonical fields; no UI side metadata changes authority.

## Payload and precision

GET `/api/v1/operations/projects/:project/dgraph-editor` returns a bounded
datum.dgraph-editor-base/1 envelope containing accepted graph_json strings and exact
base revision strings. POST `/.../dgraph-editor/validate` takes graph_json and nullable
base. It returns datum.dgraph-editor-validation/1: valid, digest, bounded findings.
The envelope is an editor read/validation bridge, not a second DGraph contract.
graph_json is canonical datum.dgraph/1 or datum.dgraph/2. Strict wrapper and exact canonical field
roundtrip reject unknown/missing fields in the editor submission.

Frontend quotes integer tokens outside JSON strings before parsing, preserving u64
memory/revision as decimal strings in the UI projection; canonicalJson emits them
back as unquoted exact integer JSON. Services/calls serialize sorted by stable ID,
consistent with canonical set/digest semantics. CPU is safe u32. No layout coordinates,
selection, project header, collapsed state or diff labels enter canonical JSON.
Canonical JCS currently rejects numbers above 2^53−1 even though model fields are u64;
these remain losslessly represented but validation returns canonical_digest_invalid,
never rounding or panicking. No canonical digest rule was changed.

## Validation and concurrency

Explicit Validate Draft reuses CanonicalDGraphV1.validate and recompute_dgraph_digest.
Zero validation findings does NOT mean accepted, placed, deployed, healthy or observed.
For existing applications base ID/revision/digest must match current accepted source
and candidate identity must be unchanged with revision N+1. New application requires
no current accepted base and revision 1. Otherwise 409 base_revision_conflict leaves
the local draft unchanged; no automatic refresh/rebase. Validation is only a momentary
check, not a reservation/lease or future acceptance authorization.

Bounds: 100 complete application bases, 2000 services/4000 calls per graph,
256 KiB graph_json, 512 KiB request envelope, 2 MiB base response, 32 safe findings,
project/identity 256 bytes, two admitted blocking readers, 10-second timeout.
The browser request has a 15-second abort deadline and navigation cancellation.
No truncated graph is offered as a complete editable base. API messages are fixed
safe text; no raw SQL, internal paths or candidate echo. Timed-out worker retains its
permit until completion. The existing trusted/local control-plane limitation applies;
before untrusted exposure authentication/authorization or an enforced trusted boundary
is mandatory. This phase does not add authentication.

## Visual and layout semantics

Existing Cytoscape dependency and graphVisualStyles centralized tokens are reused.
DServ remains purple logical software; DCall is solid directed neutral. No raw
category colors, status overlays or ownership claims are introduced. Grid/preset
layout is deterministic; existing positions, zoom and pan survive draft changes.
Moving a node is presentation-only. Fit and zoom buttons supplement pan/zoom.
Inspector stacks below the canvas at 768px. Canvas failure retains complete keyboard
forms and graph lists. 50 services/100 calls have a deterministic bounded model test.

## Tests and visual fixture

Model/client tests cover lossless payload, canonical identities, CRUD diff, incident
edges, self/reciprocal calls, duplicate/dangling/unknown fields, 50/100 elements,
explicit validation-only POST and safe conflicts. Built-app DOM tests exercise
CRUD, focus, no write before Validate, incident deletion, dirty navigation/form state,
validation findings and stale base retention. Backend tests cover canonical structural
findings, exact base conflict, strict JSON, safe digest overflow, absent database
non-creation and real HTTP GET/validate leaving the control DB byte-identical.

Terminal 1 (test-only base source, not production persistence):

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD\console
$env:DATUM_DGRAPH_EDITOR_FIXTURE_PORT = '18136'
node --experimental-strip-types tests/dgraph-editor-visual-fixture.mjs
```

Terminal 2 (stop your previous Vite before using port 5173):

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD\console
$env:DATUM_DEV_API_TARGET = 'http://127.0.0.1:18136'
npm run dev
```

Open http://127.0.0.1:5173/projects/foundation-demo/graph/edit. Fixture explicitly
simulates an accepted-looking base and validator; it does not certify server
acceptance. `/projects/empty/graph/edit` supplies a valid empty fixture source.
Add service ID `fixture-invalid` to trigger a TEST ONLY validation finding.
To simulate external base change (fixture only):

```powershell
Invoke-RestMethod http://127.0.0.1:18136/fixture/stale
```

Review checklist: load base; add DServ; connect/edit/remove DCall; edit requirements;
delete DServ and review incident edges; inspect diff; Validate; trigger fixture error;
toggle stale base and Validate; decline dirty navigation; inspect exact JSON; resize
to ~768px. No fake infrastructure/runtime palette entries are provided.

## Explicit exclusions

No placement, deploy, runtime realization, acceptance workflow, AI authority,
resource ownership inference, monitoring writes or node provisioning. No new
ServiceInstance/ComponentInstance/RequirementBinding/TenantBinding/
ProjectResourceBinding/ResourceAllocation. Phase 133B remains optional and unchanged
except its documented non-blocking Compose-path issue. Phase 135 has not started.
