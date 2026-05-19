use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
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

fn docker_filter(cfg: &crate::config::Config) -> crate::config::DockerConfig {
    cfg.collectors
        .as_ref()
        .and_then(|c| c.docker.as_ref())
        .cloned()
        .unwrap_or_default()
}

fn normalize_name(s: &str) -> String {
    s.trim().to_lowercase()
}

fn image_base(s: &str) -> String {
    let s = normalize_name(s);
    s.split(':').next().unwrap_or(&s).to_string()
}

fn should_keep_container(
    mode: &str,
    allow_names: &std::collections::HashSet<String>,
    allow_images: &std::collections::HashSet<String>,
    row_name: &str,
    row_image: &str,
) -> bool {
    match mode {
        "all" => true,
        "allowlist" => {
            if allow_names.is_empty() && allow_images.is_empty() {
                return false;
            }
            let n = normalize_name(row_name);
            let img_full = normalize_name(row_image);
            let img_base = image_base(row_image);

            allow_names.contains(&n)
                || allow_images.contains(&img_full)
                || allow_images.contains(&img_base)
        }
        _ => true,
    }
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

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        if !cfg!(unix) {
            return Ok(());
        }

        let dcfg = docker_filter(cfg);
        if !dcfg.enabled {
            // docker pode ser pesado em ambiente compartilhado; só roda se habilitado
            return Ok(());
        }

        let names = dcfg.names.unwrap_or_default();
        let images = dcfg.images.unwrap_or_default();

        let mode = match dcfg.mode.as_deref() {
            Some(m) => m.to_string(),
            None => {
                if !names.is_empty() || !images.is_empty() {
                    "allowlist".to_string()
                } else {
                    "all".to_string()
                }
            }
        };

        let allow_names: HashSet<String> = names.into_iter().map(|s| normalize_name(&s)).collect();
        let allow_images: HashSet<String> =
            images.into_iter().map(|s| normalize_name(&s)).collect();

        let max_entries = dcfg.max_entries.unwrap_or(200);

        let stdout = match docker_ps_json_lines() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("plugin={} status=skipped err={}", self.name(), e);
                return Ok(());
            }
        };

        let observed_at = snapshot.meta.observed_at_utc;
        let host_id = snapshot.meta.host_id.clone();

        let mut emitted: usize = 0;

        for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let row: DockerPsRow = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!(
                        "plugin={} status=warn msg=parse_line_failed err={}",
                        self.name(),
                        e
                    );
                    continue;
                }
            };

            if !should_keep_container(&mode, &allow_names, &allow_images, &row.names, &row.image) {
                continue;
            }

            if emitted >= max_entries {
                break;
            }

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
            emitted += 1;
        }

        Ok(())
    }
}
