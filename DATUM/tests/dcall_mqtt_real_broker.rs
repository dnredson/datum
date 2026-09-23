//! Phase 113B3 (section 36): real-broker MQTT transport integration seam.
//! Ordinary `cargo test` runs must NEVER require Docker/Mosquitto -- every
//! test here is `#[ignore]`d and additionally env-gated on
//! `DATUM_TEST_MQTT_BROKER_URI`, so it is inert unless a real broker is
//! explicitly provided. This exercises the transport primitives
//! (`dcall::mqtt::RumqttcPubSub`) directly against a real broker -- it does
//! NOT attempt the full governed fog/cloud D-Call E2E closure (grant
//! issuance, redemption, DServer authority) against a real broker; that is
//! explicitly Phase 113B4's scope (section 36).
//!
//! Run against a real Mosquitto instance, e.g.:
//!   docker run --rm -p 1883:1883 eclipse-mosquitto
//!   DATUM_TEST_MQTT_BROKER_URI=mqtt://127.0.0.1:1883 \
//!     cargo test --locked --test dcall_mqtt_real_broker -- --ignored --test-threads=1

use datum::dcall::mqtt::{parse_broker_uri_for_delivery, MqttPubSub, MqttQos, RumqttcPubSub};

fn broker_uri_from_env() -> Option<String> {
    std::env::var("DATUM_TEST_MQTT_BROKER_URI").ok()
}

#[test]
#[ignore]
fn real_broker_subscribe_publish_receive_round_trip() {
    let Some(broker_uri) = broker_uri_from_env() else {
        eprintln!("DATUM_TEST_MQTT_BROKER_URI not set -- skipping real-broker MQTT test");
        return;
    };
    let broker = parse_broker_uri_for_delivery(&broker_uri).expect("valid mqtt:// broker uri");

    let request_topic = "datum/v1/dcall/it-project/it-app/it-placement/it-node/it-channel";
    let reply_topic = "datum/v1/dcall-reply/it-project/it-app/it-node/it-grant";

    let mut publisher =
        RumqttcPubSub::connect(&broker, "datum-dcall-pub-it-test").expect("connect publisher");
    let mut subscriber =
        RumqttcPubSub::connect(&broker, "datum-dcall-sub-it-test").expect("connect subscriber");

    subscriber
        .subscribe(request_topic)
        .expect("subscribe to request topic");
    // Give the broker a moment to register the subscription before publishing.
    std::thread::sleep(std::time::Duration::from_millis(300));

    let request_payload = b"real-broker-request-envelope";
    publisher
        .publish(request_topic, request_payload, MqttQos::AtLeastOnce, false)
        .expect("publish request, QoS1, retain=false");

    let (received_topic, received_payload) = subscriber
        .poll(std::time::Duration::from_secs(10))
        .expect("poll for request")
        .expect("a message must arrive within the timeout");
    assert_eq!(received_topic, request_topic);
    assert_eq!(received_payload, request_payload);

    // Response round trip: subscriber (destination) replies on the reply
    // topic, publisher (source) receives it.
    publisher
        .subscribe(reply_topic)
        .expect("publisher subscribes to its own reply topic");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let response_payload = b"real-broker-response-envelope";
    subscriber
        .publish(reply_topic, response_payload, MqttQos::AtLeastOnce, false)
        .expect("publish response, QoS1, retain=false");

    let (received_reply_topic, received_reply_payload) = publisher
        .poll(std::time::Duration::from_secs(10))
        .expect("poll for reply")
        .expect("a reply message must arrive within the timeout");
    assert_eq!(received_reply_topic, reply_topic);
    assert_eq!(received_reply_payload, response_payload);
}
