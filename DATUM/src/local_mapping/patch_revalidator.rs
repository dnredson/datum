use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    config::{Config, DispatchContainerMapping},
    installation_tasks::model::{
        LocalMappingPatchRevalidationItem, LocalMappingPatchRevalidationReport,
        LocalMappingPatchRevalidationSummary,
    },
};

const REVIEW_PLACEHOLDER: &str = "REVIEW_AND_SET_CONTAINER_NAME";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMappingProposal {
    pub mapping_key: String,
    pub proposed_container_name: String,
    pub source_line: Option<String>,
}

pub fn extract_mapping_proposals_from_patch_file(content: &str) -> Vec<ParsedMappingProposal> {
    let mut proposals = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };

        let Some(mapping_key) = parse_quoted_string(raw_key.trim()) else {
            continue;
        };
        let Some(proposed_container_name) = parse_quoted_string(raw_value.trim()) else {
            continue;
        };

        proposals.push(ParsedMappingProposal {
            mapping_key,
            proposed_container_name,
            source_line: Some(raw_line.to_string()),
        });
    }

    proposals
}

pub fn revalidate_local_mapping_patch_file(
    config: &Config,
    config_path: &Path,
    proposal_path: &Path,
) -> Result<LocalMappingPatchRevalidationReport> {
    let content = fs::read_to_string(proposal_path).with_context(|| {
        format!(
            "read local mapping patch proposal file {}",
            proposal_path.display()
        )
    })?;

    let parsed_items = extract_mapping_proposals_from_patch_file(&content);
    let mut items = Vec::new();
    let mut summary = LocalMappingPatchRevalidationSummary::default();
    let config_index = index_container_mappings(config);

    for parsed in parsed_items {
        let normalized_key = normalize_key(&parsed.mapping_key);
        let current_container_name = config_index.get(&normalized_key).cloned();
        let proposed_container_name = if parsed.proposed_container_name.is_empty() {
            None
        } else {
            Some(parsed.proposed_container_name.clone())
        };

        let (status, reason) = classify_mapping_item(
            parsed.mapping_key.trim(),
            proposed_container_name.as_deref(),
            current_container_name.as_deref(),
        );
        increment_summary(&mut summary, &status);

        items.push(LocalMappingPatchRevalidationItem {
            mapping_key: if parsed.mapping_key.trim().is_empty() {
                None
            } else {
                Some(parsed.mapping_key)
            },
            proposed_container_name,
            current_container_name,
            status,
            reason,
            source_line: parsed.source_line,
        });
    }

    summary.total_items = items.len() as u64;
    let status = overall_status(&summary);
    let metadata = Some(json!({
        "advisory": true,
        "operator_review_required": true,
        "proposal_file_format": "toml_like_patch_proposal",
        "parsed_mapping_lines": summary.total_items,
    }));

    Ok(LocalMappingPatchRevalidationReport {
        proposal_path: proposal_path.display().to_string(),
        config_path: config_path.display().to_string(),
        status,
        checked_at_utc: Some(Utc::now().to_rfc3339()),
        items,
        summary,
        safe_to_auto_apply: false,
        side_effects_performed: false,
        applied: false,
        metadata,
    })
}

fn parse_quoted_string(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.len() < 2 || !trimmed.starts_with('"') || !trimmed.ends_with('"') {
        return None;
    }

    Some(trimmed[1..trimmed.len() - 1].to_string())
}

fn index_container_mappings(config: &Config) -> BTreeMap<String, String> {
    let mut index = BTreeMap::new();
    let dispatch = match config.dispatch.as_ref() {
        Some(dispatch) => dispatch,
        None => return index,
    };

    for mapping in &dispatch.container_mappings {
        insert_mapping_key(&mut index, mapping.module_id.as_deref(), mapping);
        insert_mapping_key(&mut index, mapping.service_name.as_deref(), mapping);
    }

    index
}

fn insert_mapping_key(
    index: &mut BTreeMap<String, String>,
    raw_key: Option<&str>,
    mapping: &DispatchContainerMapping,
) {
    let Some(raw_key) = raw_key else {
        return;
    };
    let normalized_key = normalize_key(raw_key);
    if normalized_key.is_empty() {
        return;
    }

    index
        .entry(normalized_key)
        .or_insert_with(|| mapping.container_name.clone());
}

fn classify_mapping_item(
    mapping_key: &str,
    proposed_container_name: Option<&str>,
    current_container_name: Option<&str>,
) -> (String, String) {
    if mapping_key.trim().is_empty() {
        return (
            "ignored".to_string(),
            "mapping key is empty, so the proposal line was ignored".to_string(),
        );
    }

    match (proposed_container_name, current_container_name) {
        (Some(proposed), Some(current))
            if normalize_key(proposed) == normalize_key(current) && !proposed.is_empty() =>
        {
            (
                "already_present".to_string(),
                "active config already contains the same container mapping".to_string(),
            )
        }
        (Some(proposed), Some(current)) if proposed == REVIEW_PLACEHOLDER || proposed.is_empty() => (
            "conflict".to_string(),
            format!(
                "active config already maps this key to '{}' but the proposal still uses an incomplete placeholder",
                current
            ),
        ),
        (Some(_), Some(current)) => (
            "conflict".to_string(),
            format!(
                "active config already maps this key to '{}' with a different container name",
                current
            ),
        ),
        (Some(proposed), None) if proposed == REVIEW_PLACEHOLDER || proposed.is_empty() => (
            "incomplete".to_string(),
            "operator must replace REVIEW_AND_SET_CONTAINER_NAME before any manual config update"
                .to_string(),
        ),
        (Some(_), None) => (
            "valid_new_mapping".to_string(),
            "proposal would add a new mapping if an operator later copies it manually".to_string(),
        ),
        (None, Some(current)) => (
            "warning".to_string(),
            format!(
                "proposal line did not contain a container value; active config currently maps this key to '{}'",
                current
            ),
        ),
        (None, None) => (
            "incomplete".to_string(),
            "proposal line did not contain a container value".to_string(),
        ),
    }
}

fn increment_summary(summary: &mut LocalMappingPatchRevalidationSummary, status: &str) {
    match status {
        "valid_new_mapping" => summary.valid_new_mappings += 1,
        "already_present" => summary.already_present += 1,
        "conflict" => summary.conflicts += 1,
        "incomplete" => summary.incomplete += 1,
        "ignored" => summary.ignored += 1,
        "warning" => summary.warnings += 1,
        _ => summary.warnings += 1,
    }
}

fn overall_status(summary: &LocalMappingPatchRevalidationSummary) -> String {
    if summary.total_items == 0 {
        return "empty".to_string();
    }
    if summary.conflicts > 0 {
        return "failed".to_string();
    }
    if summary.incomplete > 0 || summary.ignored > 0 || summary.warnings > 0 {
        return "warning".to_string();
    }
    "passed".to_string()
}

fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{
        extract_mapping_proposals_from_patch_file, revalidate_local_mapping_patch_file,
        ParsedMappingProposal,
    };
    use crate::{
        config::{AgentConfig, Config, DispatchConfig, DispatchContainerMapping, OutputsConfig},
        installation_tasks::model::LocalMappingPatchRevalidationReport,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn base_config() -> Config {
        Config {
            agent: AgentConfig {
                node_id: "fog-01".to_string(),
                cycle_s: 5,
                schema_version: "0.0.1".to_string(),
                hostname: None,
                stage: None,
                events: None,
            },
            outputs: OutputsConfig {
                file_json: None,
                http: None,
            },
            dispatch: Some(DispatchConfig {
                enabled: true,
                url: None,
                project_id: Some("smart-city-campinas".to_string()),
                timeout_s: None,
                execute: Some(false),
                publish_results: Some(false),
                allow_container_execution: Some(false),
                configuration_root: None,
                secret_bindings: Vec::new(),
                container_mappings: Vec::new(),
                authorization_clock_skew_seconds: None,
                minimum_execution_pinning_mode: None,
                allow_native_process_execution: None,
                native_process_root: None,
            }),
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    fn temp_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "datum-local-mapping-revalidator-{}-{}",
            label,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    fn write_temp_patch_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).expect("proposal file should be written");
        path
    }

    fn revalidate(content: &str, config: Config) -> LocalMappingPatchRevalidationReport {
        let dir = temp_test_dir("report");
        let proposal_path = write_temp_patch_file(&dir, "proposal.toml", content);
        let report =
            revalidate_local_mapping_patch_file(&config, Path::new("config.toml"), &proposal_path)
                .expect("revalidation should succeed");
        let _ = fs::remove_dir_all(dir);
        report
    }

    #[test]
    fn parser_extracts_quoted_mapping_key_value_pairs() {
        let parsed = extract_mapping_proposals_from_patch_file(
            "\"chirpstack-primary\" = \"local-chirpstack\"",
        );

        assert_eq!(
            parsed,
            vec![ParsedMappingProposal {
                mapping_key: "chirpstack-primary".to_string(),
                proposed_container_name: "local-chirpstack".to_string(),
                source_line: Some("\"chirpstack-primary\" = \"local-chirpstack\"".to_string()),
            }]
        );
    }

    #[test]
    fn parser_ignores_comments_blank_lines_and_sections() {
        let parsed = extract_mapping_proposals_from_patch_file(
            "# advisory\n\n[dispatch.container_mappings]\n\"chirpstack-primary\" = \"local-chirpstack\"\n# trailing",
        );

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].mapping_key, "chirpstack-primary");
    }

    #[test]
    fn placeholder_value_produces_incomplete_status() {
        let report = revalidate(
            "[dispatch.container_mappings]\n\"chirpstack-primary\" = \"REVIEW_AND_SET_CONTAINER_NAME\"\n",
            base_config(),
        );

        assert_eq!(report.status, "warning");
        assert_eq!(report.summary.incomplete, 1);
        assert_eq!(report.items[0].status, "incomplete");
    }

    #[test]
    fn existing_same_mapping_produces_already_present() {
        let mut config = base_config();
        config
            .dispatch
            .as_mut()
            .expect("dispatch")
            .container_mappings = vec![DispatchContainerMapping {
            module_id: Some("chirpstack-primary".to_string()),
            service_name: None,
            container_name: "local-chirpstack".to_string(),
            artifact: None,
            ports: Vec::new(),
        }];

        let report = revalidate(
            "[dispatch.container_mappings]\n\"chirpstack-primary\" = \"local-chirpstack\"\n",
            config,
        );

        assert_eq!(report.status, "passed");
        assert_eq!(report.summary.already_present, 1);
        assert_eq!(report.items[0].status, "already_present");
    }

    #[test]
    fn existing_different_mapping_produces_conflict() {
        let mut config = base_config();
        config
            .dispatch
            .as_mut()
            .expect("dispatch")
            .container_mappings = vec![DispatchContainerMapping {
            module_id: Some("chirpstack-primary".to_string()),
            service_name: None,
            container_name: "existing-container".to_string(),
            artifact: None,
            ports: Vec::new(),
        }];

        let report = revalidate(
            "[dispatch.container_mappings]\n\"chirpstack-primary\" = \"other-container\"\n",
            config,
        );

        assert_eq!(report.status, "failed");
        assert_eq!(report.summary.conflicts, 1);
        assert_eq!(report.items[0].status, "conflict");
    }

    #[test]
    fn missing_concrete_mapping_produces_valid_new_mapping() {
        let report = revalidate(
            "[dispatch.container_mappings]\n\"chirpstack-primary\" = \"new-container\"\n",
            base_config(),
        );

        assert_eq!(report.status, "passed");
        assert_eq!(report.summary.valid_new_mappings, 1);
        assert_eq!(report.items[0].status, "valid_new_mapping");
    }

    #[test]
    fn empty_proposal_content_produces_empty_report() {
        let report = revalidate("# advisory only\n# no mappings\n", base_config());

        assert_eq!(report.status, "empty");
        assert_eq!(report.summary.total_items, 0);
    }

    #[test]
    fn report_safety_flags_remain_false() {
        let report = revalidate(
            "[dispatch.container_mappings]\n\"chirpstack-primary\" = \"new-container\"\n",
            base_config(),
        );

        assert!(!report.safe_to_auto_apply);
        assert!(!report.side_effects_performed);
        assert!(!report.applied);
    }

    #[test]
    fn report_is_json_serializable() {
        let report = revalidate(
            "[dispatch.container_mappings]\n\"chirpstack-primary\" = \"new-container\"\n",
            base_config(),
        );

        let rendered =
            serde_json::to_string_pretty(&report).expect("report should serialize to JSON");

        assert!(rendered.contains("\"status\": \"passed\""));
        assert!(rendered.contains("\"safe_to_auto_apply\": false"));
        assert!(rendered.contains("\"applied\": false"));
    }

    #[test]
    fn patch_revalidator_source_guard_has_no_docker_shell_or_config_writes() {
        let source = include_str!("patch_revalidator.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("implementation section should exist");

        for forbidden in [
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "Command::new",
            "std::process::Command",
            "fs::write",
            "OpenOptions",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "patch revalidator must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
