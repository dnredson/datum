# Stage 78 — E2E service-artifact catalog

This directory defines the declarative service artifacts for the planned distributed IoT
proof: a simulated LoRa device on `mist-01`, a virtual Semtech UDP gateway path and
ChirpStack on `fog-01`, and Magistrala plus Smart Adapter on `cloud-01`.

Stage 78 registers metadata only. It does not clone repositories, build images, render
configuration, start containers, contact a node, or execute WASM. Deployment remains
blocked until the artifact-to-SmartSentinel execution path is implemented and explicitly
authorized in a later stage.

## Files

- `scenario.json`: node addresses, artifact membership, relations, expected data flow and secret references.
- One JSON file per software component: each file can be POSTed to the dserver artifact registry.
- Images from the official Magistrala and ChirpStack compose definitions are recorded in
  the artifacts. The mist images use the `local/` namespace and are intentionally marked
  `build_before_deploy`; they are not presented as existing public images.
- No credential values are stored. `secret_refs` and `bindings` identify values that a
  later governed deployment must resolve.

## Import contract

The dserver request header must select the project scope. The server normalizes the
artifact's control-plane project to that scope and persists it in the configured control
state database. Import order should follow `control_plane.dependencies`; duplicate IDs
return HTTP 409 and can be updated with PUT.

The intended next stages are:

1. validate this catalog and materialize the static operational D-Graph;
2. make dserver distribute only the selected artifact and its resolved bindings;
3. invoke WASM for a decision;
4. let SmartSentinel perform the explicitly authorized local deployment;
5. validate the real data path end to end.

Always expect `safe_to_auto_execute=false` and `side_effects_performed=false`
from the catalog and registry responses.
