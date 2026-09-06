# SmartSentinel O2 checkpoint — Firestore/Eventarc passive relay

Date: 2026-09-06
Branch: `agent/o2-firestore-eventarc-passive-trigger`
Baseline main: `00f7da755bceee3e336735b6c18a8ede7e01dd51` (step 66 LIVE PASS)

## Progress status

- 59/100 — LIVE PASS.
- 60/100 — LIVE PASS.
- 61/100 — LIVE PASS.
- 62/100 — LIVE PASS.
- 63/100 — LIVE PASS.
- 64/100 — LIVE PASS.
- 65/100 — LIVE PASS.
- 66/100 — LIVE PASS.
- 67/100 — IMPLEMENTED, pending cloud/live relay validation.

## Step 67 — Firestore direct event to durable passive-notification queue

Step 67 adds the cloud-side half of the disconnected observation path.

The local lab dserver runs on a private LAN address and is intentionally not exposed to the public Internet. Eventarc therefore must not target the dserver directly.

The durable architecture is:

```text
SmartSentinel -> Firestore datum_snapshots_latest
                    |
                    v
             Eventarc direct event
                    |
                    v
       private Cloud Run function relay
                    |
                    v
       Pub/Sub datum-passive-observation-notifications
                    |
                    v
      dserver outbound subscriber (step 68)
```

Step 67 implements and validates through Pub/Sub. Step 68 will attach dserver to the queue without inbound public exposure.

## Cloud relay

`cloud/firestore-passive-relay/main.py` implements the Cloud Run function entry point `relay_firestore_latest`.

It decodes Firestore `DocumentEventData` delivered by Eventarc and extracts only:

- project id;
- node id;
- cycle id;
- observation timestamp;
- provider event id/type/source;
- Firestore document name.

The relay does not copy the full telemetry payload into Pub/Sub. Full observed state remains authoritative in Firestore; the queue carries only durable notification metadata.

Delete events are ignored because a deleted latest document has no current observation to assess.

## Notification contract

Schema version: `0.2.0`.

The Pub/Sub data body contains:

```json
{
  "schema_version": "0.2.0",
  "provider": "google-cloud-firestore-eventarc",
  "provider_event_id": "...",
  "provider_event_type": "...",
  "provider_event_source": "...",
  "firestore_document": "...",
  "project_id": "monitor-f49c8",
  "node_id": "fog-01",
  "cycle_id": "...",
  "observed_at_utc": "..."
}
```

The Pub/Sub message also repeats project/node/cycle/provider event id as attributes.

`provider_event_id` is preserved because provider/queue delivery is at-least-once and Firestore event ordering is not guaranteed. Deduplication and safe ordering belong to the dserver subscriber in step 68, not to SmartSentinel and not to the relay.

## Deployment

`cloud/firestore-passive-relay/deploy.ps1`:

- enables required APIs;
- discovers the Firestore database location;
- requires an explicit Cloud Run region when Firestore is multi-region;
- creates the relay service account if absent;
- creates the Pub/Sub topic if absent;
- grants Pub/Sub publisher, Eventarc receiver and Cloud Run invoker roles;
- deploys the private Cloud Run function;
- creates a Firestore Eventarc trigger for `datum_snapshots_latest/{document}` if absent.

The trigger uses `google.cloud.firestore.document.v1.written` and `application/protobuf`.

## Architecture boundary

The relay:

- does not read or mutate SQLite;
- does not read or mutate the Operational D-Graph;
- does not authorize host mutation;
- does not run desired-vs-observed assessment;
- does not invoke reconciliation;
- does not choose placement;
- does not invoke AI/GNN.

It only converts a Firestore observation event into a durable notification.

## Planned live proof

1. checkout the step-67 branch;
2. authenticate `gcloud` against project `monitor-f49c8`;
3. run `cloud/firestore-passive-relay/deploy.ps1`;
4. verify the private Cloud Run function and Eventarc trigger exist;
5. create a temporary pull subscription on `datum-passive-observation-notifications`;
6. write a synthetic `step67-probe` document directly to `datum_snapshots_latest` through Firestore, without calling dserver;
7. pull the Pub/Sub message;
8. verify provider is `google-cloud-firestore-eventarc` and project/node/cycle/timestamp match the direct Firestore write;
9. verify provider event id is non-empty;
10. delete the synthetic Firestore probe and temporary subscription;
11. verify the delete event is ignored by the relay.

PASS proves a Firestore observation can produce a durable passive-analysis notification while dserver is completely outside the write path and without exposing the LAN control plane to inbound cloud traffic.

## Forward path

Step 68: dserver outbound Pub/Sub subscriber, deduplication and ordering semantics feeding the existing step-66 passive trigger core.

Step 69: multi-node unified desired/observed materialization suitable for graph consumers.

Step 70: versioned GNN feature/edge contract and graph tensor/export foundation.

Step 71+: GNN inference adapter in dry-run/proposal mode before any governed action path is considered.
