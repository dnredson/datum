use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::process::Command;

use crate::core::ids;
use crate::core::model::{Edge, Entity, Snapshot};
use crate::core::plugin::Plugin;

#[derive(Debug, Deserialize)]
struct DockerPsRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "Names")]
    names: String,
    #[serde(rename = "State")]
    state: String,
    #[serde(rename = "Status")]
    status: String,

    #[serde(rename = "CreatedAt")]
    created_at: Option<String>,
    #[serde(rename = "Command")]
    command: Option<String>,
}

fn docker_ps_json_lines() -> Result<String> {
    let out = Command::new("docker")
        .args(["ps", "--no-trunc", "--format", "{{json .}}"])
        .output()?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("docker ps failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub struct DockerLinuxCliPlugin;

impl DockerLinuxCliPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for DockerLinuxCliPlugin {
    fn name(&self) -> &'static str {
        "docker_linux_cli"
    }

    fn apply(&self, snapshot: &mut Snapshot, _cfg: &crate::config::Config) -> Result<()> {
        if !cfg!(unix) {
            return Ok(());
        }

        let stdout = match docker_ps_json_lines() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("plugin={} status=skipped err={}", self.name(), e);
                return Ok(());
            }
        };

        let observed_at = snapshot.meta.observed_at_utc;
        let host_id = snapshot.meta.host_id.clone();

        for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let row: DockerPsRow = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("plugin={} status=warn msg=parse_line_failed err={}", self.name(), e);
                    continue;
                }
            };

            let container_eid = ids::container_id("docker", &row.id);

            let ent = Entity {
                id: container_eid.clone(),
                kind: "container".to_string(),
                attrs: json!({
                    "runtime": "docker",
                    "container_id": row.id,
                    "name": row.names,
                    "image": row.image,
                    "state": row.state,
                    "status": row.status,
                    "created_at_raw": row.created_at,
                    "command": row.command,
                    "host_id": host_id,
                }),
                observed_at_utc: observed_at,
                ttl_s: 60,
            };

            let edge = Edge {
                from: snapshot.meta.host_id.clone(),
                to: container_eid,
                rel: "runs".to_string(),
                attrs: json!({ "runtime": "docker" }),
                observed_at_utc: observed_at,
            };

            snapshot.entities.push(ent);
            snapshot.edges.push(edge);
        }

        Ok(())
    }
}
