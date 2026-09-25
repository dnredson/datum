# ADR-0025 — DScript v1 non-functional annotation representation

- Status: Implemented for Phase 116B (`CanonicalDScriptV1`/
  `CanonicalDFunctionV1`, `dserver/src/core/canonical_dscript.rs` and
  `DATUM/src/agent/canonical_dscript.rs`).
- Scope: the wire/type representation of a D-Function's "non-functional
  annotations" (ADR-0001's own phrase) in `datum.dscript/1`. Nothing else
  about the DScript contract is decided by this ADR — see
  `documentation/reference/phase116b-canonical-dscript-v1.md` for the full
  contract.

## Context

`ADR-0001-canonical-datum-vocabulary.md` defines D-Script as "a set of
D-Functions with non-functional annotations," but neither that document nor
any other repository artifact defines what those annotations contain.
Phase 116A (`documentation/reference/phase116a-dscript-architecture-audit.md`,
Section 4) confirmed this gap explicitly: no evidence anywhere supports
giving annotations a specific domain meaning (CPU/memory requirements,
latency targets, QoS, locality, placement hints, retries, timeouts,
readiness dependencies, or security requirements).

This decision needs architectural permanence, not just a Phase 116B
implementation note, because:

1. It is a wire-format commitment. Once fixtures, tests, and (eventually)
   real DScript documents exist against a chosen shape, changing it is a
   breaking schema change, not a private refactor.
2. It sets the boundary for what ANY future annotation content is
   structurally allowed to be, for as long as `datum.dscript/1` exists —
   a decision later phases (117 DCompile onward) will build against and
   must not silently reinterpret.
3. The task that produced it (Phase 116B) explicitly flagged this as the
   one place a wrong default could quietly reintroduce exactly the kind of
   invented domain semantics, or authority-smuggling catch-all, this
   project's architecture has consistently refused elsewhere.

## Decision

`CanonicalDFunctionV1::annotations` is a `BTreeMap<String, String>`:
a deterministically-ordered map from an opaque annotation name to an
opaque string value. No key is reserved or given meaning by this contract.
No key or value is ever interpreted, validated beyond non-emptiness of the
key, or acted upon by `canonical_dscript.rs` or any other module.

Rejected alternatives:

- **`serde_json::Value` / `BTreeMap<String, Value>`** — explicitly rejected.
  An arbitrary-JSON value can recursively encode a nested object, meaning a
  caller could embed a `{"node": "..."}`-shaped placement hint, an
  executable-content blob, or a control-flow-shaped structure inside an
  "annotation" with no type-level barrier stopping it. This would reopen,
  at the type level, exactly the negative-space DScript must respect
  (Phase 116A Section 16): no placement, no executable identity, no
  control flow. A flat `String` value cannot represent any of those no
  matter what text a caller supplies — the restriction is structural, not
  a matter of validation discipline that could later be relaxed by mistake.
- **Bare tag list (`Vec<String>`, no values)** — considered as the most
  minimal possible representation, but rejected as under-fitting the
  canonical definition's own wording ("annotations," not "flags" or
  "capabilities"). A name-only representation cannot express even the
  simplest real non-functional fact (e.g. a label and a value for it)
  and would likely need to be replaced wholesale the moment any real use
  emerges, making it a worse foundation than a name→opaque-value map.
- **A typed enum of known annotation kinds** — rejected outright: no
  repository evidence identifies any specific kind, and enumerating
  invented kinds (cpu/memory/qos/etc.) is exactly the domain-semantics
  invention this phase was explicitly instructed to avoid.
- **`HashMap<String, String>`** — rejected in favor of `BTreeMap`: this
  project's canonical-contract discipline requires deterministic
  serialization order (see `canonical_dgraph`'s own array-ordering digest
  precedent); an unordered `HashMap`'s iteration order is not guaranteed
  stable across runs/versions and must never leak into a canonical digest
  or wire representation.

## Consequences

- `datum.dscript/1` can express a small, flat set of named facts about a
  D-Function today, and no more. This is a deliberate limitation, not an
  oversight (Phase 116B, Section 5).
- Any future phase that needs richer annotation structure (e.g., a typed
  resource requirement, once a phase produces actual evidence for one)
  must make that its own new, evidence-driven decision — it cannot smuggle
  it in by simply changing an annotation's value type, because doing so
  changes `datum.dscript/1`'s wire shape and therefore its digest identity
  for every existing document.
- `recompute_dscript_digest` (both crates) includes `annotations` in its
  digest input unmodified — an annotation is part of a D-Function's
  identity, exactly like `function_id` itself. This is proven by
  `digest_changes_when_function_semantics_change` in both crates' test
  suites.
