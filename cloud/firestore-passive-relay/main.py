import json
import os
from typing import Any

from cloudevents.http import CloudEvent
import functions_framework
from google.cloud import pubsub_v1
from google.events.cloud import firestore


NOTIFICATION_SCHEMA_VERSION = "0.2.0"
DEFAULT_TOPIC = "datum-passive-observation-notifications"

_publisher = pubsub_v1.PublisherClient()


def _string_field(document: Any, name: str) -> str:
    field = document.fields.get(name)
    if field is None or not field.string_value.strip():
        raise ValueError(f"Firestore latest document missing string field '{name}'")
    return field.string_value.strip()


def _notification_from_event(
    cloud_event: CloudEvent,
    payload: firestore.DocumentEventData,
) -> dict[str, Any]:
    document = payload.value
    if not document.name:
        raise ValueError("Firestore event does not contain a current document value")

    project_id = _string_field(document, "project_id")
    node_id = _string_field(document, "node_id")
    cycle_id = _string_field(document, "cycle_id")
    observed_at_utc = _string_field(document, "observed_at_utc")

    return {
        "schema_version": NOTIFICATION_SCHEMA_VERSION,
        "provider": "google-cloud-firestore-eventarc",
        "provider_event_id": str(cloud_event["id"]),
        "provider_event_type": str(cloud_event["type"]),
        "provider_event_source": str(cloud_event["source"]),
        "firestore_document": document.name,
        "project_id": project_id,
        "node_id": node_id,
        "cycle_id": cycle_id,
        "observed_at_utc": observed_at_utc,
    }


@functions_framework.cloud_event
def relay_firestore_latest(cloud_event: CloudEvent) -> None:
    """Relay a latest SmartSentinel Firestore write to a durable Pub/Sub topic.

    Firestore/Eventarc owns event delivery into this Cloud Run function. The
    function intentionally forwards only observation identity/timing metadata;
    dserver retrieves the authoritative latest telemetry from Firestore before
    assessing desired-vs-observed state.
    """

    payload = firestore.DocumentEventData()
    payload._pb.ParseFromString(cloud_event.data)

    # A `written` Eventarc trigger also receives deletes. A deleted latest
    # document has no current value to assess, so ignore it explicitly instead
    # of poisoning the relay/retry path.
    if not payload.value.name:
        print(
            json.dumps(
                {
                    "relayed": False,
                    "ignored": "deleted_document",
                    "provider_event_id": str(cloud_event["id"]),
                    "provider_event_type": str(cloud_event["type"]),
                },
                sort_keys=True,
            )
        )
        return

    notification = _notification_from_event(cloud_event, payload)

    project_id = os.environ.get("GOOGLE_CLOUD_PROJECT") or notification["project_id"]
    topic_name = os.environ.get("DATUM_PASSIVE_NOTIFICATION_TOPIC", DEFAULT_TOPIC).strip()
    if not topic_name:
        raise ValueError("DATUM_PASSIVE_NOTIFICATION_TOPIC must not be empty")

    topic_path = _publisher.topic_path(project_id, topic_name)
    body = json.dumps(notification, separators=(",", ":"), sort_keys=True).encode("utf-8")

    future = _publisher.publish(
        topic_path,
        body,
        schema_version=NOTIFICATION_SCHEMA_VERSION,
        project_id=notification["project_id"],
        node_id=notification["node_id"],
        cycle_id=notification["cycle_id"],
        provider_event_id=notification["provider_event_id"],
    )
    message_id = future.result(timeout=30)

    print(
        json.dumps(
            {
                "relayed": True,
                "message_id": message_id,
                "topic": topic_path,
                "provider_event_id": notification["provider_event_id"],
                "project_id": notification["project_id"],
                "node_id": notification["node_id"],
                "cycle_id": notification["cycle_id"],
            },
            sort_keys=True,
        )
    )
