# Stage 96 — Distributed E2E service-artifact catalog

This directory defines the governed service artifacts for the distributed IoT proof:

- desktop `local-01`: simulated LoRa device and virtual Semtech UDP packet forwarder;
- `mist-01`: ChirpStack Gateway Bridge;
- `fog-01`: Mosquitto and ChirpStack;
- `cloud-01`: Magistrala v1.0.0 LTS core and the Smart Adapter.

The catalog is declarative only. Registering these JSON documents does not clone a
repository, build an image, pull an image, render host files, contact a node, start
a container, execute WASM, or mutate the D-Graph.

## Magistrala v1.0.0 LTS composition

The cloud slice follows the official v1.0.0 Docker composition for the minimum
telemetry path:

- Atom and its PostgreSQL database;
- the v1 Bootstrap service and its PostgreSQL database;
- FluxMQ authentication plus three FluxMQ broker nodes;
- nginx as the public HTTP/HTTP-adapter ingress;
- TimescaleDB, Timescale writer, and Timescale reader.

Magistrala v1.0.0 loads Atom actions and policies from a mounted
`atom-bootstrap.yaml` at Atom startup. The separate `atom-bootstrap:v0.51.0`
one-shot job is not part of this catalog and was removed from the scenario.

The artifacts record required configuration and certificate mounts as governed
configuration contracts. Materializing those mounts and the official FluxMQ/nginx
configuration remains a later execution concern; no cloud mutation is performed
by this stage.

## End-to-end flow

```text
lora-device-simulator
  -> lora-packet-forwarder
  -> chirpstack-gateway-bridge
  -> mosquitto
  -> chirpstack
  -> sma
  -> magistrala-nginx
  -> magistrala-fluxmq-auth
  -> magistrala-timescale-writer
  -> magistrala-timescale
```

Atom remains the control-plane path for workspaces, devices, resources, credentials,
and authorization:

```text
magistrala-atom-db -> magistrala-atom -> magistrala-bootstrap
```

Cross-node links are represented by scenario relations and bindings, not by
node-local artifact dependencies. Secrets are represented only by `secret_refs`;
no credential values are stored here.

Expected Stage 96 validation:

- scenario has 4 nodes;
- scenario has 21 artifact references;
- scenario has 6 explicit relations;
- all cloud Magistrala images are pinned to `v1.0.0` or the official pinned
  infrastructure image;
- `magistrala-atom-bootstrap.json` is not registered.


## Governed configuration mounts

Container artifacts may declare `runtime.container.configuration_mounts` with relative logical
sources and absolute container targets. Sources are resolved only below the node-local
`[dispatch].configuration_root` and are mounted read-only. Absolute host paths,
path traversal, and writable configuration mounts are rejected. The reconciler never
executes a shell command to materialize these files.
