use std::sync::Arc;

use anyhow::{anyhow, Result};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

use crate::core::model::SnapshotEnvelope;
use crate::{config::AppConfig, core::capabilities::NodeCapabilities};

pub struct FirebaseClient {
    pub client: Client,
    pub project_id: String,
    pub base_url: String,

    pub collection_raw: String,
    pub collection_latest: String,
    pub collection_nodes: String,

    pub collection_graph_latest: String,
    pub collection_analysis_latest: String,
    pub collection_global_events_latest: String,
    pub collection_project_topology: String,
    pub collection_project_dmap: String,
    pub collection_action_plan_latest: String,
    pub collection_dispatch_latest: String,
    pub collection_node_capabilities: String,
    pub _config: Arc<AppConfig>,
}

impl FirebaseClient {
    pub async fn new(app_config: Arc<AppConfig>) -> Result<Self> {
        let project_id = app_config.firebase.project_id.clone();

        let base_url = format!(
            "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents",
            project_id
        );

        let collection_raw = app_config
            .firebase
            .collection_raw
            .clone()
            .unwrap_or_else(|| "datum_snapshots_raw".to_string());

        let collection_latest = app_config
            .firebase
            .collection_latest
            .clone()
            .unwrap_or_else(|| "datum_snapshots_latest".to_string());

        let collection_nodes = app_config
            .firebase
            .collection_nodes
            .clone()
            .unwrap_or_else(|| "datum_nodes".to_string());

        let collection_graph_latest = app_config
            .firebase
            .collection_graph_latest
            .clone()
            .unwrap_or_else(|| "datum_graph_latest".to_string());

        let collection_analysis_latest = app_config
            .firebase
            .collection_analysis_latest
            .clone()
            .unwrap_or_else(|| "datum_analysis_latest".to_string());

        let collection_global_events_latest = app_config
            .firebase
            .collection_global_events_latest
            .clone()
            .unwrap_or_else(|| "datum_global_events_latest".to_string());

        let collection_project_topology = app_config
            .firebase
            .collection_project_topology
            .clone()
            .unwrap_or_else(|| "datum_project_topology".to_string());
        let collection_project_dmap = app_config
            .firebase
            .collection_project_dmap
            .clone()
            .unwrap_or_else(|| "datum_project_dmap".to_string());
        let collection_action_plan_latest = app_config
            .firebase
            .collection_action_plan_latest
            .clone()
            .unwrap_or_else(|| "datum_action_plan_latest".to_string());
        let collection_dispatch_latest = app_config
            .firebase
            .collection_dispatch_latest
            .clone()
            .unwrap_or_else(|| "datum_dispatch_latest".to_string());
        let collection_node_capabilities = app_config
            .firebase
            .collection_node_capabilities
            .clone()
            .unwrap_or_else(|| "datum_node_capabilities".to_string());
        Ok(Self {
            client: Client::new(),
            project_id,
            base_url,
            collection_raw,
            collection_latest,
            collection_nodes,
            collection_graph_latest,
            collection_analysis_latest,
            collection_global_events_latest,
            collection_project_topology,
            collection_project_dmap,
            collection_action_plan_latest,
            collection_dispatch_latest,
            collection_node_capabilities,
            _config: app_config,
        })
    }

    fn debug_enabled(&self) -> bool {
        self._config.server.debug.unwrap_or(false)
    }

    fn debug_log(&self, msg: &str) {
        if self.debug_enabled() {
            println!("[DEBUG][firebase] {}", msg);
        }
    }

    fn doc_url(&self, collection: &str, doc_id: &str) -> String {
        format!("{}/{}/{}", self.base_url, collection, doc_id)
    }

    fn collection_url(&self, collection: &str) -> String {
        format!("{}/{}", self.base_url, collection)
    }

    fn firestore_string(value: &str) -> Value {
        json!({ "stringValue": value })
    }

    fn firestore_bool(value: bool) -> Value {
        json!({ "booleanValue": value })
    }

    fn firestore_json_string<T: serde::Serialize>(value: &T) -> Result<Value> {
        Ok(json!({
            "stringValue": serde_json::to_string(value)?
        }))
    }

    pub async fn list_latest_snapshots_by_project(&self, project_id: &str) -> Result<Vec<Value>> {
        let url = self.collection_url(&self.collection_latest);

        self.debug_log(&format!(
            "list_latest_snapshots_by_project -> project_id={}",
            project_id
        ));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!(
                "firebase list_latest_snapshots_by_project failed: {}",
                text
            ));
        }

        let raw: Value = serde_json::from_str(&text)?;
        let docs = raw
            .get("documents")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let mut filtered = Vec::new();

        for doc in docs {
            let fields = doc.get("fields").cloned().unwrap_or_default();

            let doc_project_id = fields
                .get("project_id")
                .and_then(|v| v.get("stringValue"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if doc_project_id == project_id {
                filtered.push(doc);
            }
        }

        Ok(filtered)
    }

    pub async fn list_nodes(&self, project_id: &str) -> Result<Value> {
        let url = self.collection_url(&self.collection_nodes);

        self.debug_log(&format!("list_nodes -> project_id={}", project_id));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("firebase list_nodes failed: {}", text));
        }

        let raw: Value = serde_json::from_str(&text)?;
        let docs = raw
            .get("documents")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let mut filtered = Vec::new();

        for doc in docs {
            let fields = doc.get("fields").cloned().unwrap_or_default();

            let doc_project_id = fields
                .get("project_id")
                .and_then(|v| v.get("stringValue"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if doc_project_id == project_id {
                filtered.push(doc);
            }
        }

        Ok(json!({
            "project_id": project_id,
            "count": filtered.len(),
            "documents": filtered
        }))
    }

    pub async fn get_latest_snapshot(&self, project_id: &str, node_id: &str) -> Result<Value> {
        let doc_id = format!("{}__{}", project_id, node_id);
        let url = self.doc_url(&self.collection_latest, &doc_id);

        self.debug_log(&format!(
            "get_latest_snapshot -> project_id={}, node_id={}",
            project_id, node_id
        ));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("firebase get_latest_snapshot failed: {}", text));
        }

        let raw: Value = serde_json::from_str(&text)?;
        Ok(raw)
    }
    pub async fn store_dispatch_latest(
        &self,
        project_id: &str,
        dispatch: &serde_json::Value,
    ) -> Result<()> {
        let url = self.doc_url(&self.collection_dispatch_latest, project_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "generated_at_utc": Self::firestore_string(
                    dispatch.get("generated_at_utc")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                ),
                "payload": Self::firestore_json_string(dispatch)?
            }
        });

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("store_dispatch_latest failed: {}", text));
        }

        Ok(())
    }

    pub async fn get_dispatch_latest(&self, project_id: &str) -> Result<serde_json::Value> {
        let url = self.doc_url(&self.collection_dispatch_latest, project_id);

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("get_dispatch_latest failed: {}", text));
        }

        let raw: serde_json::Value = serde_json::from_str(&text)?;
        Ok(raw)
    }
    pub async fn store_project_dmap(
        &self,
        project_id: &str,
        dmap: &serde_json::Value,
    ) -> Result<()> {
        let url = self.doc_url(&self.collection_project_dmap, project_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "payload": Self::firestore_json_string(dmap)?
            }
        });

        self.debug_log(&format!("store_project_dmap -> project_id={}", project_id));
        self.debug_log(&format!("URL: {}", url));
        self.debug_log(&format!("BODY: {}", body));

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("store_project_dmap failed: {}", text));
        }

        Ok(())
    }

    pub async fn get_project_dmap(&self, project_id: &str) -> Result<Option<serde_json::Value>> {
        let url = self.doc_url(&self.collection_project_dmap, project_id);

        self.debug_log(&format!("get_project_dmap -> project_id={}", project_id));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.get(&url).send().await?;
        let status = res.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            self.debug_log("RESPONSE STATUS: 404 Not Found");
            return Ok(None);
        }

        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("get_project_dmap failed: {}", text));
        }

        let raw: serde_json::Value = serde_json::from_str(&text)?;

        let payload = raw
            .get("fields")
            .and_then(|f| f.get("payload"))
            .and_then(|p| p.get("stringValue"))
            .and_then(|s| s.as_str());

        if let Some(p) = payload {
            let parsed = serde_json::from_str::<serde_json::Value>(p)?;
            return Ok(Some(parsed));
        }

        Ok(None)
    }
    pub async fn store_snapshot_raw(
        &self,
        project_id: &str,
        envelope: &SnapshotEnvelope,
    ) -> Result<()> {
        let node_id = &envelope.snapshot.meta.node_id;
        let cycle_id = &envelope.snapshot.meta.cycle_id;
        let observed_at_utc = &envelope.snapshot.meta.observed_at_utc;
        let schema_version = &envelope.snapshot.meta.schema_version;
        let protocol_version = &envelope.protocol_version;

        let doc_id = format!("{}__{}_{}", project_id, node_id, cycle_id);
        let url = self.doc_url(&self.collection_raw, &doc_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "node_id": Self::firestore_string(node_id),
                "cycle_id": Self::firestore_string(cycle_id),
                "observed_at_utc": Self::firestore_string(observed_at_utc),
                "schema_version": Self::firestore_string(schema_version),
                "protocol_version": Self::firestore_string(protocol_version),
                "payload": Self::firestore_json_string(envelope)?
            }
        });

        self.debug_log(&format!(
            "store_snapshot_raw -> project_id={}, node_id={}, cycle_id={}",
            project_id, node_id, cycle_id
        ));
        self.debug_log(&format!("URL: {}", url));
        self.debug_log(&format!("BODY: {}", body));

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("firebase store_snapshot_raw failed: {}", text));
        }

        Ok(())
    }

    pub async fn store_latest_snapshot(
        &self,
        project_id: &str,
        envelope: &SnapshotEnvelope,
    ) -> Result<()> {
        let node_id = &envelope.snapshot.meta.node_id;
        let cycle_id = &envelope.snapshot.meta.cycle_id;
        let observed_at_utc = &envelope.snapshot.meta.observed_at_utc;
        let schema_version = &envelope.snapshot.meta.schema_version;
        let protocol_version = &envelope.protocol_version;

        let doc_id = format!("{}__{}", project_id, node_id);
        let url = self.doc_url(&self.collection_latest, &doc_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "node_id": Self::firestore_string(node_id),
                "cycle_id": Self::firestore_string(cycle_id),
                "observed_at_utc": Self::firestore_string(observed_at_utc),
                "schema_version": Self::firestore_string(schema_version),
                "protocol_version": Self::firestore_string(protocol_version),
                "payload": Self::firestore_json_string(envelope)?
            }
        });

        self.debug_log(&format!(
            "store_latest_snapshot -> project_id={}, node_id={}, cycle_id={}",
            project_id, node_id, cycle_id
        ));
        self.debug_log(&format!("URL: {}", url));
        self.debug_log(&format!("BODY: {}", body));

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("firebase store_latest_snapshot failed: {}", text));
        }

        Ok(())
    }

    pub async fn upsert_node(&self, project_id: &str, envelope: &SnapshotEnvelope) -> Result<()> {
        let node_id = &envelope.snapshot.meta.node_id;
        let cycle_id = &envelope.snapshot.meta.cycle_id;
        let observed_at_utc = &envelope.snapshot.meta.observed_at_utc;
        let schema_version = &envelope.snapshot.meta.schema_version;
        let protocol_version = &envelope.protocol_version;

        let doc_id = format!("{}__{}", project_id, node_id);
        let url = self.doc_url(&self.collection_nodes, &doc_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "node_id": Self::firestore_string(node_id),
                "last_cycle_id": Self::firestore_string(cycle_id),
                "last_seen_utc": Self::firestore_string(observed_at_utc),
                "schema_version": Self::firestore_string(schema_version),
                "protocol_version": Self::firestore_string(protocol_version),
                "status": Self::firestore_string("online"),
                "active": Self::firestore_bool(true)
            }
        });

        self.debug_log(&format!(
            "upsert_node -> project_id={}, node_id={}, cycle_id={}",
            project_id, node_id, cycle_id
        ));
        self.debug_log(&format!("URL: {}", url));
        self.debug_log(&format!("BODY: {}", body));

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("firebase upsert_node failed: {}", text));
        }

        Ok(())
    }

    pub async fn store_graph_latest(&self, project_id: &str, graph: &Value) -> Result<()> {
        let url = self.doc_url(&self.collection_graph_latest, project_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "generated_at_utc": Self::firestore_string(
                    graph.get("generated_at_utc")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                ),
                "payload": Self::firestore_json_string(graph)?
            }
        });

        self.debug_log(&format!("store_graph_latest -> project_id={}", project_id));
        self.debug_log(&format!("URL: {}", url));
        self.debug_log(&format!("BODY: {}", body));

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("firebase store_graph_latest failed: {}", text));
        }

        Ok(())
    }

    pub async fn get_graph_latest(&self, project_id: &str) -> Result<Value> {
        let url = self.doc_url(&self.collection_graph_latest, project_id);

        self.debug_log(&format!("get_graph_latest -> project_id={}", project_id));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("firebase get_graph_latest failed: {}", text));
        }

        let raw: Value = serde_json::from_str(&text)?;
        Ok(raw)
    }

    pub async fn store_analysis_latest(&self, project_id: &str, analysis: &Value) -> Result<()> {
        let url = self.doc_url(&self.collection_analysis_latest, project_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "generated_at_utc": Self::firestore_string(
                    analysis.get("generated_at_utc")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                ),
                "payload": Self::firestore_json_string(analysis)?
            }
        });

        self.debug_log(&format!(
            "store_analysis_latest -> project_id={}",
            project_id
        ));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("firebase store_analysis_latest failed: {}", text));
        }

        Ok(())
    }

    pub async fn get_analysis_latest(&self, project_id: &str) -> Result<Value> {
        let url = self.doc_url(&self.collection_analysis_latest, project_id);

        self.debug_log(&format!("get_analysis_latest -> project_id={}", project_id));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("firebase get_analysis_latest failed: {}", text));
        }

        let raw: Value = serde_json::from_str(&text)?;
        Ok(raw)
    }

    pub async fn store_global_events_latest(&self, project_id: &str, events: &Value) -> Result<()> {
        let url = self.doc_url(&self.collection_global_events_latest, project_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "generated_at_utc": Self::firestore_string(
                    events.get("generated_at_utc")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                ),
                "payload": Self::firestore_json_string(events)?
            }
        });

        self.debug_log(&format!(
            "store_global_events_latest -> project_id={}",
            project_id
        ));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!(
                "firebase store_global_events_latest failed: {}",
                text
            ));
        }

        Ok(())
    }

    pub async fn get_global_events_latest(&self, project_id: &str) -> Result<Value> {
        let url = self.doc_url(&self.collection_global_events_latest, project_id);

        self.debug_log(&format!(
            "get_global_events_latest -> project_id={}",
            project_id
        ));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!(
                "firebase get_global_events_latest failed: {}",
                text
            ));
        }

        let raw: Value = serde_json::from_str(&text)?;
        Ok(raw)
    }

    pub async fn store_project_topology(&self, project_id: &str, topology: &Value) -> Result<()> {
        let url = self.doc_url(&self.collection_project_topology, project_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "payload": Self::firestore_json_string(topology)?
            }
        });

        self.debug_log(&format!(
            "store_project_topology -> project_id={}",
            project_id
        ));
        self.debug_log(&format!("URL: {}", url));
        self.debug_log(&format!("BODY: {}", body));

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("store_project_topology failed: {}", text));
        }

        Ok(())
    }

    pub async fn get_project_topology(&self, project_id: &str) -> Result<Option<Value>> {
        let url = self.doc_url(&self.collection_project_topology, project_id);

        self.debug_log(&format!(
            "get_project_topology -> project_id={}",
            project_id
        ));
        self.debug_log(&format!("URL: {}", url));

        let res = self.client.get(&url).send().await?;
        let status = res.status();

        if status == StatusCode::NOT_FOUND {
            self.debug_log("RESPONSE STATUS: 404 Not Found");
            return Ok(None);
        }

        let text = res.text().await.unwrap_or_default();

        self.debug_log(&format!("RESPONSE STATUS: {}", status));
        self.debug_log(&format!("RESPONSE BODY: {}", text));

        if !status.is_success() {
            return Err(anyhow!("get_project_topology failed: {}", text));
        }

        let raw: Value = serde_json::from_str(&text)?;

        let payload = raw
            .get("fields")
            .and_then(|f| f.get("payload"))
            .and_then(|p| p.get("stringValue"))
            .and_then(|s| s.as_str());

        if let Some(p) = payload {
            let parsed = serde_json::from_str::<Value>(p)?;
            return Ok(Some(parsed));
        }

        Ok(None)
    }
    pub async fn store_action_plan_latest(
        &self,
        project_id: &str,
        plan: &serde_json::Value,
    ) -> Result<()> {
        let url = self.doc_url(&self.collection_action_plan_latest, project_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "generated_at_utc": Self::firestore_string(
                    plan.get("generated_at_utc")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                ),
                "payload": Self::firestore_json_string(plan)?
            }
        });

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("store_action_plan_latest failed: {}", text));
        }

        Ok(())
    }

    pub async fn get_action_plan_latest(&self, project_id: &str) -> Result<serde_json::Value> {
        let url = self.doc_url(&self.collection_action_plan_latest, project_id);

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("get_action_plan_latest failed: {}", text));
        }

        let raw: serde_json::Value = serde_json::from_str(&text)?;
        Ok(raw)
    }

    pub async fn put_node_capabilities(
        &self,
        project_id: &str,
        node_id: &str,
        payload: &NodeCapabilities,
    ) -> Result<()> {
        let doc_id = format!("project:{}:node:{}", project_id, node_id);
        let url = self.doc_url(&self.collection_node_capabilities, &doc_id);

        let body = json!({
            "fields": {
                "project_id": Self::firestore_string(project_id),
                "node_id": Self::firestore_string(node_id),
                "updated_at_utc": Self::firestore_string(&payload.updated_at_utc),
                "payload": Self::firestore_json_string(payload)?
            }
        });

        let res = self.client.patch(&url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("put_node_capabilities failed: {}", text));
        }

        Ok(())
    }

    pub async fn get_node_capabilities(
        &self,
        project_id: &str,
        node_id: &str,
    ) -> Result<Option<NodeCapabilities>> {
        let doc_id = format!("project:{}:node:{}", project_id, node_id);
        let url = self.doc_url(&self.collection_node_capabilities, &doc_id);

        let res = self.client.get(&url).send().await?;
        let status = res.status();

        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("get_node_capabilities failed: {}", text));
        }

        let raw: Value = serde_json::from_str(&text)?;
        let payload = raw
            .get("fields")
            .and_then(|f| f.get("payload"))
            .and_then(|p| p.get("stringValue"))
            .and_then(|s| s.as_str())
            .ok_or_else(|| anyhow!("get_node_capabilities payload missing"))?;

        Ok(Some(serde_json::from_str(payload)?))
    }

    pub async fn list_node_capabilities(&self, project_id: &str) -> Result<Vec<NodeCapabilities>> {
        let url = self.collection_url(&self.collection_node_capabilities);

        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("list_node_capabilities failed: {}", text));
        }

        let raw: Value = serde_json::from_str(&text)?;
        let docs = raw
            .get("documents")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let mut capabilities = Vec::new();

        for doc in docs {
            let fields = doc.get("fields").cloned().unwrap_or_default();
            let doc_project_id = fields
                .get("project_id")
                .and_then(|v| v.get("stringValue"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if doc_project_id != project_id {
                continue;
            }

            if let Some(payload) = fields
                .get("payload")
                .and_then(|p| p.get("stringValue"))
                .and_then(|s| s.as_str())
            {
                capabilities.push(serde_json::from_str(payload)?);
            }
        }

        Ok(capabilities)
    }
}
