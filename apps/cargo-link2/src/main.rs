use std::path::{Path, PathBuf};

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

    let link_candidates = list_of_crates(&args.target_dir)?;

    Ok(())
}

fn list_of_crates(target_dir: &Path) -> Result<Vec<String>> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .with_context(|| format!("failed to run cargo metadata in '{}'", target_dir.display()))?;
}
