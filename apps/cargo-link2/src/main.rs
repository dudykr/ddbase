use std::{
    collections::HashSet,
    env::current_dir,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use cargo_metadata::{Metadata, MetadataCommand};
use clap::Parser;

cargo_subcommand_metadata::description!(
    "Link crates from a cargo workspace to the current project"
);

#[derive(Parser)]
#[command(bin_name = "cargo", version, author, disable_help_subcommand = true)]
enum Subcommand {
    /// Show the result of macro expansion.
    #[command(name = "link", version, author, disable_version_flag = true)]
    Link(Link),
}

#[derive(Debug, Parser)]
struct Link {
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
    let Subcommand::Link(args) = Subcommand::parse();

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PatchPkg {
    name: String,
    path: PathBuf,
}

fn list_of_crates(target_dir: &Path) -> Result<Vec<PatchPkg>> {
    let md = MetadataCommand::new()
        .no_deps()
        .current_dir(target_dir)
        .exec()
        .with_context(|| format!("failed to run cargo metadata in '{}'", target_dir.display()))?;

    let ws_members = md.workspace_members;

    Ok(md
        .packages
        .into_iter()
        .filter(|p| ws_members.contains(&p.id))
        .map(|p| PatchPkg {
            name: p.name,
            path: PathBuf::from(p.manifest_path)
                .parent()
                .unwrap()
                .to_path_buf(),
        })
        .collect())
}

fn add_patch_section(working_dir: &Path, link_candidates: &[PatchPkg]) -> Result<Vec<String>> {
    let md = MetadataCommand::new()
        .current_dir(working_dir)
        .exec()
        .with_context(|| {
            format!(
                "failed to run cargo metadata in '{}'",
                working_dir.display()
            )
        })?;

    let root_manifest_path = find_root_manifest_path(&md).with_context(|| {
        format!(
            "failed to find the root manifest for '{}'",
            working_dir.display()
        )
    })?;

    let toml = std::fs::read_to_string(&root_manifest_path)
        .with_context(|| format!("failed to read '{}'", root_manifest_path.display()))?;

    let mut doc = toml.parse::<toml_edit::DocumentMut>().with_context(|| {
        format!(
            "failed to parse Cargo.toml at '{}'",
            root_manifest_path.display()
        )
    })?;

    let (crates_to_link, all_deps) = find_used_crates(&md, link_candidates)
        .with_context(|| format!("failed to find used crates in '{}'", working_dir.display()))?;

    if doc.get("patch").is_none() {
        doc["patch"] = toml_edit::table();
    }

    let patch = doc["patch"].as_table_mut().unwrap();
    if patch.get("crates-io").is_none() {
        patch["crates-io"] = toml_edit::table();
    }

    let crates_io = patch["crates-io"].as_table_mut().unwrap();

    for PatchPkg { name, path } in &crates_to_link {
        let mut v = toml_edit::table();
        v["path"] = toml_edit::value(path.display().to_string());
        crates_io[&**name] = v;
    }

    std::fs::write(&root_manifest_path, doc.to_string())
        .with_context(|| format!("failed to write to '{}'", root_manifest_path.display()))?;

    Ok(all_deps)
}

fn find_root_manifest_path(md: &Metadata) -> Result<PathBuf> {
    if let Some(root) = md.root_package() {
        Ok(root.manifest_path.clone().into())
    } else {
        Ok(PathBuf::from(md.workspace_root.clone()).join("Cargo.toml"))
    }
}

/// `(direct, all)``
fn find_used_crates(
    md: &Metadata,
    link_candidates: &[PatchPkg],
) -> Result<(Vec<PatchPkg>, Vec<String>)> {
    let mut direct_deps = HashSet::new();
    let mut all_deps = HashSet::new();

    let workspace_packages = md
        .packages
        .iter()
        .filter(|p| md.workspace_members.contains(&p.id))
        .map(|p| p.name.clone())
        .collect::<HashSet<_>>();

    for pkg in &md.packages {
        for dep in &pkg.dependencies {
            if workspace_packages.contains(&pkg.name) {
                if let Some(linked) = link_candidates.iter().find(|c| c.name == dep.name) {
                    direct_deps.insert(linked.clone());
                }
            } else if link_candidates.iter().any(|c| c.name == dep.name) {
                all_deps.insert(pkg.name.clone());
            }
        }
    }

    let mut direct_deps = direct_deps.into_iter().collect::<Vec<_>>();
    direct_deps.sort();

    let mut all_deps = all_deps.into_iter().collect::<Vec<_>>();
    all_deps.sort();

    Ok((direct_deps, all_deps))
}

fn run_cargo_update(dir: &PathBuf, crates: &[String]) -> Result<()> {
    let mut cmd = std::process::Command::new(cargo_bin());
    cmd.current_dir(dir);
    cmd.arg("update");
    for pkg in crates {
        cmd.arg("--package");
        cmd.arg(pkg);
    }

    eprintln!("Running: {:?}", cmd);
    let status = cmd.status().context("failed to run cargo update")?;

    if !status.success() {
        anyhow::bail!("cargo update failed with status: {}", status);
    }

    Ok(())
}

fn cargo_bin() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}
