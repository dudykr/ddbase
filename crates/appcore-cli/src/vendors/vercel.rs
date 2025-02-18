use std::sync::Arc;

use anyhow::{Context, Result};
use cached::proc_macro::cached;
use serde_derive::{Deserialize, Serialize};

use crate::provision::EnvVar;

fn token() -> String {
    std::env::var("VERCEL_TOKEN").expect("VERCEL_TOKEN is not set")
}

#[derive(Debug, Deserialize)]
pub struct Project {
    pub id: String,
}

#[cached(result = true)]
pub(crate) async fn get_project(team_slug: String, project: String) -> Result<Arc<Project>> {
    let resp = reqwest::Client::new()
        .get(format!(
            "https://api.vercel.com/v9/projects/{project}?slug={team_slug}"
        ))
        .bearer_auth(token())
        .send()
        .await
        .with_context(|| {
            format!(
                "failed to get project `{}` in team `{}`",
                project, team_slug
            )
        })?;

    let body = resp.json::<Project>().await.with_context(|| {
        format!(
            "failed to parse project response for `{}` in team `{}`",
            project, team_slug
        )
    })?;

    Ok(Arc::new(body))
}

// #[derive(Debug, Deserialize)]
// struct ProjectEnvVarsResponse {
//     pub envs: Vec<ProjectEnvVar>,
// }

// #[derive(Debug, Deserialize)]
// struct ProjectEnvVar {
//     pub target: Targets,
//     pub key: String,
// }

// #[derive(Debug, Deserialize)]
// pub enum Targets {
//     Env(String),
//     Envs(Vec<String>),
// }

// pub(crate) async fn get_project_env_vars(project_id: &str) ->
// Result<Arc<Vec<ProjectEnvVar>>> {     let resp = reqwest::get(format!(
//         "https://api.vercel.com/v9/projects/{project_id}/env"
//     ))
//     .await
//     .context("failed to get project env vars")?;

//     let body = resp
//         .json::<ProjectEnvVarsResponse>()
//         .await
//         .context("failed to parse project env vars")?;

//     Ok(Arc::new(body.envs))
// }

#[derive(Debug, Serialize)]
struct SetEnvItem<'a> {
    pub key: &'a str,
    pub value: &'a str,
    pub r#type: &'a str,
    pub target: Vec<String>,
}

pub(crate) async fn set_env_vars(
    team_slug: &str,
    project_id: &str,
    env_vars: &[EnvVar],
) -> Result<()> {
    for env_var in env_vars {
        let body = SetEnvItem {
            key: &env_var.key,
            value: &env_var.value,
            r#type: if env_var.secret { "encrypted" } else { "plain" },
            target: env_var.stage.map_or(
                vec!["production".to_string(), "development".to_string()],
                |stage| vec![stage.env_name().to_string()],
            ),
        };

        let resp = reqwest::Client::new()
            .post(format!(
                "https://api.vercel.com/v10/projects/{project_id}/env?team_slug={team_slug}&upsert=true"
            ))
            .bearer_auth(token())
            .json(&body)
            .send()
            .await
            .context("failed to set project env vars")?;

        if resp.status().is_success() {
        } else {
            return Err(anyhow::anyhow!(
                "failed to set project env vars: {}",
                resp.text().await?
            ));
        }
    }

    Ok(())
}
