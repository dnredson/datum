use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::time::Instant;

mod cli;
mod config;
mod core;
mod outputs;

use cli::show::show_command;
use cli::top::top_command;
use core::cycle::run_cycle;
use outputs::file_json::FileJsonOutput;

fn main() -> Result<()> {
    let raw_args: Vec<String> = std::env::args().collect();

    // Se o primeiro argumento "real" for um subcomando conhecido, usa a CLI nova.
    // Senão, mantém o comportamento antigo (run por padrão).
    if raw_args.len() >= 2 {
        match raw_args[1].as_str() {
            "top" | "show" | "once" | "run" => {
                let cli = Cli::parse();
                return dispatch_cli(cli);
            }
            _ => {}
        }
    }

    // Compatibilidade com o modo antigo:
    let args = LegacyRunArgs::parse();
    run_command(
        &args.config,
        args.out,
        args.cycle_s,
        args.node_id,
        args.hostname,
    )
}

fn dispatch_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Run(args) => run_command(
            &args.config,
            args.out,
            args.cycle_s,
            args.node_id,
            args.hostname,
        ),
        Command::Once(args) => once_command(
            &args.config,
            args.out,
            args.node_id,
            args.hostname,
        ),
        Command::Top(args) => top_command(&args.config, args.refresh_ms),
        Command::Show(args) => show_command(&args.config, args.target),
    }
}

#[derive(Parser, Debug)]
#[command(name = "datum")]
#[command(about = "DATUM core agent")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Run(RunArgs),
    Once(OnceArgs),
    Top(TopArgs),
    Show(ShowArgs),
}

#[derive(Args, Debug)]
struct RunArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    out: Option<String>,

    #[arg(long)]
    cycle_s: Option<u64>,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    hostname: Option<String>,
}

#[derive(Args, Debug)]
struct OnceArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    out: Option<String>,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    hostname: Option<String>,
}

#[derive(Args, Debug)]
struct TopArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    refresh_ms: Option<u64>,
}

#[derive(Args, Debug)]
struct ShowArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[command(subcommand)]
    target: cli::commands::ShowCommand,
}

#[derive(Parser, Debug)]
struct LegacyRunArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    out: Option<String>,

    #[arg(long)]
    cycle_s: Option<u64>,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    hostname: Option<String>,
}

fn run_command(
    config_path: &str,
    out_override: Option<String>,
    cycle_s_override: Option<u64>,
    node_id_override: Option<String>,
    hostname_override: Option<String>,
) -> Result<()> {
    let cfg = config::load_config(config_path)?;

    let node_id = node_id_override.unwrap_or(cfg.agent.node_id.clone());
    let cycle_s = cycle_s_override.unwrap_or(cfg.agent.cycle_s);
    let hostname = hostname_override.or(cfg.agent.hostname.clone());

    let file_cfg = cfg.outputs.file_json.clone().unwrap_or(config::FileJsonConfig {
        enabled: true,
        path: "snapshot.json".to_string(),
    });

    let out_path = out_override.unwrap_or(file_cfg.path);

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
                    "mode=run cycle={} dur_ms={} entities={} edges={} events={} actions={} out={}",
                    snapshot.meta.cycle_id,
                    dur_ms,
                    snapshot.entities.len(),
                    snapshot.edges.len(),
                    snapshot.events.len(),
                    snapshot.actions.len(),
                    out_status
                );
            }
            Err(e) => {
                let dur_ms = t0.elapsed().as_millis();
                println!("mode=run cycle=err dur_ms={} err={}", dur_ms, e);
            }
        }

        std::thread::sleep(std::time::Duration::from_secs(cycle_s));
    }
}

fn once_command(
    config_path: &str,
    out_override: Option<String>,
    node_id_override: Option<String>,
    hostname_override: Option<String>,
) -> Result<()> {
    let cfg = config::load_config(config_path)?;

    let node_id = node_id_override.unwrap_or(cfg.agent.node_id.clone());
    let hostname = hostname_override.or(cfg.agent.hostname.clone());

    let file_cfg = cfg.outputs.file_json.clone().unwrap_or(config::FileJsonConfig {
        enabled: true,
        path: "snapshot.json".to_string(),
    });

    let out_path = out_override.unwrap_or(file_cfg.path);

    let file_output = if file_cfg.enabled {
        Some(FileJsonOutput::new(out_path))
    } else {
        None
    };

    let t0 = Instant::now();
    let snapshot = run_cycle(&node_id, hostname.as_deref(), &cfg.agent.schema_version, &cfg)?;

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
        "mode=once cycle={} dur_ms={} entities={} edges={} events={} actions={} out={}",
        snapshot.meta.cycle_id,
        dur_ms,
        snapshot.entities.len(),
        snapshot.edges.len(),
        snapshot.events.len(),
        snapshot.actions.len(),
        out_status
    );

    Ok(())
}