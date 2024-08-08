use std::{
    env::current_dir,
    path::{Path, PathBuf},
    ptr::metadata,
};

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Debug, Parser)]
struct CliArgs {
    /// Changes the link location to <dir>.
    ///
    /// Defaults to the current directory.
    #[clap(short = 'C', long)]
    dir: Option<PathBuf>,

    /// The target directory to link to the current project.
    ///
    /// If the target directory is a cargo workspace, all packages in the
    /// workspace will be linked.
    target_dir: PathBuf,
}

fn main() -> Result<()> {
    let args = CliArgs::parse();

    let working_dir = match args.dir {
        Some(v) => v,
        None => current_dir().context("failed to get current directory")?,
    };

    let link_candidates =
        list_of_crates(&args.target_dir).context("failed to get candidates for linking")?;

    let crate_names = add_patch_section(&working_dir, &link_candidates)
        .context("failed to add patch section to Cargo.toml")?;

    run_cargo_update(&working_dir, &crate_names)
        .context("failed to run cargo update in the working directory")?;

    Ok(())
}

fn list_of_crates(target_dir: &Path) -> Result<Vec<String>> {
    let md = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .with_context(|| format!("failed to run cargo metadata in '{}'", target_dir.display()))?;

    let ws_members = md.workspace_members;

    Ok(md
        .packages
        .into_iter()
        .filter(|p| ws_members.contains(&p.id))
        .map(|p| p.name)
        .collect())
}

fn add_patch_section(working_dir: &Path, link_candidates: &[String]) -> Result<Vec<String>> {}

fn run_cargo_update(dir: &PathBuf, crate_names: &[String]) -> Result<()> {
    let mut cmd = std::process::Command::new(cargo_bin());
    cmd.current_dir(dir);
    cmd.arg("update");
    for name in crate_names {
        cmd.arg("--package");
        cmd.arg(name);
    }

    let status = cmd.status().context("failed to run cargo update")?;

    if !status.success() {
        anyhow::bail!("cargo update failed with status: {}", status);
    }

    Ok(())
}

fn cargo_bin() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}
