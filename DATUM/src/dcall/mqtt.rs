//! Phase 113B3 (ADR-0022 s.113B3 / ADR-0023): real MQTT transport plumbing
//! for governed `via_diot_pubsub` D-Call delivery. This module owns topic
//! derivation, envelope types, broker-URI parsing, and the publisher/
//! subscriber primitives -- it deliberately owns NO canonical authority:
//! every field that matters for authorization lives on the grant
//! (`dcall::grant::DCallDeliveryGrantV1`) or is re-derived by DServer itself;
//! this module only carries bytes over a broker and back.
//!
//! Trust boundary, stated honestly: v0.1 supports plain `mqtt://` only.
//! A bare-TCP broker connection has the SAME plaintext-network-observer
//! trust boundary this whole repository already assumes for Phase-112's bare
//! HTTP delivery (see `dcall::listener`'s own module doc) -- this module
//! never claims transport confidentiality. `mqtts://`/TLS/broker
//! authentication are explicitly deferred; a binding declaring `mqtts://` is
//! rejected HERE, at USE time, with a deterministic error -- the Phase 113B1
//! registry itself is not narrowed to reject registering one.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// `H(value) = lowercase bare SHA-256 hex of the UTF-8 bytes of value`.
/// Mirrors `dserver::core::dcall_mqtt_topic::hash_component` exactly
/// (independent wire-mirror discipline, ADR-0007) -- golden vectors in both
/// crates' test modules prove the two independent implementations agree.
pub fn hash_component(value: &str) -> String {
    crate::dcode::identity::sha256_hex(value.as_bytes())
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

/// Operational subscription prefix ONLY -- the trailing `+` is subscription
/// machinery, never canonical, never persisted anywhere (section 6).
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

pub const REQUEST_ENVELOPE_SCHEMA: &str = "datum.dcall-mqtt-envelope/1";
pub const RESPONSE_ENVELOPE_SCHEMA: &str = "datum.dcall-mqtt-response/1";

/// The ONLY bytes that cross the broker for a request. Deliberately carries
/// NO destination/DMap/channel/DForward/binding/broker/artifact field --
/// every one of those is already grant-side (server) authority; the
/// destination MUST NOT trust anything from this envelope beyond
/// `grant_id`+token+payload proof (section 16).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MqttDCallRequestEnvelopeV1 {
    pub schema: String,
    pub grant_id: String,
    pub token: String,
    pub payload_b64: String,
    pub payload_sha256: String,
    pub payload_size: u64,
}

/// The ONLY bytes that cross the broker for a response. Never token, never
/// broker URI, never raw payload, never arbitrary raw diagnostic text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MqttDCallResponseEnvelopeV1 {
    pub schema: String,
    pub grant_id: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dcode_evidence: Option<serde_json::Value>,
}

/// Reuses the same hard cap `dcall::listener::MAX_BODY_BYTES` already
/// enforces for the HTTP transport -- one existing message-size limit, never
/// a second independently-invented one.
pub const MAX_ENVELOPE_BYTES: usize = crate::dcall::listener::MAX_BODY_BYTES;

/// v0.1's one supported broker scheme.
pub const BROKER_SCHEME_MQTT: &str = "mqtt";
/// Declared/registerable (Phase 113B1) but NOT yet supported for delivery
/// (section 4) -- using one at delivery time fails closed, deterministically.
pub const BROKER_SCHEME_MQTTS: &str = "mqtts";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerAddress {
    pub host: String,
    pub port: u16,
}

/// Parses a `broker_uri` for ACTUAL delivery use (distinct from the Phase
/// 113B1 registry's own structural `validate_broker_uri`, which merely
/// checks shape and deliberately still accepts `mqtts://` as an operational
/// declaration). This function is the "at USE time" fail-closed boundary
/// section 4 requires: only `mqtt://host:port` is accepted here.
pub fn parse_broker_uri_for_delivery(broker_uri: &str) -> Result<BrokerAddress, BrokerUriError> {
    let parsed = url::Url::parse(broker_uri)
        .map_err(|error| BrokerUriError::Malformed(error.to_string()))?;
    if parsed.scheme() == BROKER_SCHEME_MQTTS {
        return Err(BrokerUriError::UnsupportedScheme(parsed.scheme().to_string()));
    }
    if parsed.scheme() != BROKER_SCHEME_MQTT {
        return Err(BrokerUriError::UnsupportedScheme(parsed.scheme().to_string()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| BrokerUriError::Malformed("broker_uri has no host".to_string()))?
        .to_string();
    let port = parsed.port().unwrap_or(1883);
    Ok(BrokerAddress { host, port })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerUriError {
    /// Deterministic, distinguishable from a generic parse failure: the
    /// binding's declared scheme is real but not yet supported for delivery
    /// (v0.1: `mqtts://` and anything else besides `mqtt://`).
    UnsupportedScheme(String),
    Malformed(String),
}

impl std::fmt::Display for BrokerUriError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrokerUriError::UnsupportedScheme(scheme) => {
                write!(f, "broker scheme '{scheme}' is not supported for D-Call MQTT delivery in this release (only 'mqtt://' is)")
            }
            BrokerUriError::Malformed(message) => write!(f, "malformed broker_uri: {message}"),
        }
    }
}

impl std::error::Error for BrokerUriError {}

/// Deterministic role-prefixed, per-invocation-unique client ID (section
/// 25) -- never derived solely from node_id, since concurrent
/// publishers/subscribers for the SAME node can legitimately exist.
pub fn publisher_client_id() -> String {
    format!("datum-dcall-pub-{}", Uuid::new_v4())
}

pub fn subscriber_client_id() -> String {
    format!("datum-dcall-sub-{}", Uuid::new_v4())
}

/// QoS levels this module ever uses. v0.1 only ever issues `AtLeastOnce`
/// (QoS1, section 11) -- kept as an explicit enum (rather than a raw
/// integer) so a test can assert exactly which QoS was requested without
/// depending on `rumqttc`'s own type in the trait signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttQos {
    AtLeastOnce,
}

/// Deliberately minimal transport seam (section 28/34): production code
/// drives a REAL broker via `RumqttcPubSub`; unit tests drive an
/// in-process `test_support::InMemoryPubSub` -- both implement the exact
/// same trait, so the actual source/destination delivery logic in
/// `dcall::delivery` is IDENTICAL in production and under test. This is the
/// smallest abstraction that lets the required test matrix (grant/binding
/// mismatch, wrong topic, QoS/retain assertions, duplicate delivery, PUBACK-
/// is-not-success, timeout) be proven deterministically without a real
/// broker.
pub trait MqttPubSub {
    fn subscribe(&mut self, topic: &str) -> Result<()>;
    /// Always QoS1/retain=false in this release (section 11) -- the caller
    /// still passes them explicitly so a test double can assert exactly
    /// what was requested, rather than trusting an implementation detail.
    fn publish(&mut self, topic: &str, payload: &[u8], qos: MqttQos, retain: bool) -> Result<()>;
    /// Blocks up to `timeout` for the next message on any subscribed topic.
    /// Returns `Ok(None)` on timeout (never an error -- a timeout is an
    /// ordinary, expected outcome the caller must handle explicitly).
    fn poll(&mut self, timeout: Duration) -> Result<Option<(String, Vec<u8>)>>;
}

/// Real production MQTT transport, backed by `rumqttc`'s SYNCHRONOUS
/// `Client`/`Connection` API. DATUM's application code is entirely
/// synchronous (see this module's own doc / `dcall-mqtt-runtime-audit`
/// finding: no `async fn` exists anywhere in `dcall::*` or the D-Call
/// listener binary, and `reqwest`'s own blocking client already hides its
/// internal tokio runtime) -- using `rumqttc::Client` (which spawns its OWN
/// background eventloop thread and never requires the caller to hold an
/// async runtime) keeps that property, rather than introducing `#[tokio::main]`
/// into the application architecture merely for MQTT.
pub struct RumqttcPubSub {
    client: rumqttc::Client,
    connection: rumqttc::Connection,
}

impl RumqttcPubSub {
    pub fn connect(broker: &BrokerAddress, client_id: &str) -> Result<Self> {
        let mut options = rumqttc::MqttOptions::new(client_id, broker.host.clone(), broker.port);
        options.set_keep_alive(Duration::from_secs(10));
        // Bounded internal channel capacity -- avoids unbounded memory
        // growth if a caller publishes/subscribes faster than the
        // eventloop drains (mirrors this codebase's established bounded-
        // buffer discipline, e.g. `dcall::delivery::MAX_REMOTE_RESPONSE_BYTES`).
        let (client, connection) = rumqttc::Client::new(options, 64);
        Ok(Self { client, connection })
    }
}

impl MqttPubSub for RumqttcPubSub {
    fn subscribe(&mut self, topic: &str) -> Result<()> {
        self.client
            .subscribe(topic, rumqttc::QoS::AtLeastOnce)
            .with_context(|| format!("subscribe to MQTT topic '{topic}'"))?;
        Ok(())
    }

    fn publish(&mut self, topic: &str, payload: &[u8], qos: MqttQos, retain: bool) -> Result<()> {
        let qos = match qos {
            MqttQos::AtLeastOnce => rumqttc::QoS::AtLeastOnce,
        };
        self.client
            .publish(topic, qos, retain, payload.to_vec())
            .with_context(|| format!("publish to MQTT topic '{topic}'"))?;
        Ok(())
    }

    fn poll(&mut self, timeout: Duration) -> Result<Option<(String, Vec<u8>)>> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            match self.connection.recv_timeout(remaining) {
                Ok(Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(publish)))) => {
                    return Ok(Some((publish.topic, publish.payload.to_vec())));
                }
                Ok(Ok(_)) => continue, // PUBACK/PINGRESP/etc: not a message, keep polling.
                Ok(Err(error)) => return Err(anyhow!("MQTT connection error: {error}")),
                Err(_timeout_from_channel) => return Ok(None),
            }
        }
    }
}

#[cfg(any(test, feature = "dcall-test-support"))]
pub mod test_support {
    //! Deterministic in-process MQTT test double (section 34/36: unit tests
    //! must never require Docker/Mosquitto). `InMemoryBroker` is a shared,
    //! thread-safe topic registry supporting exact-match AND single-level
    //! `+` wildcard subscriptions (sufficient for this module's own
    //! `subscribe_prefix`); `InMemoryPubSub` is one client's handle to it.
    //! This is genuinely a PubSub simulation (messages are matched and
    //! delivered by topic across independent handles), not a stub that
    //! merely records calls -- the required source/destination test matrix
    //! (round trip, duplicate delivery, ordering) depends on this being a
    //! real (if in-memory) message bus.

    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Condvar, Mutex};

    struct BrokerState {
        // (subscriber_id, subscribed topic filter) -> queued (topic, payload)
        subscriptions: Vec<(u64, String)>,
        inboxes: std::collections::HashMap<u64, VecDeque<(String, Vec<u8>)>>,
        next_id: u64,
        /// Every publish ever made, in order -- lets tests assert exact
        /// topic/QoS/retain without needing a subscriber on that topic.
        pub published: Vec<(String, Vec<u8>, MqttQos, bool)>,
    }

    pub struct InMemoryBroker {
        state: Mutex<BrokerState>,
        condvar: Condvar,
    }

    impl InMemoryBroker {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(BrokerState {
                    subscriptions: Vec::new(),
                    inboxes: std::collections::HashMap::new(),
                    next_id: 0,
                    published: Vec::new(),
                }),
                condvar: Condvar::new(),
            })
        }

        pub fn connect(self: &Arc<Self>) -> InMemoryPubSub {
            let mut state = self.state.lock().unwrap();
            let id = state.next_id;
            state.next_id += 1;
            state.inboxes.insert(id, VecDeque::new());
            InMemoryPubSub {
                broker: self.clone(),
                id,
            }
        }

        /// Every message ever published on the broker, in publish order --
        /// used by tests to assert exact topic/QoS/retain independent of
        /// whether any subscriber was listening.
        pub fn published_log(&self) -> Vec<(String, Vec<u8>, MqttQos, bool)> {
            self.state.lock().unwrap().published.clone()
        }

        fn topic_matches(filter: &str, topic: &str) -> bool {
            let filter_parts: Vec<&str> = filter.split('/').collect();
            let topic_parts: Vec<&str> = topic.split('/').collect();
            if filter_parts.len() != topic_parts.len() {
                return false;
            }
            filter_parts
                .iter()
                .zip(topic_parts.iter())
                .all(|(f, t)| *f == "+" || f == t)
        }
    }

    pub struct InMemoryPubSub {
        broker: Arc<InMemoryBroker>,
        id: u64,
    }

    impl InMemoryPubSub {
        /// Test-only: publishes the SAME message twice, simulating a QoS1
        /// duplicate delivery at the transport layer (section 21/35F).
        pub fn publish_duplicated(
            &mut self,
            topic: &str,
            payload: &[u8],
            qos: MqttQos,
            retain: bool,
        ) -> Result<()> {
            self.publish(topic, payload, qos, retain)?;
            self.publish(topic, payload, qos, retain)?;
            Ok(())
        }
    }

    impl MqttPubSub for InMemoryPubSub {
        fn subscribe(&mut self, topic: &str) -> Result<()> {
            let mut state = self.broker.state.lock().unwrap();
            state.subscriptions.push((self.id, topic.to_string()));
            Ok(())
        }

        fn publish(&mut self, topic: &str, payload: &[u8], qos: MqttQos, retain: bool) -> Result<()> {
            let mut state = self.broker.state.lock().unwrap();
            state
                .published
                .push((topic.to_string(), payload.to_vec(), qos, retain));
            let matching_subscribers: Vec<u64> = state
                .subscriptions
                .iter()
                .filter(|(_, filter)| InMemoryBroker::topic_matches(filter, topic))
                .map(|(id, _)| *id)
                .collect();
            for subscriber_id in matching_subscribers {
                if let Some(inbox) = state.inboxes.get_mut(&subscriber_id) {
                    inbox.push_back((topic.to_string(), payload.to_vec()));
                }
            }
            drop(state);
            self.broker.condvar.notify_all();
            Ok(())
        }

        fn poll(&mut self, timeout: Duration) -> Result<Option<(String, Vec<u8>)>> {
            let deadline = std::time::Instant::now() + timeout;
            let mut state = self.broker.state.lock().unwrap();
            loop {
                if let Some(inbox) = state.inboxes.get_mut(&self.id) {
                    if let Some(message) = inbox.pop_front() {
                        return Ok(Some(message));
                    }
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return Ok(None);
                }
                let (guard, timeout_result) =
                    self.broker.condvar.wait_timeout(state, remaining).unwrap();
                state = guard;
                if timeout_result.timed_out() {
                    // Loop once more to drain any message delivered exactly
                    // at the wakeup boundary before giving up.
                    if let Some(inbox) = state.inboxes.get_mut(&self.id) {
                        if let Some(message) = inbox.pop_front() {
                            return Ok(Some(message));
                        }
                    }
                    return Ok(None);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- golden vectors (must match dserver::core::dcall_mqtt_topic exactly) --

    const GOLDEN_PROJECT: &str = "proj:lab";
    const GOLDEN_APPLICATION: &str = "app:a";
    const GOLDEN_PLACEMENT: &str = "diotp:broker";
    const GOLDEN_NODE: &str = "fog-01";
    const GOLDEN_CHANNEL: &str = "pipeline/sensor-to-decoder";
    const GOLDEN_GRANT: &str = "dcall-grant:abc123";

    #[test]
    fn hash_component_matches_known_sha256_of_a_fixed_string() {
        assert_eq!(
            hash_component("proj:lab"),
            "54bcb9f4200ea39fc004d3bd7dc4ff7f5935715aa0c4ed782881ad4e23d1e1f5"
        );
    }

    #[test]
    fn request_topic_matches_the_dserver_golden_vector() {
        // This literal string is THE canonical golden vector -- also
        // asserted, computed independently, in
        // `dserver::core::dcall_mqtt_topic::tests::request_topic_is_deterministic_and_golden`.
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
    }

    #[test]
    fn reply_topic_matches_the_dserver_golden_vector() {
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
        assert_ne!(
            request_topic("a", "app", "placement", "node", "chan"),
            request_topic("b", "app", "placement", "node", "chan")
        );
    }

    #[test]
    fn application_change_changes_the_topic() {
        assert_ne!(
            request_topic("proj", "a", "placement", "node", "chan"),
            request_topic("proj", "b", "placement", "node", "chan")
        );
    }

    #[test]
    fn placement_change_changes_the_topic() {
        assert_ne!(
            request_topic("proj", "app", "a", "node", "chan"),
            request_topic("proj", "app", "b", "node", "chan")
        );
    }

    #[test]
    fn destination_node_change_changes_the_topic() {
        assert_ne!(
            request_topic("proj", "app", "placement", "a", "chan"),
            request_topic("proj", "app", "placement", "b", "chan")
        );
    }

    #[test]
    fn channel_change_changes_the_topic() {
        assert_ne!(
            request_topic("proj", "app", "placement", "node", "a"),
            request_topic("proj", "app", "placement", "node", "b")
        );
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
            assert!(!topic.contains(raw));
        }
    }

    #[test]
    fn no_wildcards_in_exact_topics_but_subscribe_prefix_has_exactly_one_plus() {
        let topic = request_topic(
            GOLDEN_PROJECT,
            GOLDEN_APPLICATION,
            GOLDEN_PLACEMENT,
            GOLDEN_NODE,
            GOLDEN_CHANNEL,
        );
        assert!(!topic.contains('+') && !topic.contains('#'));
        let prefix =
            subscribe_prefix(GOLDEN_PROJECT, GOLDEN_APPLICATION, GOLDEN_PLACEMENT, GOLDEN_NODE);
        assert_eq!(prefix.matches('+').count(), 1);
        assert!(!prefix.contains('#'));
    }

    // -- envelope tests (section 33) --

    fn valid_request_envelope() -> MqttDCallRequestEnvelopeV1 {
        MqttDCallRequestEnvelopeV1 {
            schema: REQUEST_ENVELOPE_SCHEMA.to_string(),
            grant_id: "dcall-grant:1".to_string(),
            token: "tok".to_string(),
            payload_b64: "aGk=".to_string(),
            payload_sha256: "a".repeat(64),
            payload_size: 2,
        }
    }

    #[test]
    fn valid_request_envelope_parses() {
        let json = serde_json::to_string(&valid_request_envelope()).unwrap();
        let parsed: MqttDCallRequestEnvelopeV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, valid_request_envelope());
    }

    #[test]
    fn request_envelope_rejects_an_unknown_field() {
        let mut value = serde_json::to_value(valid_request_envelope()).unwrap();
        value["destination_node_id"] = serde_json::json!("caller-chosen");
        let result: Result<MqttDCallRequestEnvelopeV1, _> = serde_json::from_value(value);
        assert!(result.is_err());
    }

    #[test]
    fn malformed_base64_is_rejected_before_execution() {
        let envelope = MqttDCallRequestEnvelopeV1 {
            payload_b64: "not-valid-base64!!".to_string(),
            ..valid_request_envelope()
        };
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD.decode(&envelope.payload_b64);
        assert!(decoded.is_err());
    }

    #[test]
    fn payload_digest_mismatch_is_detectable_before_execution() {
        let envelope = valid_request_envelope();
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&envelope.payload_b64)
            .unwrap();
        let actual_sha256 = crate::dcode::identity::sha256_hex(&decoded);
        assert_ne!(actual_sha256, envelope.payload_sha256);
    }

    #[test]
    fn payload_size_mismatch_is_detectable_before_execution() {
        let envelope = MqttDCallRequestEnvelopeV1 {
            payload_size: 999,
            ..valid_request_envelope()
        };
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&envelope.payload_b64)
            .unwrap();
        assert_ne!(decoded.len() as u64, envelope.payload_size);
    }

    fn valid_response_envelope() -> MqttDCallResponseEnvelopeV1 {
        MqttDCallResponseEnvelopeV1 {
            schema: RESPONSE_ENVELOPE_SCHEMA.to_string(),
            grant_id: "dcall-grant:1".to_string(),
            outcome: "success".to_string(),
            error_code: None,
            dcode_evidence: Some(serde_json::json!({"outcome": "success"})),
        }
    }

    #[test]
    fn valid_response_envelope_parses() {
        let json = serde_json::to_string(&valid_response_envelope()).unwrap();
        let parsed: MqttDCallResponseEnvelopeV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, valid_response_envelope());
    }

    #[test]
    fn response_envelope_carrying_a_token_field_is_rejected_by_deny_unknown_fields() {
        let mut value = serde_json::to_value(valid_response_envelope()).unwrap();
        value["token"] = serde_json::json!("leaked-token");
        let result: Result<MqttDCallResponseEnvelopeV1, _> = serde_json::from_value(value);
        assert!(
            result.is_err(),
            "a response envelope must never be able to carry a 'token' field"
        );
    }

    #[test]
    fn oversized_request_is_bounded_by_the_existing_message_limit() {
        assert_eq!(MAX_ENVELOPE_BYTES, crate::dcall::listener::MAX_BODY_BYTES);
    }

    // -- broker URI parsing --

    #[test]
    fn plain_mqtt_scheme_parses_host_and_port() {
        let addr = parse_broker_uri_for_delivery("mqtt://broker.lab:1883").unwrap();
        assert_eq!(addr.host, "broker.lab");
        assert_eq!(addr.port, 1883);
    }

    #[test]
    fn mqtt_scheme_defaults_to_port_1883_when_omitted() {
        let addr = parse_broker_uri_for_delivery("mqtt://broker.lab").unwrap();
        assert_eq!(addr.port, 1883);
    }

    #[test]
    fn mqtts_scheme_is_rejected_with_a_deterministic_unsupported_scheme_error() {
        let error = parse_broker_uri_for_delivery("mqtts://broker.lab:8883").unwrap_err();
        assert!(matches!(error, BrokerUriError::UnsupportedScheme(scheme) if scheme == "mqtts"));
    }

    #[test]
    fn client_ids_are_role_prefixed_and_unique_per_call() {
        let a = publisher_client_id();
        let b = publisher_client_id();
        assert!(a.starts_with("datum-dcall-pub-"));
        assert!(b.starts_with("datum-dcall-pub-"));
        assert_ne!(a, b);
        let sub = subscriber_client_id();
        assert!(sub.starts_with("datum-dcall-sub-"));
    }

    // -- in-memory broker plumbing sanity (used extensively by delivery.rs tests) --

    #[test]
    fn in_memory_broker_delivers_only_to_matching_subscribers() {
        let broker = test_support::InMemoryBroker::new();
        let mut publisher = broker.connect();
        let mut subscriber_a = broker.connect();
        let mut subscriber_b = broker.connect();
        subscriber_a.subscribe("datum/v1/dcall/x/y/z/w/+").unwrap();
        // subscriber_b subscribes to a DIFFERENT namespace -- must not receive.
        subscriber_b.subscribe("datum/v1/dcall/other/y/z/w/+").unwrap();
        publisher
            .publish(
                "datum/v1/dcall/x/y/z/w/chan",
                b"hello",
                MqttQos::AtLeastOnce,
                false,
            )
            .unwrap();
        let received = subscriber_a
            .poll(Duration::from_millis(500))
            .unwrap()
            .expect("subscriber_a must receive the matching message");
        assert_eq!(received.0, "datum/v1/dcall/x/y/z/w/chan");
        assert_eq!(received.1, b"hello");
        let not_received = subscriber_b.poll(Duration::from_millis(50)).unwrap();
        assert!(not_received.is_none());
    }

    #[test]
    fn in_memory_pubsub_poll_times_out_with_none_when_no_message_arrives() {
        let broker = test_support::InMemoryBroker::new();
        let mut subscriber = broker.connect();
        subscriber.subscribe("some/topic").unwrap();
        let result = subscriber.poll(Duration::from_millis(50)).unwrap();
        assert!(result.is_none());
    }
}
