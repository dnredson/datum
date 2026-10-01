# DATUM Graph Visual Semantics

Stable Console presentation/design contract, established after Phase 131 and
before Phase 132. This contract creates **no operational authority** and does
not introduce runtime, continuum, placement, monitoring or tenancy data models.
Canonical/read-model source semantics always take precedence over presentation.

## Entity categories

| Token | Category / family | Shape | Meaning |
| --- | --- | --- | --- |
| `infrastructure` | Infrastructure · blue `#416d9c` | Rounded resource rectangle | DNode or explicitly modeled physical/virtual execution resource; cloud/fog/mist describes supported resource roles, not ownership |
| `software` | Logical software · purple `#7852a1` | Component rectangle | DServ / logical application component; not a running instance |
| `runtime` | Runtime realization · cyan/teal `#258a96` | Compact rounded rectangle | Explicitly projected container/native/WASM/realized instance; never inferred from a logical service |
| `storage` | Data/storage · green `#357e58` | Storage-oriented barrel | Explicitly modeled database/volume/state store; never synthesized from arbitrary service metadata |
| `iot` | IoT/device · amber `#b58126` | Device-oriented hexagon | Explicit canonical/read-model DIoT/sensor/actuator/endpoint; never fabricated from labels or metadata |

Categories retain these identities across graph phases. Equal shape families are
disambiguated with size and explicit category/entity text. Entity type never
depends on color alone. Current supported rendering maps only DServ to software.
The existing accepted DGraph source is unchanged; DCode/DIoT are not synthesized.

Application compound groups are **presentation-only containers**, not canonical
vertices or a sixth entity category. Neutral pale fill `#f0f3f6`, grey border
`#98a3b0` and container text remain visually subordinate to the contained entities.
Grouping never changes logical relationships or identity.

## Relationship categories

| Token | Line / target arrow | Meaning |
| --- | --- | --- |
| `logical` | Solid neutral-blue/grey `#71859a` / triangle | DCall: directed logical invocation, **not** deployment order or physical connectivity; cycles, reciprocal calls, parallel calls and self-calls remain valid |
| `realization` | Solid teal / open vee | Logical service → runtime instance; distinct from DCall |
| `placement` | Dashed neutral-blue/grey / triangle | Runtime/workload → resource; never exclusive ownership by inference |
| `physical` | Solid neutral infrastructure grey / no arrow by default | Physical/network relation only; future directed relations must follow their actual typed source semantics |
| `observation` | Dotted subtle grey / no arrow by default | Observational provenance; not invocation, realization or placement |

Use line style, arrow style and an explicit legend label together, not only color.
Only DCall is active in the current graph source. All other relations are reserved
presentation vocabulary, not implemented routes, data, joins or topology.

## Status overlays are orthogonal

Category fill and shape must **not change with operational status**. A critical
DServ stays purple software, with a separate red outline/halo/badge and textual
state. A positive state never turns logical software into a green entity.

Reserved overlay tones:

| Token | Tone | Evidence requirement |
| --- | --- | --- |
| `normal` | Green `#357e58` | Known positive/normal state supported by the appropriate authority; absence of alerts is insufficient |
| `attention` | Yellow `#c5a52b` | Explicit attention/indeterminate presentation, with reason |
| `degraded` | Orange `#c87526` | Explicit degradation/high attention, with source and dimension |
| `critical` | Red `#b33d48` | Explicit critical/failure evidence; does not convert alerts into runtime authority |
| `neutral` | Grey `#7b8691` | Unknown/not-configured/neutral, not numeric zero or positive health |

The shared overlay helper produces outline-only properties; it never sets fill,
shape, health, authority or ownership. No operational overlays are applied to the
current structural graph. Future consumers must combine supported typed evidence
with accessible status text and preserve separate temporal dimensions.
Selection/focus is an interaction marker, not an operational status.

Storage green and device amber denote categories, **not normal or warning states**.
Shape, entity text, source semantics and separate overlay text distinguish them.

## Single semantic source of truth

`console/src/components/graph-visuals.ts` owns category/relation/status token maps,
the Cytoscape stylesheet and reusable textual legend rendering. Graph pages must
not duplicate raw category colors or derive categories from human prose.

The current DTO-to-element adapter adds presentation-only `visual_category` and
`visual_relation` values from typed DServ/DCall kinds. They do not modify the
backend DTO or canonical identities. Existing graph validation, source coverage,
accepted provenance, grouped deterministic layout, Fit, Reset and filtering remain
unchanged. Headless layout tests now use the production shared stylesheet.

`graphLegendItemsFor` accepts explicitly supported present category/relation sets;
it deduplicates them in stable contract order. Future source-specific adapters can
reuse this builder and `renderGraphLegend`. The current `graphLegendItems` adapter
recognizes only existing DServ/DCall/grouping types. Filtering updates the legend
for the visible set. Unused reserved categories never appear in the current legend.

CSS controls swatch geometry/spacing; token maps supply swatch colors and relation
styles. Decorative swatches are hidden from screen readers while adjacent text
preserves the complete semantics.

## Accessibility and source honesty

- Color + shape + text identify categories; line + arrow + text identify relations.
- Keep full canonical identity inspectable in the accessible list/details even
  when canvas labels are shortened.
- Maintain keyboard-accessible alternative structure and selection controls;
  canvas rendering alone is not a full accessibility claim.
- Show only types actually present, including after filtering. A missing category
  is not proof the entity does not exist outside the current source/preview.
- Preserve Available/Partial/NotConfigured/Unavailable/Corrupt/Indeterminate and
  accepted snapshot provenance; visual tokens never imply complete inventory.
- Do not infer liveness, convergence, connectivity or positive health from colors,
  placement, structural presence or absence of alerts.

## Multi-project and shared-service compatibility

Infrastructure category means **resource**, not "owned by this project". Logical
project/application graph != physical resource ownership. Future global DNodes,
ProjectResourceBinding, reservations, exclusive/partial allocation and resources
bound to multiple projects remain compatible; no such model is implemented here.

A tenant-aware shared service remains software and/or runtime according to the
entity represented. Sharing does not reclassify it as infrastructure. Future
shared/exclusive tenancy is an independent typed badge/marker, never a category
color or ownership inferred from graph membership.

## Reuse and phase boundaries

- Phase 132: runtime graph adopts the vocabulary only for entities/relations its
  own canonical/read-model contract actually supports.
- Phase 134: visual editor reuses categories and relations without making styling
  canonical authority.
- Phase 135: continuum view uses infrastructure categories for supported resources.
- Phase 136: placement uses distinct dashed relations backed by explicit intent/
  acceptance semantics, never inferred from DCall or visual position.

This change is presentation only: no backend changes, new dependencies, runtime
expansion, continuum topology, placement, tenancy, allocation or monitoring overlay
implementation. Phase 131's source and authority contracts are not reopened.
