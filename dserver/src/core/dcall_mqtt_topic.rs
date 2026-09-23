//! Phase 113B3 (ADR-0023): deterministic operational MQTT topic mapping for
//! governed `via_diot_pubsub` D-Call delivery. Canonical DMap/2 `channel` is
//! NEVER used raw as a globally-scoped MQTT topic -- every identifier
//! segment is hashed independently so the physical topic string carries no
//! raw canonical identifier and so project/application/placement/
//! destination-node/channel each independently scope the namespace. This
//! mapping is NEVER stored in DMap/2, DForward, or the DIoT runtime binding
//! -- it is recomputed on demand, identically, wherever it is needed
//! (DServer for redemption-time verification, DATUM independently for
//! publish/subscribe). Mirrored independently in `DATUM/src/dcall/mqtt.rs`
//! per this project's established no-shared-crate wire-mirror discipline
//! (ADR-0007) -- golden vectors in both crates prove the two independent
//! implementations agree.

use crate::core::sha256::sha256_hex;

/// `H(value) = lowercase bare SHA-256 hex of the UTF-8 bytes of value`.
pub fn hash_component(value: &str) -> String {
    sha256_hex(value.as_bytes())
}

/// `datum/v1/dcall/H(project_id)/H(application_id)/H(diot_placement_id)/H(destination_node_id)/H(channel)`.
pub fn request_topic(
    project_id: &str,
    application_id: &str,
    diot_placement_id: &str,
    destination_node_id: &str,
    channel: &str,
) -> String {
    format!(
        "datum/v1/dcall/{}/{}/{}/{}/{}",
        hash_component(project_id),
        hash_component(application_id),
        hash_component(diot_placement_id),
        hash_component(destination_node_id),
        hash_component(channel),
    )
}

/// `datum/v1/dcall-reply/H(project_id)/H(application_id)/H(source_node_id)/H(grant_id)`.
pub fn reply_topic(
    project_id: &str,
    application_id: &str,
    source_node_id: &str,
    grant_id: &str,
) -> String {
    format!(
        "datum/v1/dcall-reply/{}/{}/{}/{}",
        hash_component(project_id),
        hash_component(application_id),
        hash_component(source_node_id),
        hash_component(grant_id),
    )
}

/// Operational subscription prefix a destination listener actually
/// subscribes to: `datum/v1/dcall/H(project)/H(application)/H(placement)/H(local_node)/+`.
/// The trailing `+` is subscription machinery only -- never canonical, never
/// persisted anywhere. Because `channel` is hashed into exactly one final
/// path segment, `+` (not `#`) is sufficient and correct.
pub fn subscribe_prefix(
    project_id: &str,
    application_id: &str,
    diot_placement_id: &str,
    local_node_id: &str,
) -> String {
    format!(
        "datum/v1/dcall/{}/{}/{}/{}/+",
        hash_component(project_id),
        hash_component(application_id),
        hash_component(diot_placement_id),
        hash_component(local_node_id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed golden vector -- ALSO asserted byte-for-byte identical in
    /// `DATUM/src/dcall/mqtt.rs`'s own test module. If either
    /// implementation drifts, one of the two golden-vector tests fails.
    const GOLDEN_PROJECT: &str = "proj:lab";
    const GOLDEN_APPLICATION: &str = "app:a";
    const GOLDEN_PLACEMENT: &str = "diotp:broker";
    const GOLDEN_NODE: &str = "fog-01";
    const GOLDEN_CHANNEL: &str = "pipeline/sensor-to-decoder";
    const GOLDEN_GRANT: &str = "dcall-grant:abc123";

    #[test]
    fn hash_component_matches_known_sha256_of_a_fixed_string() {
        // sha256("proj:lab") precomputed independently (Python hashlib).
        assert_eq!(
            hash_component("proj:lab"),
            "54bcb9f4200ea39fc004d3bd7dc4ff7f5935715aa0c4ed782881ad4e23d1e1f5"
        );
        assert_eq!(hash_component("proj:lab").len(), 64);
        assert!(hash_component("proj:lab")
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn request_topic_is_deterministic_and_golden() {
        let topic = request_topic(
            GOLDEN_PROJECT,
            GOLDEN_APPLICATION,
            GOLDEN_PLACEMENT,
            GOLDEN_NODE,
            GOLDEN_CHANNEL,
        );
        let expected = format!(
            "datum/v1/dcall/{}/{}/{}/{}/{}",
            hash_component(GOLDEN_PROJECT),
            hash_component(GOLDEN_APPLICATION),
            hash_component(GOLDEN_PLACEMENT),
            hash_component(GOLDEN_NODE),
            hash_component(GOLDEN_CHANNEL),
        );
        assert_eq!(topic, expected);
        // Recomputing twice must be byte-identical (determinism).
        assert_eq!(
            topic,
            request_topic(
                GOLDEN_PROJECT,
                GOLDEN_APPLICATION,
                GOLDEN_PLACEMENT,
                GOLDEN_NODE,
                GOLDEN_CHANNEL,
            )
        );
    }

    #[test]
    fn reply_topic_is_deterministic_and_golden() {
        let topic = reply_topic(GOLDEN_PROJECT, GOLDEN_APPLICATION, GOLDEN_NODE, GOLDEN_GRANT);
        let expected = format!(
            "datum/v1/dcall-reply/{}/{}/{}/{}",
            hash_component(GOLDEN_PROJECT),
            hash_component(GOLDEN_APPLICATION),
            hash_component(GOLDEN_NODE),
            hash_component(GOLDEN_GRANT),
        );
        assert_eq!(topic, expected);
    }

    #[test]
    fn project_change_changes_the_topic() {
        let a = request_topic("proj:a", "app", "placement", "node", "channel");
        let b = request_topic("proj:b", "app", "placement", "node", "channel");
        assert_ne!(a, b);
    }

    #[test]
    fn application_change_changes_the_topic() {
        let a = request_topic("proj", "app:a", "placement", "node", "channel");
        let b = request_topic("proj", "app:b", "placement", "node", "channel");
        assert_ne!(a, b);
    }

    #[test]
    fn placement_change_changes_the_topic() {
        let a = request_topic("proj", "app", "placement:a", "node", "channel");
        let b = request_topic("proj", "app", "placement:b", "node", "channel");
        assert_ne!(a, b);
    }

    #[test]
    fn destination_node_change_changes_the_topic() {
        let a = request_topic("proj", "app", "placement", "node:a", "channel");
        let b = request_topic("proj", "app", "placement", "node:b", "channel");
        assert_ne!(a, b);
    }

    #[test]
    fn channel_change_changes_the_topic() {
        let a = request_topic("proj", "app", "placement", "node", "channel:a");
        let b = request_topic("proj", "app", "placement", "node", "channel:b");
        assert_ne!(a, b);
    }

    #[test]
    fn no_raw_canonical_identifier_appears_in_the_topic() {
        let topic = request_topic(
            GOLDEN_PROJECT,
            GOLDEN_APPLICATION,
            GOLDEN_PLACEMENT,
            GOLDEN_NODE,
            GOLDEN_CHANNEL,
        );
        for raw in [
            GOLDEN_PROJECT,
            GOLDEN_APPLICATION,
            GOLDEN_PLACEMENT,
            GOLDEN_NODE,
            GOLDEN_CHANNEL,
        ] {
            assert!(
                !topic.contains(raw),
                "topic '{topic}' must not contain raw identifier '{raw}'"
            );
        }
        let reply = reply_topic(GOLDEN_PROJECT, GOLDEN_APPLICATION, GOLDEN_NODE, GOLDEN_GRANT);
        for raw in [GOLDEN_PROJECT, GOLDEN_APPLICATION, GOLDEN_NODE, GOLDEN_GRANT] {
            assert!(
                !reply.contains(raw),
                "reply topic '{reply}' must not contain raw identifier '{raw}'"
            );
        }
    }

    #[test]
    fn no_wildcard_characters_appear_in_exact_request_or_reply_topics() {
        let topic = request_topic(
            GOLDEN_PROJECT,
            GOLDEN_APPLICATION,
            GOLDEN_PLACEMENT,
            GOLDEN_NODE,
            GOLDEN_CHANNEL,
        );
        assert!(!topic.contains('+') && !topic.contains('#'));
        let reply = reply_topic(GOLDEN_PROJECT, GOLDEN_APPLICATION, GOLDEN_NODE, GOLDEN_GRANT);
        assert!(!reply.contains('+') && !reply.contains('#'));
    }

    #[test]
    fn subscribe_prefix_ends_in_a_single_plus_wildcard_only() {
        let prefix = subscribe_prefix(GOLDEN_PROJECT, GOLDEN_APPLICATION, GOLDEN_PLACEMENT, GOLDEN_NODE);
        assert!(prefix.ends_with("/+"));
        assert!(!prefix.contains('#'));
        assert_eq!(prefix.matches('+').count(), 1);
    }
}
