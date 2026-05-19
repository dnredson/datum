use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;

use crate::core::model::Snapshot;

pub struct HttpJsonOutput {
    client: Client,
    url: String,
    project_id: String,
}

impl HttpJsonOutput {
    pub fn new(url: String, project_id: String, timeout_s: u64) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_s))
            .build()
            .context("build blocking reqwest client")?;

        Ok(Self {
            client,
            url,
            project_id,
        })
    }

    pub fn emit(&self, snapshot: &Snapshot) -> Result<()> {
        let payload = serde_json::json!({
            "protocol_version": "s0.v1",
            "snapshot": snapshot
        });

        let payload_str =
            serde_json::to_string(&payload).context("serialize snapshot payload to json string")?;

        let res = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("X-Datum-Project", &self.project_id)
            .body(payload_str)
            .send()
            .context("send snapshot to server")?;

        let status = res.status();
        let body = res.text().unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!(
                "http_json emit failed: status={} body={}",
                status,
                body
            ));
        }

        Ok(())
    }
}
