#[path = "../storage/mod.rs"]
mod storage;

use std::{env, path::PathBuf};

use storage::control_state::{
    domain_history_count, list_current_domains, CONTROL_STATE_SCHEMA_VERSION,
};

fn main() {
    let db_path = parse_db_path().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });

    let domains = list_current_domains(&db_path).unwrap_or_else(|error| {
        eprintln!("failed to inspect control-state database: {error}");
        std::process::exit(1);
    });

    println!("control_state_db={}", db_path.display());
    println!("schema_version={CONTROL_STATE_SCHEMA_VERSION}");
    println!("current_domain_count={}", domains.len());
    for (domain, updated_at_utc) in domains {
        let history = domain_history_count(&db_path, &domain).unwrap_or(0);
        println!(
            "domain={domain} updated_at_utc={updated_at_utc} history_count={history}"
        );
    }
}

fn parse_db_path() -> Result<PathBuf, String> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--db" {
            return args
                .next()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| "--db requires a path".to_string());
        }
    }
    Err("usage: control-state-inspect --db <path>".to_string())
}
