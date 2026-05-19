use crate::dispatch::model::DispatchAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchActionValidation {
    Valid,
    Skipped { reason: String },
    Rejected { reason: String },
}

pub fn validate_dispatch_action(
    action: &DispatchAction,
    local_node_id: &str,
) -> DispatchActionValidation {
    let action_id = match action.action_id.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => v,
        _ => {
            return DispatchActionValidation::Rejected {
                reason: "action_id is required".to_string(),
            };
        }
    };

    let action_kind = match action.action_kind.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => v,
        _ => {
            return DispatchActionValidation::Rejected {
                reason: format!("action {action_id} is missing action_kind"),
            };
        }
    };

    if let Some(assigned_node) = action.assigned_node.as_deref() {
        if assigned_node != local_node_id {
            return DispatchActionValidation::Rejected {
                reason: format!(
                    "assigned_node mismatch: action is assigned to {assigned_node}, local node is {local_node_id}"
                ),
            };
        }
    }

    if !action_kind.starts_with("migrate_") {
        if let Some(target_node) = action.target_node.as_deref() {
            if target_node != local_node_id {
                return DispatchActionValidation::Rejected {
                    reason: format!(
                        "target_node mismatch: action targets {target_node}, local node is {local_node_id}"
                    ),
                };
            }
        }
    }

    if action_kind.starts_with("migrate_") {
        return DispatchActionValidation::Skipped {
            reason: "migration execution is not implemented yet".to_string(),
        };
    }

    if matches!(action.runtime.as_deref(), Some("wasm")) {
        return DispatchActionValidation::Skipped {
            reason: "wasm execution is not implemented yet".to_string(),
        };
    }

    match action_kind {
        "ensure_container_running" => {
            if action
                .artifact
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return DispatchActionValidation::Rejected {
                    reason: "artifact must be present for ensure_container_running".to_string(),
                };
            }
            DispatchActionValidation::Valid
        }
        _ => DispatchActionValidation::Skipped {
            reason: format!("unknown action_kind is not supported: {action_kind}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_dispatch_action, DispatchActionValidation};
    use crate::dispatch::model::DispatchAction;

    fn base_action() -> DispatchAction {
        DispatchAction {
            action_id: Some("gap:1".to_string()),
            action_kind: Some("ensure_container_running".to_string()),
            runtime: Some("container".to_string()),
            artifact: Some("chirpstack/chirpstack:latest".to_string()),
            target_node: Some("fog-01".to_string()),
            assigned_node: Some("fog-01".to_string()),
            ..DispatchAction::default()
        }
    }

    #[test]
    fn validates_local_container_action() {
        assert_eq!(
            validate_dispatch_action(&base_action(), "fog-01"),
            DispatchActionValidation::Valid
        );
    }

    #[test]
    fn rejects_action_assigned_to_other_node() {
        let mut action = base_action();
        action.assigned_node = Some("fog-02".to_string());

        assert!(matches!(
            validate_dispatch_action(&action, "fog-01"),
            DispatchActionValidation::Rejected { .. }
        ));
    }

    #[test]
    fn rejects_non_migration_action_with_other_target_node() {
        let mut action = base_action();
        action.target_node = Some("fog-02".to_string());

        assert!(matches!(
            validate_dispatch_action(&action, "fog-01"),
            DispatchActionValidation::Rejected { .. }
        ));
    }

    #[test]
    fn skips_migrate_actions() {
        let mut action = base_action();
        action.action_kind = Some("migrate_container".to_string());

        assert!(matches!(
            validate_dispatch_action(&action, "fog-01"),
            DispatchActionValidation::Skipped { .. }
        ));
    }

    #[test]
    fn skips_wasm_actions() {
        let mut action = base_action();
        action.action_kind = Some("ensure_wasm_running".to_string());
        action.runtime = Some("wasm".to_string());

        assert!(matches!(
            validate_dispatch_action(&action, "fog-01"),
            DispatchActionValidation::Skipped { .. }
        ));
    }

    #[test]
    fn skips_unknown_action_kind() {
        let mut action = base_action();
        action.action_kind = Some("reboot_everything".to_string());

        assert!(matches!(
            validate_dispatch_action(&action, "fog-01"),
            DispatchActionValidation::Skipped { .. }
        ));
    }

    #[test]
    fn rejects_missing_artifact_for_ensure_container_running() {
        let mut action = base_action();
        action.artifact = None;

        assert!(matches!(
            validate_dispatch_action(&action, "fog-01"),
            DispatchActionValidation::Rejected { .. }
        ));
    }

    #[test]
    fn rejects_missing_action_id() {
        let mut action = base_action();
        action.action_id = None;

        assert!(matches!(
            validate_dispatch_action(&action, "fog-01"),
            DispatchActionValidation::Rejected { .. }
        ));
    }

    #[test]
    fn rejects_missing_action_kind() {
        let mut action = base_action();
        action.action_kind = None;

        assert!(matches!(
            validate_dispatch_action(&action, "fog-01"),
            DispatchActionValidation::Rejected { .. }
        ));
    }
}
