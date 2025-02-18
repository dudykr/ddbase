/// Explicit extern crate to change memory allocator
extern crate swc_malloc;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use futures::future::try_join_all;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use tracing::{info, level_filters::LevelFilter};

use crate::{
    config::{find_appcore_app_configs, parse_app_config},
    provision::provision_app,
};

mod config;
mod provision;
mod vendors;

#[derive(Debug, Parser)]
struct CliArgs {
    #[clap(subcommand)]
    cmd: CliCmd,
}

#[derive(Debug, Subcommand)]
enum CliCmd {
    Provision(ProvisionArgs),
}

#[derive(Debug, Args)]
struct ProvisionArgs {
    #[clap(long)]
    only: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = CliArgs::parse();

    dotenv::dotenv().ok();

    tracing_subscriber::fmt()
        .with_ansi(true)
        .with_thread_names(false)
        .with_max_level(LevelFilter::INFO)
        .pretty()
        .init();

    let git_root_dir = get_git_root_dir().context("failed to get git root dir")?;
    info!("Git root dir: {}", git_root_dir);

    let config_files =
        find_appcore_app_configs(&git_root_dir).context("failed to find appcore app configs")?;

    for config_file in &config_files {
        info!("Found appcore app config: {}", config_file.display());
    }

    match args.cmd {
        CliCmd::Provision(args) => {
            let mut configs = config_files
                .par_iter()
                .map(|config_file| {
                    parse_app_config(config_file.clone()).with_context(|| {
                        format!(
                            "failed to parse appcore app config at `{}`",
                            config_file.display()
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            if !args.only.is_empty() {
                configs.retain(|config| args.only.contains(&config.config.name));
            }

            for config in &configs {
                info!("Parsed appcore app config: {}", config.config.name);
            }

            try_join_all(configs.into_iter().map(provision_app)).await?;
        }
    }

    Ok(())
}

fn get_git_root_dir() -> Result<String> {
    let output = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()?;

    let root_dir = String::from_utf8(output.stdout)?;
    Ok(root_dir.trim().to_string())
}
