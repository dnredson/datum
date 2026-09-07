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

    // opcionais (quando existirem)
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
        // Fail-safe: não quebra o ciclo; só sinaliza que não deu
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

pub struct DockerWindowsCliPlugin;

impl DockerWindowsCliPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for DockerWindowsCliPlugin {
    fn name(&self) -> &'static str {
        "docker_windows_cli"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        // só roda no Windows (plugin por OS, como você quer)
        if !cfg!(windows) {
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

        // Fail-safe: se docker não existir/daemon off → não quebra ciclo
        let stdout = match docker_ps_json_lines() {
            Ok(s) => s,
            Err(e) => {
                set_runtime_observation(snapshot, false, None, Some(e.to_string()));
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

            // IDs determinísticos
            // Nota: seu ids::container_id hoje NÃO inclui host_id; isso pode colidir em cenários multi-host.
            // Para Fase 1 (single node) é ok; depois podemos evoluir para container:<host>:docker:<id>.
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

        set_runtime_observation(snapshot, true, Some(emitted as u64), None);
        Ok(())
    }
}

fn set_runtime_observation(
    snapshot: &mut Snapshot,
    available: bool,
    container_count: Option<u64>,
    error: Option<String>,
) {
    let Some(host) = snapshot
        .entities
        .iter_mut()
        .find(|entity| entity.id == snapshot.meta.host_id)
    else {
        return;
    };
    if !host.attrs.is_object() {
        host.attrs = json!({});
    }
    let attrs = host.attrs.as_object_mut().expect("host attrs is an object");
    let system = attrs.entry("system").or_insert_with(|| json!({}));
    if !system.is_object() {
        *system = json!({});
    }
    let system = system.as_object_mut().expect("system attrs is an object");
    system.insert("container_runtime".to_string(), json!("docker"));
    system.insert("container_runtime_available".to_string(), json!(available));
    if let Some(count) = container_count {
        system.insert("container_count".to_string(), json!(count));
    } else {
        system.remove("container_count");
    }
    if let Some(error) = error {
        system.insert("container_runtime_error".to_string(), json!(error));
    } else {
        system.remove("container_runtime_error");
    }
}
