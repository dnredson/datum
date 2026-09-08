use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;

use datum::telemetry::SmartSentinelTelemetryEnvelope;

pub struct TelemetryHttpOutput {
    client: Client,
    url: String,
    project_id: String,
}

impl TelemetryHttpOutput {
    pub fn new(url: String, project_id: String, timeout_s: u64) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_s))
            .build()
            .context("build blocking reqwest telemetry client")?;
        Ok(Self {
            client,
            url,
            project_id,
        })
    }

    pub fn emit(&self, telemetry: &SmartSentinelTelemetryEnvelope) -> Result<()> {
        let payload =
            serde_json::to_string(telemetry).context("serialize SmartSentinel telemetry")?;
        let res = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("X-Datum-Project", &self.project_id)
            .body(payload)
            .send()
            .context("send SmartSentinel telemetry to dserver")?;
        let status = res.status();
        let body = res.text().unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!(
                "telemetry HTTP emit failed: status={} body={}",
                status,
                body
            ));
        }
        Ok(())
    }
}
