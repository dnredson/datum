use clap::Parser;

mod core;
mod outputs;
mod config;
use std::time::Instant;
use core::cycle::run_cycle;
use outputs::file_json::FileJsonOutput;

#[derive(Parser, Debug)]
#[command(name = "DATUM")]
#[command(about = "DATUM core agent - Phase 0 (snapshot contracts + outputs)", long_about = None)]
struct Args {
    /// Path to config.toml
    #[arg(long, default_value = "config.toml")]
    config: String,

    /// Override: output JSON file path
    #[arg(long)]
    out: Option<String>,

    /// Override: cycle interval in seconds
    #[arg(long)]
    cycle_s: Option<u64>,

    /// Override: node id
    #[arg(long)]
    node_id: Option<String>,

    /// Override: hostname
    #[arg(long)]
    hostname: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cfg = config::load_config(&args.config)?;

    // Apply overrides (CLI > config)
    let node_id = args.node_id.unwrap_or(cfg.agent.node_id.clone());
    let cycle_s = args.cycle_s.unwrap_or(cfg.agent.cycle_s);
    let hostname = args.hostname.or(cfg.agent.hostname.clone());

    // Output config (file_json)
    let file_cfg = cfg.outputs.file_json.clone().unwrap_or(config::FileJsonConfig {
        enabled: true,
        path: "snapshot.json".to_string(),
    });

    let out_path = args.out.unwrap_or(file_cfg.path);

    // Só cria output se enabled=true
    let file_output = if file_cfg.enabled {
        Some(FileJsonOutput::new(out_path))
    } else {
        None
    };

    loop {
        let t0 = Instant::now();

        let snapshot_res = run_cycle(&node_id, hostname.as_deref(), &cfg.agent.schema_version, &cfg);

        match snapshot_res {
            Ok(snapshot) => {
                let mut out_status = String::new();

                if let Some(out) = &file_output {
                    match out.emit(&snapshot) {
                        Ok(_) => out_status.push_str("file_json=ok"),
                        Err(e) => out_status.push_str(&format!("file_json=err({})", e)),
                    }
                } else {
                    out_status.push_str("file_json=disabled");
                }

                let dur_ms = t0.elapsed().as_millis();
                println!(
                    "cycle={} dur_ms={} entities={} edges={} events={} out={}",
                    snapshot.meta.cycle_id,
                    dur_ms,
                    snapshot.entities.len(),
                    snapshot.edges.len(),
                    snapshot.events.len(),
                    out_status
                );
            }
            Err(e) => {
                let dur_ms = t0.elapsed().as_millis();
                println!("cycle=err dur_ms={} err={}", dur_ms, e);
            }
        }

        std::thread::sleep(std::time::Duration::from_secs(cycle_s));
    }
}