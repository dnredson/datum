# Hardware, role binding and historical telemetry semantics

DATUM distinguishes the stable logical role of a node from the concrete hardware, operating-system installation, capacity profile, and SmartSentinel process currently realizing that role.

This is required for long-lived IoTinuum management. A logical node such as `fog-01` can survive hardware replacement, OS reinstall, resource upgrades, and agent restarts without losing its historical context. At the same time, those changes must remain visible so AI/GNN/planner consumers do not treat heterogeneous observations as one homogeneous machine.

## Identity dimensions

```text
node_id
  stable logical DATUM role/participant

hardware_fingerprint_id
  one-way fingerprint of the physical/virtual compute resource when trustworthy anchors are observable

os_installation_id
  one-way identifier of the installed OS instance

capacity_profile_id
  structural capacity profile currently exposed by the resource

role_binding_id
  optional candidate relationship binding node_id to observed hardware

instance_id
  one concrete SmartSentinel process lifetime
```

### Logical role: `node_id`

`node_id` is the stable DATUM/D-Graph role identity. A replacement computer can continue to be `fog-01` if the control plane deliberately binds the replacement hardware to that role.

Telemetry history by `node_id` therefore remains continuous across legitimate replacement or upgrade events.

### Hardware fingerprint

`hardware_fingerprint_id` is derived from trustworthy hardware anchors visible to the guest environment, such as system UUID, baseboard/system serials, Raspberry Pi/board serials, or corresponding virtual-machine identifiers.

Upgradeable capacity such as RAM size, disk capacity, GPU inventory, current CPU utilization, and installed software must not be part of the hardware fingerprint.

Raw serials and UUIDs are not emitted. SmartSentinel hashes canonicalized anchors locally and reports only the derived fingerprint plus the classes of evidence used and a confidence level.

The fingerprint is observational evidence, not cryptographic identity attestation.

Critically, SmartSentinel does **not** fabricate hardware identity from hostname or OS identity. If the guest cannot observe a trustworthy hardware anchor, discovery reports:

```text
hardware_fingerprint_id = null
hardware_identity_basis = []
hardware_identity_confidence = unknown
```

This fail-closed behavior avoids collisions such as multiple VMs sharing the hostname `Xubuntu` and avoids pretending that an OS installation identifier proves physical hardware identity.

### OS installation identity

`os_installation_id` is intentionally separate from hardware identity. Linux can use `/etc/machine-id`; Windows can use `MachineGuid`. Raw values are hashed before leaving the host.

This separation handles cases such as:

```text
same hardware + new OS installation
  hardware_fingerprint_id unchanged
  os_installation_id changed
  logical role continuity preserved

same SD card moved from Raspberry Pi A to Raspberry Pi B
  hardware_fingerprint_id changed
  os_installation_id unchanged
  logical role continuity can be preserved by a controlled rebind
```

When hardware identity is unavailable, `os_installation_id` remains useful correlation evidence but must not be promoted to a hardware fingerprint.

### Capacity profile

`capacity_profile_id` fingerprints structural resource capacity rather than instantaneous load. The current O2 profile includes logical CPU count, total RAM, total swap/pagefile capacity, and filesystem identities/total capacities.

CPU use, used/free RAM, used filesystem space, and uptime do not affect the identifier.

A RAM upgrade therefore changes `capacity_profile_id` without changing hardware identity or role binding.

### Role binding

`role_binding_id` expresses the DATUM relationship between a logical role and hardware currently realizing it.

Conceptually:

```text
hardware HW-A -- performs --> role fog-01
                     binding B1
```

SmartSentinel may derive an observation-side binding candidate only when it has a hardware fingerprint. The candidate is based on `node_id + hardware_fingerprint_id`.

If trustworthy hardware identity is unavailable, `role_binding_id` remains unresolved rather than falling back to OS identity or hostname. The dserver/resource registry can later correlate the observation and assign canonical binding history.

When hardware is replaced, a new binding is created:

```text
B1: HW-A -> fog-01
B2: HW-B -> fog-01
```

The dserver, not SmartSentinel, owns historical binding lineage. It may persist:

```text
B2.predecessor_binding_id = B1
transition = hardware_replacement
```

`predecessor_binding_id` is therefore empty in standalone discovery until the control plane correlates the report with existing history.

### SmartSentinel process: `instance_id`

`instance_id` represents one SmartSentinel process lifetime. A process restart changes this ID without implying hardware, OS, capacity, or role changes.

## Discovered resources without a DATUM role

A compute resource does not need to have a logical DATUM role before it becomes useful to the control plane.

The target model is:

```text
SmartSentinel discovery
        ↓
resource registry
        ↓
resource_id = resource:X
hardware / OS / capacity / capabilities known
role_binding = none
        ↓
planner candidate pool
        ↓
DATUM planning decision
        ↓
resource:X -- performs --> fog-02
```

This separates two questions:

```text
what compute resources exist and what can they do?

which DATUM role, if any, should each resource perform?
```

A registered but unbound resource can therefore publish capability evidence and participate in placement/deployment planning without being prematurely represented as `fog-01`, `cloud-01`, or another logical role.

A future dserver resource registry should assign a canonical `resource_id`. That identity can be retained even when hardware fingerprint evidence is unavailable or uncertain, while preserving the underlying identity evidence and confidence rather than hiding it.

This enables bootstrap states richer than a binary known/unknown node distinction:

```text
known_bound_node
  known resource with an active DATUM role binding

known_unbound_resource
  registered resource with capabilities but no role; available to the planner

unknown_resource
  resource not yet registered; discovery/registration required
```

Role assignment then becomes a planning/control-plane decision rather than an intrinsic property of the host.

## Change interpretation

```text
only instance_id changed
  SmartSentinel restart

os_installation_id changed, hardware unchanged
  OS reinstall/reimage

capacity_profile_id changed, hardware unchanged
  upgrade/downgrade or capacity reconfiguration

hardware_fingerprint_id changed for same node_id
  physical/virtual resource replacement or recreation

hardware changed but os_installation_id stayed the same
  OS/storage migrated to replacement hardware

same hardware becomes another node_id
  hardware role reassignment
```

A hardware change must not automatically authorize a takeover. If the old binding is still active, the dserver should treat a competing binding as conflict until policy/operator evidence permits a controlled rebind.

## Historical queries

Different questions require different historical axes:

```text
history(node_id = fog-01)
  complete logical history of the role across hardware generations

history(hardware_fingerprint_id = HW-B)
  observations belonging only to that concrete resource when hardware identity is known

history(role_binding_id = B2)
  observations while HW-B performed fog-01

history(capacity_profile_id = CAP-X)
  observations produced under one structural capacity profile
```

No telemetry is copied from predecessor hardware to successor hardware. Instead, the binding lineage preserves logical continuity while retaining the original context of every observation.

## AI/GNN context

AI/GNN/planner inputs should receive these identity and lineage dimensions together with telemetry. This enables contextual reasoning that is impossible from a flat node time series alone.

Example: if `fog-01` is rebound to replacement hardware three times in one month, the system can expose a repeated-hardware-replacement pattern. An AI reasoning layer can then correlate it with other evidence such as power-quality telemetry, temperature, environmental events, device error logs, lightning/weather information, or operator maintenance records.

The observation itself should remain deterministic:

```text
three hardware bindings in 30 days
```

Higher-level interpretation may propose hypotheses such as:

```text
possible unstable power supply
possible environmental damage
possible repeated hardware incompatibility
```

Those hypotheses must remain distinguishable from observed facts.

## Baseline rules

When hardware changes, host-specific performance baselines should be segmented or reset while logical workload history remains available.

When only capacity changes, capacity-sensitive baselines should be re-evaluated without declaring the hardware replaced.

When only the OS installation changes, the system should preserve hardware/capacity context but avoid assuming software/runtime state continuity.

When only the SmartSentinel instance changes, host/resource baselines normally remain valid unless independent evidence indicates another transition.

## Control-plane evolution

O2 introduces observation-side identity dimensions. dserver ingestion/materialization should later persist resource identity, role bindings, binding history, and explicit transitions such as:

```text
sentinel_instance_changed
os_installation_changed
capacity_profile_changed
hardware_replaced
role_rebound
binding_conflict
```

The dserver may additionally assign monotonic resource/node generations or binding revisions. SmartSentinel reports local evidence; it must not unilaterally declare global lineage or inherit historical authority.
