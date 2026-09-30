import { wallboardFixture } from "./wallboard-fixture.mjs";
export function projectFixture() {
  const global = wallboardFixture();
  return {
    generated_at_utc: global.generated_at_utc,
    project_id: "alpha",
    operational_graph: {
      availability: "available",
      value: {
        dgraph_id: "desired-graph",
        revision: 4,
        node_count: 3,
        desired_service_assignment_count: 6,
      },
    },
    canonical_application_count: { availability: "not_configured" },
    known_applications: structuredClone(global.known_applications),
    alerts: structuredClone(global.alerts),
    recent_events: structuredClone(global.recent_events),
    temporal_quality: structuredClone(global.temporal_quality),
    accepted_deployments: {
      availability: "available",
      value: {
        items: [
          {
            project_id: "alpha",
            application_id: "mqtt",
            dmap_id: "accepted-map-only",
            revision: 7,
          },
        ],
        limit: 100,
        has_more: false,
      },
    },
  };
}
// Presentation-only correlated evidence, never admitted to canonical stores.
export function sharedEvidence(project = "beta", id = "shared-event") {
  const event = structuredClone(projectFixture().recent_events.value.items[2]);
  event.project_id = project;
  event.application_id = null;
  event.subject = { kind: "dnode", node_id: "fog-01" };
  event.event_id = id;
  return event;
}
