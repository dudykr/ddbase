use std::fs;

use appcore_app_spec::AppSpec;
use schemars::schema_for;

fn main() {
    let schema = schema_for!(AppSpec);
    let git_root = get_git_root();
    let json = serde_json::to_string_pretty(&schema).unwrap();

    eprintln!("Git root: {}", git_root);

    fs::write(format!("{}/schemas/appcore-app.json", git_root), json).unwrap();
}

fn get_git_root() -> String {
    let current_dir = std::env::current_dir().unwrap();
    let git_root = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(current_dir)
        .output()
        .unwrap();
    String::from_utf8(git_root.stdout)
        .unwrap()
        .trim()
        .to_string()
}
