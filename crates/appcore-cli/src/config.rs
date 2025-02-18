use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use appcore_app_spec::AppSpec;

#[derive(Debug, Clone)]
pub struct AppConfigFile {
    pub path: Arc<PathBuf>,
    pub config: Arc<AppSpec>,
}

pub fn find_appcore_app_configs(git_root_dir: &str) -> Result<Vec<Arc<PathBuf>>> {
    let walker = ignore::WalkBuilder::new(git_root_dir)
        .standard_filters(true)
        .build_parallel();

    let appcore_app_configs = Arc::new(Mutex::new(Vec::new()));

    {
        let appcore_app_configs = appcore_app_configs.clone();
        walker.run(move || {
            let appcore_app_configs = appcore_app_configs.clone();
            Box::new(move |entry| {
                if let Ok(entry) = entry {
                    if entry.path().is_file() && entry.path().ends_with("appcore.yml") {
                        appcore_app_configs
                            .lock()
                            .unwrap()
                            .push(Arc::new(entry.path().to_path_buf()));
                    }
                }

                ignore::WalkState::Continue
            })
        });
    }

    let mut appcore_app_configs = Arc::try_unwrap(appcore_app_configs)
        .unwrap()
        .into_inner()
        .unwrap();

    appcore_app_configs.sort();

    Ok(appcore_app_configs)
}

pub fn parse_app_config(config_file: Arc<PathBuf>) -> Result<AppConfigFile> {
    let content = std::fs::read_to_string(&**config_file).context("failed to read config file")?;
    let config =
        serde_yaml::from_str::<AppSpec>(&content).context("failed to parse config file")?;
    Ok(AppConfigFile {
        path: config_file.clone(),
        config: Arc::new(config),
    })
}
