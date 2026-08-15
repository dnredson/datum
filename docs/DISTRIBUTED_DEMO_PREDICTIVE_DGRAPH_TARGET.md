# Distributed Demo and Predictive D-Graph Target

## Demo objective

The Controlled WASM Deployment phase should culminate in a visually demonstrable distributed SmartSentinel deployment rather than only isolated runtime tests.

Target physical/logical environment:

```text
Raspberry Pi / edge-fog DATUM
Local PC / DATUM
Local or lab VM / DATUM
Optional AWS VM / DATUM, bootstrapped through Ansible
                 ^
                 |
              dserver
                 ^
                 |
          external D-Graph
```

Ansible is infrastructure bootstrap, not the SmartSentinel control plane. It may provision or prepare cloud hosts and install/start DATUM. After bootstrap, deployment intent, node assignment, artifact verification, runtime state, monitoring, and adaptation remain SmartSentinel responsibilities.

## End-to-end demonstration target

The demonstration should make the following sequence visible:

```text
external D-Graph
  -> dserver validation/materialization
  -> Deployment Manifest
  -> node-scoped assignment
  -> DATUM artifact acquisition
  -> SHA-256 verification
  -> explicit bounded Wasmtime execution
  -> runtime reports / telemetry
  -> live D-Graph state
```

A useful live scenario distributes distinct components or WASM modules across Raspberry Pi, local PC, VM, and optionally AWS, then leaves the system running in its requested desired state.

The demo should remain functional with the local three-node environment even if public cloud or Internet connectivity is unavailable during presentation.

## Live D-Graph dashboard

A dedicated UI should render the current D-Graph as an interactive operational view.

Node/resource clicks should expose, where available:

- node status and health;
- CPU and memory utilization;
- uptime;
- capabilities;
- processes and process-level CPU/memory;
- containers/services;
- assigned WASM modules;
- artifact SHA-256/integrity state;
- runtime status and last execution/report;
- assignment and deployment state.

Relationship/edge clicks should expose, where available:

- relationship type;
- protocol;
- traffic/message rate;
- latency;
- health/state;
- source and destination resources.

The UI should visually distinguish desired/known, assigned, healthy/running, degraded, and failed state. Cytoscape.js or an equivalent graph UI is a suitable presentation layer; telemetry can be streamed from dserver through SSE or WebSocket when that interface is introduced.

## Predictive / what-if D-Graph layer

A second mode should treat the graph as a predictive digital representation of the deployment.

Representative operator questions:

- What is the predicted impact on this fog if three or five additional sensors are attached?
- How does that change propagate to the mist layer?
- What is the predicted cloud impact?
- Can the cloud tier sustain 50 additional sensors spread across ten fog nodes?
- Which node becomes the first bottleneck?
- What placement or capacity change would reduce the predicted overload risk?

The what-if engine should clone the observed graph, apply hypothetical mutations, and estimate the resulting resource state without modifying the live deployment.

Example:

```text
observed graph
   -> clone
   -> +50 sensors / +10 fog attachments
   -> predictive inference
   -> predicted CPU/RAM/network/queue/latency
   -> bottleneck and SLA-risk explanation
```

## GNN role

A Graph Neural Network is a strong candidate because both topology and resource state matter and load effects propagate through adjacent layers.

Possible node features include:

- node/resource type;
- CPU utilization and capacity;
- memory utilization and capacity;
- network throughput/capacity;
- queue depth;
- request/message rate;
- sensor count;
- process/service count;
- current runtime/module assignments;
- historical utilization summaries.

Possible edge features include:

- relationship type;
- protocol;
- bandwidth;
- measured latency;
- traffic rate;
- loss/error indicators.

Potential prediction targets include:

- future or scenario-conditioned CPU utilization;
- memory utilization;
- network load;
- queueing/latency;
- overload probability;
- SLA violation probability;
- bottleneck node/resource;
- propagated impact across edge/fog/mist/cloud tiers.

The first model should be evaluated against simpler baselines such as linear/resource models, queueing approximations, or tree-based regressors. A GNN should be retained when topology-aware prediction produces measurable benefit rather than adopted only for presentation value.

## LLM role

An LLM may be coupled to the predictive layer as a natural-language interface and explanation layer, but it should not invent quantitative capacity predictions.

Preferred separation:

```text
operator question
   -> LLM parses scenario intent
   -> deterministic graph mutation specification
   -> predictive model / GNN computes estimates
   -> constraint/capacity checks
   -> LLM explains model-backed result
```

Example operator request:

```text
"What happens if I add 50 sensors across these 10 fog nodes?"
```

The LLM can translate this to a scenario, invoke the predictor, and return a grounded answer such as predicted utilization deltas, overloaded nodes, uncertainty, and suggested placement/capacity alternatives.

## Data strategy

The predictive model should be trained/calibrated from SmartSentinel telemetry and controlled load experiments. Synthetic what-if scenarios can augment data, but evaluation should include observed configurations and held-out topology/load conditions.

The dashboard should distinguish observed measurements from model predictions and should expose uncertainty/confidence when the predictive model supports it.

## Presentation mode

A high-impact UI can offer an explicit `What-if` mode:

1. select a fog/node or group of nodes;
2. add hypothetical sensors/resources with a slider or form;
3. render hypothetical graph elements as non-live/ghost nodes;
4. run predictive inference;
5. color nodes by predicted utilization/overload risk;
6. click nodes to inspect observed versus predicted CPU, memory, traffic, and latency;
7. show downstream impact through mist/cloud layers;
8. optionally ask the natural-language assistant to explain or compare scenarios.

This predictive mode must remain read-only with respect to the live deployment unless a separate explicitly governed action is later approved.
