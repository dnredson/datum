# DATUM artifacts and laboratory bundles

`catalog/` is the canonical versioned service-artifact catalog (moved from
`dserver/fixtures/e2e/service-artifacts/`). `config/` contains the files mounted
by the prepared fog and mist artifacts. `prepare.py` creates the node-specific
files and secrets; the governed reconciler remains responsible for execution.

## What is ready, and what has been measured

| Slice | Contents | Validation status |
| --- | --- | --- |
| Mist | Gateway Bridge, EU868 TOML, ARM64 pinned image, local policy | Prior laboratory deployment converged; Semtech stats reached fog MQTT. Evidence in `evidence/`. |
| Fog core | Mosquitto, PostgreSQL 14, Redis 7, ChirpStack 4, volumes, configuration, commands, secret references | Bundle and schema tests; new deployment and functional health checks pending. |
| Optional REST API | Declarative artifact with native CLI arguments | Not included in the prepared fog core profile; deployment pending. |
| Cloud, SMA, simulated devices | Existing declarative artifacts preserved | Full reproducible configurations and end-to-end deployment remain future slices. |

The mist test proves gateway statistics transport, not device activation,
LoRaWAN application uplink decoding, SMA transformation or cloud persistence.
Images for the fog core still use upstream tags; record and pin the tested
digests in both catalog and local policy before calling a release reproducible.
The catalog is version controlled, but it is not yet a fully validated E2E release.

## Example: prepare fog on Linux

Prerequisites: Python 3.9+ for preparation, Docker, and a SmartSentinel binary
built from this change (container command propagation is required). Tests use
Python 3.11+ for `tomllib`. Run from a checkout of this repository:

```sh
python3 artifacts/prepare.py --node fog-01 --destination "$HOME/datum-bundles/fog-01"
export DATUM_RECONCILER=/absolute/path/to/smartsentinel-operational-reconcile
export DSERVER_URL=http://192.168.10.101:8080
export PROJECT_ID=monitor-f49c8
sh "$HOME/datum-bundles/fog-01/run-reconcile.sh" --out /tmp/fog-proposal.json
```

For mist, substitute `mist-01`. The preparation command does not publish artifacts,
authorize a D-Graph, pull images or start containers. The example reconcile has
no `--execute`; it can still publish observations to DServer. DServer and the
agent must both run code that carries `runtime.container.command` into the
reconciliation context.

The bundle contains `catalog/`, `config/`, `agent.toml`, `manifest.json`,
`node.env` and `run-reconcile.sh`. It uses an absolute configuration root.
Keep the destination outside Git. The fog password and API signing secret are
generated once; rerunning preserves them and refuses to overwrite changed
configuration. On Linux the secret file is mode 0600. Windows users must apply
appropriate ACLs if preparing secrets there; prefer preparing on the target VM.
Back up the node secret file with the database: generating a new password does
not change an existing PostgreSQL volume's password.

## Governed execution round

1. Review existing containers, ports, database volumes and the current D-Graph.
   Preserve all other assignments; do not replace the graph with `scenario.json`.
2. Provision the reviewed node bundle and publish its artifacts to DServer with
   the project header. Add the desired services in a new graph revision and
   authorize that exact revision. Local container mappings are also required.
3. Deploy in the order in `manifest.json`: Mosquitto, PostgreSQL, Redis,
   ChirpStack. Wait for readiness between steps; a dependency being running is
   not proof it is ready. Review a proposal before adding `--execute` to the runner.
4. Verify the following on fog, then reconcile again and require convergence
   without container replacement or host mutation:

```sh
docker exec datum-chirpstack-postgres pg_isready -U chirpstack -d chirpstack
docker exec datum-chirpstack-postgres psql -U chirpstack -d chirpstack -Atc "SELECT extname FROM pg_extension WHERE extname='pg_trgm'"
docker exec datum-chirpstack-redis redis-cli ping
curl --fail http://127.0.0.1:19101/health
```

Expected: PostgreSQL accepts connections and returns `pg_trgm`, Redis returns
`PONG`, and ChirpStack responds successfully. PostgreSQL init scripts run only
on an empty data volume. Preserve existing volumes; handle upgrades separately.
PostgreSQL and Redis have no published host ports and no synthetic health
probe: the manual checks above are required for this round.

Mosquitto now mounts an explicit anonymous-listener configuration. Publishing
this changed artifact and executing reconciliation may replace the running
broker, interrupting MQTT clients. This change has not been deployed by the
bundle preparation. The example is an isolated local laboratory using EU868
and fog MQTT at `192.168.10.103:1883`; change both fog and mist configuration
together for another topology or region.

## Validation

```sh
python3 artifacts/test_prepare.py
cargo test --locked --manifest-path DATUM/Cargo.toml --test smartsentinel_mist_gateway_contract --test smartsentinel_fog_bundle_contract --test smartsentinel_o2_service_artifact --test smartsentinel_o2_service_deployment_reconciliation
cargo test --locked --manifest-path dserver/Cargo.toml --bin DServer
# Network canary: intentionally publishes synthetic gateway statistics.
python3 artifacts/mist_udp_mqtt_test.py
```

The canary writes to `outputs/` under the current working directory. Run it
outside the checkout. Container `command` is an argument array appended after
the image; it does not run a host shell or replace the image entrypoint. Empty
or omitted arrays retain the image default. Nonempty arrays participate in
drift detection. An artifact omitting this field cannot enforce restoration of
the image default after a previous command override.

Configuration references:
[ChirpStack configuration](https://www.chirpstack.io/docs/chirpstack/configuration.html),
[official Docker composition](https://github.com/chirpstack/chirpstack-docker),
[Gateway Bridge configuration](https://www.chirpstack.io/docs/chirpstack-gateway-bridge/configuration.html).
