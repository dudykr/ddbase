use std::path::PathBuf;

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

fn main() {
    let args = CliArgs::parse();
}
