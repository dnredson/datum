use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ShowCommand {
    Snapshot,
    Events {
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    Graph,
}