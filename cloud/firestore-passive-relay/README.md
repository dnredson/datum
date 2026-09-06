# Firestore passive observation relay

This Cloud Run function is the cloud-side producer for SmartSentinel passive-analysis notifications.

## Purpose

The dserver lab instance is intentionally not exposed to the public Internet. Firestore/Eventarc therefore must not attempt to call the LAN dserver directly.

The durable notification path is:

```text
SmartSentinel -> Firestore datum_snapshots_latest
                    |
                    v
             Eventarc direct event
                    |
                    v
       Cloud Run function relay_firestore_latest
                    |
                    v
       Pub/Sub datum-passive-observation-notifications
                    |
                    v
      dserver outbound subscriber (step 68)
                    |
                    v
      existing passive trigger/assessment core
```

The relay publishes only observation identity/timing metadata. The full telemetry payload remains in Firestore and is fetched by dserver when it processes the notification.

## Event source

The Eventarc trigger watches:

- provider: Cloud Firestore;
- database: `(default)`;
- event type: `google.cloud.firestore.document.v1.written`;
- document path pattern: `datum_snapshots_latest/{document}`;
- content type: `application/protobuf`.

Deletes are ignored because there is no current latest observation to assess.

## Pub/Sub notification contract

Schema `0.2.0`:

```json
{
  "schema_version": "0.2.0",
  "provider": "google-cloud-firestore-eventarc",
  "provider_event_id": "...",
  "provider_event_type": "google.cloud.firestore.document.v1.updated",
  "provider_event_source": "...",
  "firestore_document": "projects/.../documents/datum_snapshots_latest/monitor-f49c8__fog-01",
  "project_id": "monitor-f49c8",
  "node_id": "fog-01",
  "cycle_id": "...",
  "observed_at_utc": "..."
}
```

`provider_event_id` is carried forward because Eventarc/Pub/Sub delivery is at-least-once and Firestore event ordering is not guaranteed. Step 68 will use it together with project/node/cycle metadata for deduplication and safe ordering semantics.

## Deploy

From the repository root in PowerShell:

```powershell
.\cloud\firestore-passive-relay\deploy.ps1 -ProjectId monitor-f49c8
```

If the Firestore database uses a multi-region location, supply a Cloud Run region explicitly:

```powershell
.\cloud\firestore-passive-relay\deploy.ps1 `
  -ProjectId monitor-f49c8 `
  -RunRegion us-central1
```

The script enables the required APIs, creates the Pub/Sub topic and service account if needed, deploys the private Cloud Run function, and creates the Firestore Eventarc trigger if absent.

## Security boundary

- The Cloud Run function is not public.
- Eventarc invokes it using the configured service account.
- The runtime service account has Pub/Sub publisher capability.
- The relay cannot mutate the Operational D-Graph, authorize host mutation, reconcile workloads, or choose placement.
- DATUM remains conceptual; the relay is infrastructure supporting SmartSentinel/dserver observation delivery.
