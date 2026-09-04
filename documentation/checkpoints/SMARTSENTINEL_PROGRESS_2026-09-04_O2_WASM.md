# SmartSentinel progress checkpoint — O2 operational D-Graph/WASM

Date: 2026-09-04
Branch: `agent/o2-host-capability-discovery`
Progress index: **47/100**

## Architecture invariants

- SmartSentinel observes local resources/runtime/network state, publishes evidence, reconciles authorized desired state, and reports outcomes.
- dserver is the control plane: it governs resource binding, distributes the relevant operational D-Graph node slice, and authorizes host mutation.
- DATUM is the conceptual framework; DATUM itself does not execute host actions.
- The D-Graph is generated externally. A future AI placement/reasoning component consumes SmartSentinel telemetry and other evidence to decide placement and produce/update the D-Graph.
- The SmartSentinel does not redo the global placement decision. It periodically compares the received desired state with observed local state and reconciles differences.
- WebAssembly is the sandboxed local reconciliation step. The guest does not receive shell access or direct Docker host imports.
- Artifact-specific configuration such as ports, volumes, environment, Mosquitto configuration, package-specific pre/post configuration, and other deployment details is deliberately deferred to the artifact model.

## Live PASS evidence closed at this checkpoint

### Role-free resource discovery

On `fog-01`, SmartSentinel starts from a configuration without a node role and derives the provisional OS resource identity:

`resource:os:9d564a4d83804cff3b62e84394a8da53d8610c717cd892623a2242ebaabb6e09`

The resource can exist as `available_unbound` with no assigned DATUM role.

### Governed resource-to-role binding

The dserver successfully performed an explicit initial assignment:

- resource → `fog-01`
- binding state: `active`
- resource state: `bound`
- `automatic_assignment_performed=false`
- `control_plane_authorized=true`

The SmartSentinel subsequently bootstrapped as `known_bound_node` even though it started without a configured role.

### Operational D-Graph node slice

The external operational D-Graph was accepted by dserver and the slice delivered to `fog-01` contained exactly one desired service:

- `service_id=mqtt`
- `runtime=container`
- `desired_state=running`

The active reconciliation authorization was bound to the same project, node, and `dgraph_id`.

### Real Wasmtime reconciliation

The operational reconciler executes through Wasmtime with no host imports.

Validated behavior:

- desired running + observed missing → `action_code=1`
- desired running + observed running → `action_code=0`
- `host_imports_allowed=false`
- `guest_side_effects_performed=false`

### Real MQTT deployment

With Docker available to the SmartSentinel user, the first operational reconciliation produced:

- `bootstrap_state=known_bound_node`
- `node_id=fog-01`
- `dgraph_id=dgraph:o2-mqtt-role-1`
- `control_plane_authorized=true`
- `local_container_execution_allowed=true`
- `service_id=mqtt`
- `observed_before_running=false`
- `wasm_action_code=1`
- `execution_status=deployed`
- `observed_after_running=true`
- `host_mutation_performed=true`
- `snapshot_published=true`
- `side_effects_performed=true`

Docker confirmed the real container:

- name: `datum-mqtt`
- image: `eclipse-mosquitto:2`
- state: running

### Idempotent second reconciliation

A second reconciliation over the same D-Graph produced:

- `observed_before_running=true`
- `wasm_action_code=0`
- `execution_status=converged`
- `observed_after_running=true`
- `host_mutation_performed=false`
- `side_effects_performed=false`

This closes the first live proof of:

`resource without role → binding → D-Graph slice → Wasmtime reconciliation → governed host mutation → observed convergence → no-op subsequent reconciliation`

## Docker runtime prerequisite observations

The fog VM exposed a useful intermediate runtime state:

1. Docker CLI installed, but current user lacked daemon socket permission.
2. SmartSentinel reported `installed_but_unavailable` rather than `missing`.
3. After adding the execution user to the `docker` group and refreshing the session, SmartSentinel reported `available`.

The runtime prerequisite implementation distinguishes:

- `available`
- `installed_but_unavailable`
- `missing`

Supported typed Linux package-manager detection currently includes:

- `apt-get`
- `dnf`
- `yum`
- `pacman`
- `zypper`

A thesis-aligned runtime artifact fixture records the intended lifecycle:

`pre_install_commands → install_command → post_install_commands`

For Docker, the post-install phase is the correct place to enable/start the service and grant the SmartSentinel execution user access to the Docker group.

This artifact lifecycle is recorded but not yet integrated into the continuous operational reconciler.

## Important bug fixed during live validation

Docker 29 returned a missing-container error as:

`error: no such object: datum-mqtt`

The initial parser compared only capitalized variants and misclassified the result as Docker unavailable. Missing-container detection is now case-insensitive and correctly maps the response to `ContainerState::Missing`.

## Deliberately deferred

The following are intentionally not part of the current generic reconciler checkpoint:

- MQTT host port publication / listener configuration
- `mosquitto.conf` management
- volumes and environment variables
- container restart policies
- artifact-specific commands
- package-native Mosquitto installation
- artifact-specific configuration drift detection

These belong to the future artifact specification/executor layer, because different artifacts require different installation and configuration semantics.

## Current limitations

- Resource Registry is process-local.
- Resource bindings are process-local.
- Operational D-Graph storage is process-local.
- Reconciliation authorizations are process-local.
- Restarting dserver therefore requires rehydrating current O2 operational state.
- `scripts/o2-rehydrate-operational-state.sh` exists to reproduce that state during this phase.
- Cloud-01 current O2 live validation is still pending.
- Controlled rebind/hardware replacement lineage is not yet implemented.
- Runtime prerequisite installation is not yet integrated into the periodic reconciliation path.
- Operational reconciliation is not yet folded into the persistent SmartSentinel service loop.

## 100-step progress map

- **01–10 — Architecture/frozen foundations: DONE**
  - DATUM/dserver/SmartSentinel responsibility boundaries
  - Controlled WASM Deployment historical phase remains closed

- **11–20 — O1 continuous SmartSentinel observation loop: DONE**
  - persistent service loop, safe observation, lifecycle controls

- **21–35 — O2 identity/resource/bootstrap/binding: DONE for current fog proof**
  - typed discovery
  - hardware/OS/capacity identity
  - role-free resource discovery
  - Resource Registry
  - three-state bootstrap
  - governed initial resource→role binding

- **36–47 — Operational D-Graph + WASM live vertical slice: DONE / LIVE PASS**
  - operational node slice
  - reconciliation authorization
  - real Wasmtime decision
  - real Docker MQTT deployment
  - snapshot publication
  - idempotent second reconciliation
  - Docker runtime state classification and package-manager prerequisite groundwork

- **48–55 — Continuous operational reconciliation: NEXT**
  - integrate bootstrap + D-Graph polling into persistent SmartSentinel loop
  - detect D-Graph revision changes periodically
  - run reconciliation only when needed / periodically for convergence
  - integrate runtime prerequisite resolution into the operational path
  - preserve deterministic evidence of deploy/redeploy/reconfigure outcomes

- **56–65 — Durable control-plane state and binding lineage**
  - persist registry/bindings/D-Graph/authorizations
  - remove restart rehydration requirement
  - controlled rebind
  - hardware replacement successor/predecessor binding history

- **66–75 — O3/O4 runtime/service/network observation expansion**
  - richer runtime/service evidence
  - network interfaces/control-plane RTT
  - observed endpoint/configuration state needed for later reconfiguration

- **76–85 — O5/O6/O7 telemetry and live D-Graph intelligence**
  - peer reachability/RTT
  - telemetry ingestion/history
  - live desired-vs-observed graph
  - feed telemetry/evidence to placement reasoning

- **86–95 — AI placement and adaptive reconfiguration/failover**
  - placement/reasoning consumes deterministic telemetry
  - D-Graph changes become authoritative desired-state updates
  - examples include moving MQTT output from UFABC infrastructure to AWS after upstream failure
  - controlled data-path failover and recovery

- **96–100 — Reproducibility, closure, end-to-end proof**
  - failure/recovery reproducibility
  - preferred-node restoration
  - documented evidence bundle
  - final architecture validation

## Current marker

**47/100 — first real operational D-Graph/WASM deployment and idempotent reconciliation live-validated.**

Next marker target: **48/100 — move the operational reconciliation from a manually invoked binary into the persistent SmartSentinel service loop without changing the already validated D-Graph/WASM execution semantics.**
