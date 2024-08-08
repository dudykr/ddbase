use std::{
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

    let link_candidates =
        list_of_crates(&args.target_dir).context("failed to get candidates for linking")?;

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
