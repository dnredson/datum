# Phase 133B — Guided Service Modeling

Status: implemented for independent review; unstaged/uncommitted. Phase 134 has not
started. There is no Phase 133C. OpenAI is implemented and tested with local HTTP
mocks; a paid live-model semantic test remains opt-in for the operator.

## Authority and provider audit

AI output is never authority. Candidate generation and Apply do not persist.
Only the operator's separate Save goes through Phase 133A validation, snapshot
capture, immutable revision append and expected-revision conflict handling.
Artifact != ServiceDefinition != DServ != ServiceInstance != runtime evidence.
No placement, deployment, recipes, repository clone, monitoring or binding occurs.

Audit covered `external_ai_inference`, `active_ai_context`,
`operational_analysis_context`, `ai_gnn_data_contract`, D-Engine Phase 118,
existing API routes, provider-related code/configuration and event contracts.
The historical external inference API exports graph request data and validates
externally submitted graph projections; it explicitly does not invoke a provider.
Active AI context is transport-neutral evidence, not an LLM client. D-Engine is a
derived accepted-placement view, not inference ownership. Those graph-specific
contracts are not overloaded with global catalog authoring semantics.

Audit baseline: no production canonical LLM invocation boundary, external LLM or Ollama
adapter/model configuration existed; existing issued-request correlation applies to graph
export only, not new catalog drafts. Existing structured AI response validation and
side-effect disclaimers informed this separate narrow contract. Existing model
events require model_ref, but emitting a durable event for catalog draft generation
is deferred: no operational alert/event is synthesized. Future provider auditing
must record request/prompt/model identity without intent secrets or private reasoning.

`ServiceModelingProvider` receives a bounded ProviderInput and returns bounded JSON
or a typed failure. It gets no catalog writer, executor, repository client, scheduler
or action handle. Production defaults explicitly to NotConfiguredProvider. Explicit
OpenAI selection uses an isolated adapter and existing reqwest blocking/rustls/JSON;
no SDK/dependency was added. No existing JSON Schema generator, LLM invocation,
provider retry/config subsystem needed duplication. Existing local HTTP mock testing
and environment configuration patterns were reused. Mock providers are test-only,
never a production fallback. Future Ollama uses the same trait and validator.

## OpenAI configuration and transport

`DATUM_SERVICE_MODELING_PROVIDER`: absent or `not_configured` disables generation;
`openai` requires `OPENAI_API_KEY` and `DATUM_SERVICE_MODELING_MODEL`. Missing/invalid
key/model fails startup with a fixed safe error, not fixture fallback. Unknown
providers fail startup. `ollama` is reserved but explicitly not implemented. Model
identifiers are configurable. Suggested development value: `gpt-5.6-luna`; status
does not prove account authentication or model access.

POST https://api.openai.com/v1/responses uses server-only Bearer authorization,
`store:false`, `max_output_tokens:6000` and strict `text.format` JSON Schema output.
All schema objects reject additional properties; unknown fields are nullable values,
not fake canonical identities. Static versioned authority instructions are separate
from JSON user sections OPERATOR_INTENT_UNTRUSTED, CATALOG_CONTEXT_UNTRUSTED,
SELECTED_ARTIFACT_VERIFIED_DATA, BASE_REVISION_VERIFIED_DATA and ANSWERS_UNTRUSTED.
Verified descriptive data is still data, not instructions. No tools, conversation
history, web/MCP/shell/file search or repository access are enabled. Reasoning output
is ignored, never exposed/stored. The canonical ServiceDefinition remains stricter.

HTTP deadline 25 seconds, connect deadline 5 seconds; redirects disabled. No retries,
including 429. Successful envelope cap 128 KiB; candidate JSON cap 64 KiB.
Noncompleted/refused/empty/malformed/schema-invalid output fails closed; then the
provider-neutral semantic/identity validator runs. 401/403 -> NotConfigured;
429 -> RateLimited; 5xx/network -> Unavailable; timeout -> Timeout;
other 4xx/invalid output -> InvalidResponse. No raw provider body, credential/header,
debug dump or stack is logged/reflected. API key goes only in the outbound header.

Payload includes intent, bounded catalog summaries, selected safe artifact pins,
exact base metadata and answers. It excludes resolved secrets, wholesale DB/DMonitor
history, browser state, filesystem, repository content and provider key in JSON.
Operators must not put sensitive values into intent. Durable generation audit remains
deferred. Future execution requires authorization/policy; saved recipes grant none.

API sources: [Structured Outputs](https://developers.openai.com/api/docs/guides/structured-outputs)
and [GPT-5.6 Luna capabilities](https://developers.openai.com/api/docs/models/gpt-5.6-luna).

## API and draft contract

- GET `/api/v1/catalog/modeling-assistant`: typed Available/NotConfigured status.
- POST `/api/v1/catalog/modeling-assistant/draft`: non-mutating generation.

Strict requests carry intent, mode (`new_definition`/`new_revision`), optional exact
base, optional selected artifact source, question answers and exact definition
references. Server resolves references; client cannot submit catalog authority context.
Contract: `datum.service-modeling-candidate/1`.
Prompt: `datum.service-modeling-assistant/1`.

CandidateOutput contains partial_definition, assumptions, unresolved_questions,
warnings, field provenance, unsupported_requests and concise rationales. No numeric
confidence score or chain-of-thought is requested, retained or exposed. Metadata
sources are operator_input, existing_catalog/catalog_context, selected_artifact,
system_default and model_inference. Questions have unique bounded question_id,
exact field path and matching text; no positional join to unresolved fields occurs.
Repository_metadata provenance is not offered because repository analysis is absent.
Provider/model references identify provenance only, never authority.

Unresolved secret reference/version metadata can remain null with a known target
environment variable. It never needs a fabricated reference or value. Reference
names/versions must be operator supplied or retained from the exact base. Questions are
restricted to modeling fields; secret questions may ask reference/version/target
metadata, not secret-value fields. Incomplete secret metadata blocks editor Save.

Partial candidates are separate from canonical ServiceDefinition. Missing facts remain
null/absent. Canonical validators are not weakened. Complete candidates decode to the
Phase 133A type, use its validator and verify exact reference existence; partial
shapes/known enums and selected identities are checked separately. Invalid output
returns a safe error, not a partially trusted or default candidate. Valid and
Incomplete validation are distinct from AI warnings/questions. Incomplete candidates
must include questions. Commit pins must come from explicit operator input/answers
or be retained from the exact base, never fabricated. Artifact identities must match
the server-verified selected source; no image/digest import or resolution occurs.

## Context and bounds

### Safe provider-candidate rejection diagnostics

Parseable candidate failures now preserve typed path/code/message findings instead
of collapsing into InvalidResponse. HTTP remains 502 with error.code
`provider_invalid_candidate`, a fixed message and `findings`. Bounds: 16 findings,
path 128 bytes, code 64 bytes, message 256 bytes. Paths use known schema names and
bounded indices; unknown provider keys/enum values and serde error text are never
echoed. Truly malformed JSON/envelope/refusal still uses safe InvalidResponse.
The already-transitive serde_path_to_error dependency is explicitly declared for
structural paths, without introducing a new package/version.

WARN records contain configured provider/model, a server-generated request UUID,
and bounded paths/codes only. No raw candidate, provider body, prompt, secret value,
Authorization header or private reasoning is logged. Browser rendering uses safe
text nodes, removes rejected candidate Apply controls and says no draft was applied.
The manual editor is not overwritten. No production candidate-capture hook exists.
Validator acceptance rules remain unchanged. Provider instructions and schema now
encode the Atomic/Composite root invariant explicitly; no candidate normalization
or field dropping is performed.

Mock HTTP regression: Magistrala Composite/Discovered/DockerCompose,
SharedTenantAware/domain, RepositoryCommand URL plus make/run_latest and null commit
is accepted as Incomplete. Empty/main/short commits, traversal paths, Composite root
runtime/artifact, bad enum spellings, extra fields and bad question roots reject with
safe diagnostics. A syntactically valid guessed relative Compose path passes as a
declaration only; no repository verification occurs. No fabricated discovery is proven.

The live Magistrala failure was Composite plus non-null runtime_kind. The previous
schema independently allowed nullable runtime/artifact for all shapes; instructions
did not explicitly require null composite root fields. The provider schema now uses
nested anyOf branches inside partial_definition (not at the response root): Atomic
requires null composition and permits known/unresolved runtime and artifact;
Composite requires null runtime/artifact and non-null composition; unresolved shape
requires null root fields and a question. All branch fields remain required/strict.
No if/then/else, oneOf or other unsupported conditional keywords are used.
Instructions include positive/negative Magistrala, Smart Coffee and P4 examples plus
a compliance self-check without requesting reasoning. Multiple commands in a single
container do not imply Composite. Canonical validation still rejects contradictory
output with the existing safe findings. Model compliance must be retested live.

The independently confirmed HTTP health schema gap is also repaired: HTTP requires
an explicit string path, TCP requires null path. A complete HTTP /health declaration
passes canonical validation; missing/invalid paths still reject. No health semantics
or validation authority changed. Schema subset reference:
https://developers.openai.com/api/docs/guides/structured-outputs

Server reads a bounded catalog page. Advisory relevance uses stable IDs/capabilities
and intent terms, not fuzzy canonical identity joins. Summaries contain ID, exact
revision, untrusted display name, shape, tenancy, provided capabilities and runtime.
Only the selected artifact pin is included. Exact requested definitions are resolved
server-side. Catalog names remain untrusted data. A new-revision base is the actual
exact revision, including typed secret references but never resolved values.

Limits: intent 8192 UTF-8 bytes; request body 32768 bytes; answers 8 (field 128 bytes,
value 2048 bytes); explicit references 20; context summaries at most 20, deterministic
ID/revision ordering; serialized context at most 32768 bytes; one selected artifact;
provider JSON at most 65536 bytes; metadata lists at most 32, strings 512 bytes;
provenance at most 64 with 128-byte field/256-byte reference. Repository files/bytes
and conversation history: zero. Follow-up regenerations are independent requests;
only up to eight explicitly answered fields are submitted. No hidden conversation
history is authoritative. Context truncation is explicit to the provider.

Two owned blocking workers and a 30-second response deadline bound server admission.
Timeout does not terminate arbitrary synchronous provider code: the permit remains
owned until the worker completes. Future adapters must bound their own external I/O.
OpenAI owns a shorter 25-second HTTP deadline. Browser cancellation/stale sequence handling prevents late application, but does not
promise forced interruption of a synchronous server worker. Provider failures include
NotConfigured, Unavailable, Timeout, InvalidResponse, RateLimited and Indeterminate.

## Modeling semantics and examples

Magistrala: a Composite/Discovered SharedTenantAware/domain product; DockerCompose
and RepositoryCommand argv `['make','run_latest']` are declarations. If the source
commit is unknown, ask for it. No fake root image, invented SHA, members, shared
instance or tenant domains. Git alone does not pin resulting images; future execution
must capture effective Compose configuration, resolved image identities and provenance.

Smart Coffee: Atomic app with database.postgresql and messaging.mqtt requirements.
An external database requirement does not turn the API into a Composite. An existing
catalog provider can be mentioned, but is not bound or converted to exact dependency.
Unknown artifact remains unresolved.

P4 Sniffer: Atomic Dedicated, exclusive physical network-interface requirement and
30-second monitoring intent. Physical requirement != service capability != reserved
interface != exclusive DNode ownership. Health and monitoring remain separate.

Real P4 lifecycle gap: image `dnredson/p4d` and label `sw1` are not a canonical pinned
artifact or current runtime identity. docker exec/sysctl-like changes, simple_switch
startup and P4 CLI table programming are unsupported lifecycle/post-start modeling
requests. The provider must report them, not drop them or force RepositoryCommand.
Potential future RuntimeLifecycle/PostStartAction/typed sysctl/process/P4 actions need
independent modeling and policy review. None is implemented here.

Static components and exact dependencies require known exact revisions. No latest
fallback. Requires != provides; neither proves a current provider instance. SystemGlobal
is not root privilege or authorization bypass. HTTP and TCP are the only current
health declarations. Tenancy suggestions must be labeled, not facts about sharing.

## UI, Apply, review and Save

Global Catalog retains its manual editor. A guided panel contains intent, typed
preview, rationale, assumptions, unresolved question inputs, warnings and provenance.
Production NotConfigured is explicit and manual editing stays functional.
Candidate generation does not blank the manual editor. Apply is a separate button,
then a not-saved banner accompanies the editable candidate. Missing IDs, artifact
selection and commits remain empty; no fake canonical record/revision is constructed.
Shape/tenancy omissions require an explicit choice. Composite commit validation blocks
incomplete Save before POST; server validation remains final authority for all writes.

Generation replaces the separate candidate, not manual fields. Apply over dirty edits
requires an explicit Replace confirmation. Expected base revision remains unchanged;
conflict requires operator reload/review, not automatic rebase or model regeneration.
Review produces deterministic changed-section summaries, including composition/discovery,
artifact, dependencies/capabilities, resources, health, monitoring, tenancy and secrets.
Historical revisions remain disabled/read-only and exact pin inspection is preserved.

Native labels/buttons, focused review/confirmation controls, meaningful async status
and text-based validation support keyboard use. Candidate render does not remove the
intent control. At 768px catalog cards stack; JSON preview wraps rather than forcing
a horizontal mega-table. This is DOM verification, not graphical certification.

## Security and self-audit

Operator/catalog descriptions are delimited in untrusted_data, separate from static
system_rules. Prompt injection is untrusted text, not permission. Providers cannot
perform actions through this interface; validation, no writer handle and explicit
Save remain independent barriers even if a model ignores instructions.

No repository inspection is implemented: no fetch, clone, parser, Makefile, Compose
or scripts run. Repository URLs are modeling text only. Future inspection requires
explicit action, bounded immutable file evidence and prompt/data separation.

Provider credentials remain server-side and never reach the browser. Resolved
secret values are never loaded into context. Typed secret references only; no paste
secret field. Unknown secret-value properties and obvious argv credential assignments
are rejected. Arbitrary strings cannot be guaranteed secret-proof; operators must
not put credentials in intent, names, argv or explanations. Future execution requires
typed secret injection, authorization/policy and isolation, not AI approval.

Catalog and assistant APIs remain trusted/local control-plane only. DServer's default
network exposure is NOT safe for untrusted clients. Authentication/authorization or
an enforced external trusted boundary is mandatory before untrusted exposure.

Network paths added: browser→DServer status GET/draft POST and explicitly configured
DServer→OpenAI Responses POST. No browser→LLM or repository network path exists. Existing explicit catalog Save is
unchanged. `save` matches refer to that operator action or tests proving zero rows;
`docker`/`Command`/deployment matches are enum metadata, rules, comments or test data.
No executor/process shell/DMap/scheduler mutation exists in Phase 133B paths.

Known limitations: live-model behavior/account access not certified; no Ollama or repository
analysis; no durable generation event; existing compact editor's first-entry component/
dependency/resource/secret editing limitation remains, preserving additional entries.
A future production provider needs independent security/provenance review and opt-in
live smoke tests, never external network dependencies in the normal suite.

## Validation and test-only visual fixture

Run `npm test`, `npm run build`, `npm audit`, the existing built-app DOM/freeze gate,
`cargo test --manifest-path dserver/Cargo.toml --bin DServer service_modeling`,
focused service_catalog tests, full DServer tests/build and `git diff --check`.
Tests cover status/errors/output bounds, admitted artifacts/exact refs, zero writes,
incomplete Magistrala/Atomic DB/P4, injection as data, missing pins, typed secrets,
history/conflicts, context bounds, Apply/Save separation, dirty state, cancel/stale
responses, focus, labels and manual catalog regression.

Hardening results: 113 frontend tests, 100 built-app DOM/freeze tests, 32 focused
modeling tests (including 9 OpenAI transport/config/schema tests), 31 focused catalog
tests passed. Full DServer: 2317 passed, zero failed, one ignored. Strict TypeScript/
Vite build passed. These results are not paid live-provider or graphical certification.
Default production remains explicitly NotConfigured.

Manual fixture (three PowerShell terminals; no production DB required):

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD\console
node --experimental-strip-types tests/service-modeling-visual-fixture.mjs
```

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD\console
$env:DATUM_DEV_API_TARGET='http://127.0.0.1:8134'
npm run dev
```

Open http://127.0.0.1:5173/catalog. Use exact preset keys `magistrala`, `smart-coffee`,
`p4-sniffer`, `incomplete`, `invalid`, `unsupported-lifecycle`, `not-configured`.
Arbitrary free-form text returns fixture_no_scenario; it is not keyword modeled.
The UI says TEST PROVIDER / deterministic fixture / no real AI request was made.
Questions are field-specific; Coffee has no P4 resource/monitoring defaults.
Magistrala asks for a commit; Coffee asks for an artifact. P4 is a complete mock
draft. Enter `invalid` for InvalidResponse or `not-configured` for a generation
failure. Start fixture with `--not-configured` to inspect disabled availability.
All generated data is fixed TEST-ONLY output, not production AI. Explicit fixture
Save affects disposable in-memory history only; restart discards it. No service runs.
Stop any previously running Vite on 5173 before launching this fixture proxy.

## Opt-in real OpenAI smoke (PowerShell 7)

Known non-blocking live-provider issue: OpenAI may infer an unverified Docker Compose
path, causing deterministic invalid_compose_path. Validation correctly rejects that
draft. Guided modeling is optional; this issue is not being fixed in Phase 134.

Startup wiring was tested with a TEST-ONLY credential and status GET, never a paid
request. For real generation, supply your own key through masked input. These
commands isolate persistence and bind DServer to loopback; no service is deployed.

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD
$modelingSmoke = Join-Path ([System.IO.Path]::GetTempPath()) ('datum-modeling-live-' + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $modelingSmoke | Out-Null
$env:DATUM_DSERVER_CONFIG_PATH = (Resolve-Path console/tests/service-modeling-live.config.toml).Path
$env:DATUM_CONTROL_STATE_DB_PATH = Join-Path $modelingSmoke 'control.db'
$env:DATUM_SERVICE_CATALOG_DB_PATH = Join-Path $modelingSmoke 'catalog.db'
$env:DATUM_SERVICE_MODELING_PROVIDER = 'openai'
$env:DATUM_SERVICE_MODELING_MODEL = 'gpt-5.6-luna'
$env:OPENAI_API_KEY = Read-Host 'OpenAI API key (masked; never saved)' -MaskInput
./dserver/target/debug/DServer.exe
# After Ctrl+C:
Remove-Item Env:OPENAI_API_KEY
```

Other terminal (stop the old fixture-targeted Vite first):

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD\console
$env:DATUM_DEV_API_TARGET = 'http://127.0.0.1:8080'
npm run dev
```

Open /catalog: status shows openai/configured model, no fixture presets. The isolated
catalog starts empty. Atomic artifact pins stay unresolved unless actual canonical
artifacts are available; do not invent them. Generate/Apply never write rows. Save
is separate, optional and catalog-only. The startup/status commands were tested;
actual paid generation requires the operator's own credential and access.
If 5173 is occupied, stop your own prior fixture session; the agent does not kill
pre-existing user processes. Headless gate can set
`DATUM_DOM_GATE_API_BASE=http://127.0.0.1:8080` independently of that Vite proxy.

No Phase 134 editor, 135 continuum, 136 placement, 137 operational acceptance,
138 execution or 139 runtime workspace has been implemented. AI modeling drafts
are not DMap proposals. Phase 133B remains uncommitted for independent review.
