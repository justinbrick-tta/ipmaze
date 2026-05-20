use clap::{Parser, Subcommand};
use ipmaze_controller::{generate_crd_yaml, run_controller, ControllerConfig};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "ipmaze-controller")]
#[command(about = "Base implementation for the ipmaze CIDRPolicy controller")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    GenerateCrd,
    Run {
        #[arg(long, default_value_t = 300)]
        requeue_seconds: u64,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::GenerateCrd => {
            print!("{}", generate_crd_yaml()?);
        }
        Command::Run { requeue_seconds } => {
            run_controller(ControllerConfig {
                requeue_after: Duration::from_secs(requeue_seconds),
            })
            .await?;
        }
    }

    Ok(())
}
