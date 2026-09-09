# Mist Gateway Bridge preparation

The `chirpstack/gateway-bridge.toml` fixture is a static EU868 laboratory configuration for anonymous MQTT at fog-01 (192.168.10.103:1883). It uses the official Gateway Bridge v4 configuration format:
https://www.chirpstack.io/docs/chirpstack-gateway-bridge/configuration.html

The artifact mounts this file read-only at the standard configuration path. The old MQTT_BROKER_HOST, UDP_LISTEN_ADDR and REGION bindings were not native Gateway Bridge settings; the mounted TOML is now the source of configuration. No MQTT secret is required for this laboratory broker.

Before an approved deployment:

1. Install the container runtime and an ARM64 SmartSentinel on mist-01; use the verified ARM64 image pinned in the artifact and local mapping.
2. Provision this file as `/opt/datum/governed-config/chirpstack/gateway-bridge.toml`. The existing lifecycle verifies mounts; it does not copy configuration files to hosts.
3. Use `DATUM/config.mist.linux`, register/bind mist-01 and publish the updated artifact.
4. Preserve the current graph, add the mist service in a new revision and authorize that revision before reconcile with `--execute`. Do not use the obsolete cloud bootstrap assignment as a deploy template.
5. Verify the HTTP metrics endpoint at port 19100. This proves process responsiveness only.
6. Run a functional canary: subscribe to `eu868/gateway/<test-id>/event/stats` on fog, send a Semtech UDP PUSH_DATA stats packet with that gateway ID to mist UDP/1700, check the matching PUSH_ACK token and decode the MQTT protobuf GatewayStats to confirm the same gateway ID. A broker connection or UDP acknowledgement alone is insufficient.
7. Repeat reconcile and check convergence, snapshots and service stability.

No remote provisioning or control-plane publication is performed by these fixtures. ChirpStack's EU868 topic prefix must match this scenario when the fog stack is added. Other regions require coordinated changes.

Preparation verified on mist-01: image label version 4.1.2, ARM64, registry digest `sha256:cc820a195e739fa653c6d7fd403abe73b9b0c79b5dd4ab0f4bf9c6443470121b`. The image successfully loaded the mounted TOML using `configfile` with networking disabled. This does not prove UDP/MQTT delivery.
