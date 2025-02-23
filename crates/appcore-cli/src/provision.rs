use anyhow::{Context, Result};
use appcore_app_spec::{
    AppAuthConfig, AppDetails, AppSecretsConfig, AppSpec, DatabaseConfig, RedisConfig,
};
use futures::future::try_join_all;
use rand::{distr::Alphanumeric, Rng};
use tokio::try_join;
use tracing::info;

use crate::{
    config::AppConfigFile,
    vendors::{coolify, logto, vercel},
};

#[derive(Debug, Clone, Default)]
pub struct ProvisionOutput {
    pub env_vars: Vec<EnvVar>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    Development,
    Production,
}

impl Stage {
    fn all() -> impl Iterator<Item = Self> {
        [Self::Development, Self::Production].into_iter()
    }

    pub fn env_name(&self) -> &str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EnvVar {
    pub key: String,
    pub value: String,
    pub secret: bool,

    /// If true, the environment variable will not be updated if it already
    /// exists.
    ///
    /// TODO: Use it
    #[allow(unused)]
    pub no_update: bool,

    /// If [None], the environment variable will be provisioned for all stages.
    pub stage: Option<Stage>,
}

impl ProvisionOutput {
    pub fn merge(&mut self, other: ProvisionOutput) {
        self.env_vars.extend(other.env_vars);
    }
}

#[tracing::instrument(name = "provision_app", skip_all, fields(app_name = file.config.name))]
pub async fn provision_app(file: AppConfigFile) -> Result<()> {
    info!("Provisioning app");

    let outputs = try_join!(
        provision_app_auth(&file.config),
        provision_app_db(&file.config),
        provision_app_redis(&file.config),
        configure_app_details(&file.config),
    )
    .with_context(|| format!("failed to provision app `{}`", file.config.name))?;

    let mut output = ProvisionOutput::default();
    output.merge(outputs.0);
    output.merge(outputs.1);
    output.merge(outputs.2);
    output.merge(outputs.3);

    info!("Provisioned app `{}`", file.config.name);

    for env_var in &output.env_vars {
        if env_var.secret {
            info!("{}: <redacted>", env_var.key);
        } else {
            info!("{}: {}", env_var.key, env_var.value);
        }
    }

    match &file.config.app {
        AppDetails::NextJsApp(_app) => {
            for env_var in &mut output.env_vars {
                if !env_var.secret {
                    env_var.key = format!("NEXT_PUBLIC_{}", env_var.key);
                }
            }
        }
        AppDetails::NodeJsApiServer(_app) => {}
    }

    set_env_vars(&file, &output)
        .await
        .context("failed to set env vars")?;

    Ok(())
}

async fn configure_app_details(config: &AppSpec) -> Result<ProvisionOutput> {
    let mut output = ProvisionOutput::default();

    match &config.app {
        AppDetails::NextJsApp(_app) => {
            output.env_vars.push(EnvVar {
                key: "APP_URL".to_string(),
                value: format!("http://localhost:{}", config.dev.port),
                secret: false,
                no_update: false,
                stage: Some(Stage::Development),
            });
            output.env_vars.push(EnvVar {
                key: "APP_URL".to_string(),
                value: format!("https://{}", config.domain),
                secret: false,
                no_update: false,
                stage: Some(Stage::Production),
            });
        }
        AppDetails::NodeJsApiServer(_app) => {}
    }

    Ok(output)
}

async fn provision_app_auth(config: &AppSpec) -> Result<ProvisionOutput> {
    let mut output = ProvisionOutput::default();

    match &config.auth {
        Some(AppAuthConfig::Logto(auth_config)) => {
            let logto_config = logto::get_logto_management_api_config().await?;

            let app = logto::create_or_get_logto_application(
                logto_config.clone(),
                &auth_config.app_name,
                &config.domain,
                config.dev.port,
            )
            .await?;

            output.env_vars.push(EnvVar {
                key: "LOGTO_ENDPOINT".to_string(),
                value: logto_config.endpoint.clone(),
                secret: false,
                no_update: false,
                stage: None,
            });

            output.env_vars.push(EnvVar {
                key: "LOGTO_APP_ID".to_string(),
                value: app.id,
                secret: false,
                no_update: false,
                stage: None,
            });

            output.env_vars.push(EnvVar {
                key: "LOGTO_APP_SECRET".to_string(),
                value: app.secret,
                secret: true,
                no_update: false,
                stage: None,
            });

            for stage in Stage::all() {
                output.env_vars.push(EnvVar {
                    key: "LOGTO_COOKIE_SECRET".to_string(),
                    value: rand::rng()
                        .sample_iter(&Alphanumeric)
                        .take(36)
                        .map(char::from)
                        .collect(),
                    secret: true,
                    no_update: true,
                    stage: Some(stage),
                });
            }
        }
        None => {}
    }

    Ok(output)
}

async fn provision_app_db(app_config: &AppSpec) -> Result<ProvisionOutput> {
    let output = ProvisionOutput::default();

    match &app_config.db {
        Some(DatabaseConfig::Neon(_db_config)) => {
            todo!("support neon db")
        }
        Some(DatabaseConfig::Coolify(db_config)) => {
            let creator = coolify::new_resource_creator(
                db_config.project_name.clone(),
                db_config.server_name.clone(),
            )
            .await?;

            let _outputs = try_join_all(Stage::all().map(|stage| {
                let creator = creator.clone();

                async move {
                    anyhow::Ok((
                        stage,
                        creator
                            .create_postgres_db(
                                "production".to_string(),
                                format!("{}-postgres-{}", app_config.name, stage.env_name()),
                            )
                            .await?,
                    ))
                }
            }))
            .await?;
        }
        None => {}
    }

    Ok(output)
}

async fn provision_app_redis(app_config: &AppSpec) -> Result<ProvisionOutput> {
    let output = ProvisionOutput::default();

    match &app_config.redis {
        Some(RedisConfig::Coolify(redis_config)) => {
            let creator = coolify::new_resource_creator(
                redis_config.project_name.clone(),
                redis_config.server_name.clone(),
            )
            .await?;

            let _outputs = try_join_all(Stage::all().map(|stage| {
                let creator = creator.clone();

                async move {
                    anyhow::Ok((
                        stage,
                        creator
                            .create_redis(
                                stage.env_name().to_string(),
                                format!("{}-redis-{}", app_config.name, stage.env_name()),
                            )
                            .await?,
                    ))
                }
            }))
            .await?;

            // for (stage, info) in outputs {
            //     output.env_vars.push(EnvVar {
            //         key: "REDIS_URL".to_string(),
            //         value: "TODO".to_string(),
            //         secret: false,
            //         no_update: false,
            //         stage: Some(stage),
            //     });
            // }
        }
        None => {}
    }

    Ok(output)
}

async fn set_env_vars(file: &AppConfigFile, output: &ProvisionOutput) -> Result<()> {
    match &file.config.secrets {
        AppSecretsConfig::Vercel(v) => {
            let project = vercel::get_project(v.org.clone(), v.project.clone()).await?;

            vercel::set_env_vars(&v.org, &project.id, &output.env_vars)
                .await
                .with_context(|| {
                    format!(
                        "failed to set env vars for project `{}` for org `{}`",
                        project.id, v.org
                    )
                })?;
        }
    }

    Ok(())
}
