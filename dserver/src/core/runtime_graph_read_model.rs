//! Phase 132: admitted DServRealization evidence, NOT an execution-instance registry.
//! Exact typed subject/stream identity only; desired state never creates evidence.
use super::{
    canonical_dmonitor::{DMonitorContractRefV1, DMonitorHealthV1, DMonitorSubjectV1},
    dmonitor_state::{DMonitorObservationRecordV1, DMonitorStateV1},
    project_graph_read_model::{GraphContent, GraphNodeKind},
};
use serde::Serialize;
use std::collections::BTreeMap;

pub const MAX_RUNTIME_EVIDENCE: usize = 1000;
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuntimeEvidence {
    pub id: String,
    pub project_id: String,
    pub application_id: String,
    pub service_id: String,
    pub node_id: String,
    pub reporting_node_id: String,
    pub monitor_instance_id: String,
    pub observation_id: String,
    pub observation_digest: String,
    pub collector: String,
    pub observed_at_utc: String,
    pub received_at_utc: String,
    pub observed_health_claim: DMonitorHealthV1,
    pub dmap_ref: Option<DMonitorContractRefV1>,
    pub logical_node_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuntimeContent {
    pub completeness: &'static str,
    pub total_evidence_count: usize,
    pub included_evidence_count: usize,
    pub evidence: Vec<RuntimeEvidence>,
}
pub fn build(
    project: &str,
    state: &DMonitorStateV1,
    logical: Option<&GraphContent>,
) -> Result<RuntimeContent, String> {
    // Latest per exact canonical subject per independent monitor epoch, not stream head.
    let mut latest: BTreeMap<
        (String, String, String, String, String),
        &DMonitorObservationRecordV1,
    > = BTreeMap::new();
    for stream in state
        .streams
        .values()
        .filter(|s| s.project_scope == project)
    {
        for record in stream.observations.values() {
            if let DMonitorSubjectV1::DServRealization {
                project_id,
                application_id,
                service_id,
                node_id,
                ..
            } = &record.observation.subject
            {
                if project_id != project || record.server_project_scope != project {
                    return Err("contradictory realization scope".into());
                }
                // The canonical subject node is retained independently of transport identity.
                latest.insert(
                    (
                        application_id.clone(),
                        service_id.clone(),
                        node_id.clone(),
                        record.observation.node_id.clone(),
                        record.observation.monitor_instance_id.clone(),
                    ),
                    record,
                );
            }
        }
    }
    let total = latest.len();
    let mut groups: BTreeMap<(String, String), Vec<RuntimeEvidence>> = BTreeMap::new();
    for ((app, service, node, reporting_node, epoch), record) in latest {
        let DMonitorSubjectV1::DServRealization { dmap_ref, .. } = &record.observation.subject
        else {
            unreachable!()
        };
        let matched = logical
            .and_then(|g| {
                g.nodes.iter().find(|n| {
                    n.kind == GraphNodeKind::Dserv
                        && n.application_id == app
                        && n.canonical_id == service
                })
            })
            .map(|n| n.id.clone());
        let id = serde_json::to_string(&(
            "runtime_evidence",
            project,
            &app,
            &service,
            &node,
            &reporting_node,
            &epoch,
        ))
        .unwrap();
        groups
            .entry((app.clone(), service.clone()))
            .or_default()
            .push(RuntimeEvidence {
                id,
                project_id: project.into(),
                application_id: app,
                service_id: service,
                node_id: node,
                reporting_node_id: reporting_node,
                monitor_instance_id: epoch,
                observation_id: record.observation.observation_id.clone(),
                observation_digest: record.observation_digest.clone(),
                collector: record.observation.collector.clone(),
                observed_at_utc: record.observation.observed_at_utc.clone(),
                received_at_utc: record.received_at_utc.clone(),
                observed_health_claim: record.observation.health,
                dmap_ref: dmap_ref.clone(),
                logical_node_id: matched,
            });
    }
    let mut evidence = vec![];
    for group in groups.into_values() {
        // Service atomicity, deterministic stop at first exceeded budget.
        if evidence.len() + group.len() > MAX_RUNTIME_EVIDENCE {
            break;
        }
        evidence.extend(group);
    }
    Ok(RuntimeContent {
        completeness: if evidence.len() == total {
            "complete"
        } else {
            "partial"
        },
        total_evidence_count: total,
        included_evidence_count: evidence.len(),
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        canonical_dmonitor::{
            CanonicalDMonitorObservationV1, CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
        },
        dmonitor_state::ingest_observation,
    };
    fn observation(
        project: &str,
        app: &str,
        service: &str,
        node: &str,
        epoch: &str,
        seq: u64,
    ) -> CanonicalDMonitorObservationV1 {
        CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.into(),
            observation_id: format!("{project}-{app}-{service}-{node}-{epoch}-{seq}"),
            project_id: Some(project.into()),
            node_id: node.into(),
            monitor_instance_id: epoch.into(),
            sequence: seq,
            observed_at_utc: "2026-09-30T14:50:00Z".into(),
            cycle_id: None,
            collector: "test-only".into(),
            subject: DMonitorSubjectV1::DServRealization {
                project_id: project.into(),
                application_id: app.into(),
                service_id: service.into(),
                node_id: node.into(),
                dmap_ref: None,
            },
            health: DMonitorHealthV1::Unknown,
            facts: serde_json::json!({"container_name":"same-display-name"}),
        }
    }
    fn admit(state: &mut DMonitorStateV1, obs: CanonicalDMonitorObservationV1) {
        let project = obs.project_id.clone().unwrap();
        ingest_observation(state, &project, obs, "2026-09-30T15:00:00Z").unwrap();
    }
    #[test]
    fn latest_exact_subject_per_stream_not_stream_head_and_no_name_inference() {
        let mut state = DMonitorStateV1::empty();
        admit(&mut state, observation("p", "a", "s", "fog", "epoch", 1));
        admit(
            &mut state,
            observation("p", "a", "other", "fog", "epoch", 2),
        );
        admit(&mut state, observation("p", "a", "s", "fog", "epoch", 3));
        let result = build("p", &state, None).unwrap();
        assert_eq!(result.evidence.len(), 2);
        assert!(result
            .evidence
            .iter()
            .any(|e| e.observation_id.ends_with("-3")));
        assert!(!result
            .evidence
            .iter()
            .any(|e| e.observation_id.ends_with("-1")));
        assert!(result.evidence.iter().all(|e| e.logical_node_id.is_none()));
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("same-display-name"));
    }
    #[test]
    fn app_node_epoch_identity_multiple_realizations_and_project_isolation() {
        let mut state = DMonitorStateV1::empty();
        for (p, a, n, e) in [
            ("p", "a", "fog", "epoch"),
            ("p", "b", "fog", "epoch2"),
            ("p", "a", "fog2", "epoch3"),
            ("p", "a", "fog", "epoch4"),
            ("q", "a", "fog", "epoch"),
        ] {
            admit(&mut state, observation(p, a, "s", n, e, 1));
        }
        let result = build("p", &state, None).unwrap();
        assert_eq!(result.evidence.len(), 4);
        assert_eq!(
            result
                .evidence
                .iter()
                .map(|e| &e.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
        assert!(result.evidence.iter().all(|e| e.project_id == "p"));
        assert_eq!(result, build("p", &state, None).unwrap());
    }
    #[test]
    fn only_realization_subject_can_create_runtime_evidence() {
        let mut state = DMonitorStateV1::empty();
        let mut obs = observation("p", "a", "s", "fog", "e", 1);
        obs.subject = DMonitorSubjectV1::DNode {
            node_id: "fog".into(),
        };
        admit(&mut state, obs);
        assert!(build("p", &state, None).unwrap().evidence.is_empty());
        // Builder has no event, DMap, operational graph or temporal input.
        assert_eq!(
            build("p", &DMonitorStateV1::empty(), None)
                .unwrap()
                .completeness,
            "complete"
        );
    }
    #[test]
    fn oversized_first_service_is_bounded_empty_partial_not_available_zero() {
        let mut state = DMonitorStateV1::empty();
        for i in 0..=MAX_RUNTIME_EVIDENCE {
            admit(
                &mut state,
                observation("p", "a", "s", "fog", &format!("e{i:04}"), 1),
            );
        }
        let result = build("p", &state, None).unwrap();
        assert_eq!(result.completeness, "partial");
        assert_eq!(result.total_evidence_count, 1001);
        assert_eq!(result.included_evidence_count, 0);
    }
    #[test]
    fn whole_service_budget_stops_deterministically_before_later_small_service() {
        let mut state = DMonitorStateV1::empty();
        for i in 0..999 {
            admit(
                &mut state,
                observation("p", "a", "a-first", "fog", &format!("e{i:04}"), 1),
            );
        }
        for i in 0..2 {
            admit(
                &mut state,
                observation("p", "a", "b-exceeded", "fog", &format!("b{i}"), 1),
            );
        }
        admit(&mut state, observation("p", "a", "c-later", "fog", "c", 1));
        let result = build("p", &state, None).unwrap();
        assert_eq!(result.completeness, "partial");
        assert_eq!(result.total_evidence_count, 1002);
        assert_eq!(result.included_evidence_count, 999);
        assert!(result.evidence.iter().all(|e| e.service_id == "a-first"));
    }
    #[test]
    fn contradictory_typed_scope_is_rejected_not_relabelled() {
        let mut state = DMonitorStateV1::empty();
        admit(&mut state, observation("p", "a", "s", "fog", "e", 1));
        let record = state
            .streams
            .values_mut()
            .next()
            .unwrap()
            .observations
            .values_mut()
            .next()
            .unwrap();
        if let DMonitorSubjectV1::DServRealization { project_id, .. } =
            &mut record.observation.subject
        {
            *project_id = "q".into();
        }
        assert!(build("p", &state, None).is_err());
    }
    #[test]
    fn logical_join_is_exact_application_service_not_desired_placement() {
        use crate::core::project_graph_read_model::{GraphCoverage, GraphNode};
        let graph = GraphContent {
            coverage: GraphCoverage {
                scope: "active_accepted_applications_only",
                completeness: "complete",
                active_application_count: 1,
                included_application_count: 1,
            },
            applications: vec![],
            edges: vec![],
            nodes: vec![GraphNode {
                id: "logical-service".into(),
                kind: GraphNodeKind::Dserv,
                canonical_id: "s".into(),
                application_id: "a".into(),
                parent_id: None,
                execution_abi: None,
                cpu_cores: None,
                memory_bytes: None,
            }],
        };
        let mut state = DMonitorStateV1::empty();
        admit(&mut state, observation("p", "a", "s", "fog", "e", 1));
        admit(&mut state, observation("p", "b", "s", "fog", "other", 1));
        let result = build("p", &state, Some(&graph)).unwrap();
        assert_eq!(
            result.evidence[0].logical_node_id.as_deref(),
            Some("logical-service")
        );
        assert_eq!(result.evidence[1].logical_node_id, None);
        assert!(build("p", &DMonitorStateV1::empty(), Some(&graph))
            .unwrap()
            .evidence
            .is_empty());
    }
    #[test]
    fn runtime_source_failure_is_not_zero_and_temporal_dimensions_are_independent() {
        use crate::core::operations_read_models::{
            CommunicationFreshness, OperationsSources, SourceSection, SourceTimeQuality,
        };
        let at = "2026-09-30T15:00:00Z".parse().unwrap();
        let missing = OperationsSources::default()
            .project_runtime_graph("p", at)
            .unwrap();
        assert!(matches!(missing.runtime, SourceSection::NotConfigured));
        let mut state = DMonitorStateV1::empty();
        admit(&mut state, observation("p", "a", "s", "fog", "e", 1));
        let sources = OperationsSources::default()
            .with_dmonitor(state, Default::default(), vec![])
            .unwrap();
        let result = sources.project_runtime_graph("p", at).unwrap();
        assert!(matches!(result.logical.graph, SourceSection::NotConfigured));
        assert!(matches!(result.runtime, SourceSection::Available(_)));
        assert_eq!(
            result.temporal_quality[0].communication_freshness,
            CommunicationFreshness::Fresh
        );
        assert_eq!(
            result.temporal_quality[0].observation_freshness,
            CommunicationFreshness::Stale
        );
        assert_eq!(
            result.temporal_quality[0].source_time_quality,
            SourceTimeQuality::BehindOrDelayed
        );
    }
}
