use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
struct CliArgs {
    /// The target directory to link to the current project.
    ///
    /// If the target directory is a cargo workspace, all packages in the
    /// workspace will be linked.
    target_dir: PathBuf,
}

fn main() {
    let args = CliArgs::parse();
}
