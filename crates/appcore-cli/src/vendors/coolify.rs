use std::sync::Arc;

use anyhow::{Context, Result};
use cached::proc_macro::cached;
use rand::Rng;
use reqwest::StatusCode;
use serde_derive::{Deserialize, Serialize};
use tracing::warn;

async fn get_token() -> Result<String> {
    std::env::var("COOLIFY_TOKEN").context("COOLIFY_TOKEN is not set")
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: u64,
    pub uuid: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub environments: Vec<Environment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Environment {
    pub id: u64,
    pub name: String,
    pub project_id: u64,
}

#[derive(Debug, Serialize)]
struct CreateProjectRequest<'a> {
    name: &'a str,
    description: &'a str,
}

#[cached(result = true)]
pub async fn get_or_create_project(name: String) -> Result<Arc<Project>> {
    let projects = reqwest::Client::new()
        .get("https://app.coolify.io/api/v1/projects")
        .bearer_auth(get_token().await?)
        .send()
        .await?;

    let projects: Vec<Project> = projects.json().await.context("failed to parse projects")?;

    if let Some(project) = projects.iter().find(|p| p.name == name) {
        return Ok(Arc::new(project.clone()));
    }

    let resp = reqwest::Client::new()
        .post("https://app.coolify.io/api/v1/projects")
        .bearer_auth(get_token().await?)
        .json(&CreateProjectRequest {
            name: &name,
            description: &format!("Project for {}", name),
        })
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "failed to create project: {}",
            resp.text().await?
        ));
    }

    Box::pin(get_or_create_project(name)).await
}

#[derive(Debug, Clone, Deserialize)]
pub struct Server {
    pub name: String,
    pub uuid: String,
}

#[cached(result = true)]
pub async fn get_server(name: String) -> Result<Arc<Server>> {
    let servers = reqwest::Client::new()
        .get("https://app.coolify.io/api/v1/servers")
        .bearer_auth(get_token().await?)
        .send()
        .await?;

    let servers: Vec<Server> = servers.json().await?;

    if let Some(server) = servers.into_iter().find(|s| s.name == name) {
        return Ok(Arc::new(server));
    }

    Err(anyhow::anyhow!("failed to get server `{}`", name))
}

#[cached(result = true)]
pub async fn new_resource_creator(
    project_name: String,
    server_name: String,
) -> Result<Arc<ResourceCreator>> {
    let project = get_or_create_project(project_name)
        .await
        .context("failed to get or create coolify project")?;

    let server = get_server(server_name)
        .await
        .context("failed to get coolify server")?;

    Ok(Arc::new(ResourceCreator { project, server }))
}

#[derive(Debug, Clone)]
pub struct ResourceCreator {
    pub project: Arc<Project>,
    pub server: Arc<Server>,
}

#[derive(Debug, Serialize)]
struct CreatePostgresRequest<'a> {
    server_uuid: &'a str,
    project_uuid: &'a str,
    environment_name: &'a str,
    name: &'a str,
}

#[derive(Debug, Serialize)]
struct CreateRedisRequest<'a> {
    server_uuid: &'a str,
    project_uuid: &'a str,
    environment_name: &'a str,
    name: &'a str,
}

#[derive(Debug, Default, Serialize)]
struct UpdateDbRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_public: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limits_memory: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limits_memory_swap: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limits_memory_swappiness: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limits_memory_reservation: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limits_cpus: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limits_cpuset: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limits_cpu_shares: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postgres_user: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postgres_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postgres_db: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postgres_initdb_args: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postgres_host_auth_method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postgres_conf: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clickhouse_admin_user: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clickhouse_admin_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dragonfly_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redis_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redis_conf: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keydb_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keydb_conf: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mariadb_conf: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mariadb_root_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mariadb_user: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mariadb_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mariadb_database: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mongo_conf: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mongo_initdb_root_username: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mongo_initdb_root_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mongo_initdb_database: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mysql_root_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mysql_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mysql_user: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mysql_database: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mysql_conf: Option<&'a str>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseInfo {
    pub uuid: String,
    pub name: String,
    #[serde(default)]
    pub config_hash: Option<String>,
    #[serde(default)]
    pub custom_docker_run_options: Option<String>,
    pub database_type: String,
    pub image: String,
    #[serde(default)]
    pub is_public: bool,
    #[serde(default)]
    pub last_online_at: Option<String>,

    #[serde(default)]
    pub public_port: Option<u16>,

    #[serde(flatten)]
    pub detail: DbDetail,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum DbDetail {
    Postgres(PostgresDetail),
    Redis(RedisDetail),
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostgresDetail {
    pub postgres_user: String,
    pub postgres_db: String,
}

async fn list_databases() -> Result<Vec<DatabaseInfo>> {
    let resp = reqwest::Client::new()
        .get("https://app.coolify.io/api/v1/databases")
        .bearer_auth(get_token().await?)
        .send()
        .await?;

    let databases: Vec<DatabaseInfo> = resp.json().await?;

    Ok(databases)
}

async fn start_db(uuid: &str) -> Result<()> {
    // Start the database
    let resp = reqwest::Client::new()
        .post(format!(
            "https://app.coolify.io/api/v1/databases/{}/start",
            uuid
        ))
        .bearer_auth(get_token().await?)
        .send()
        .await?;

    if resp.status() == StatusCode::BAD_REQUEST {
        // Do not return error, just warn
        warn!("failed to start database: {}", resp.text().await?);
        return Ok(());
    }

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "failed to start database: {}",
            resp.text().await?
        ));
    }

    Ok(())
}

async fn make_db_public(db: &DatabaseInfo) -> Result<()> {
    let resp = reqwest::Client::new()
        .patch(format!(
            "https://app.coolify.io/api/v1/databases/{}",
            db.uuid
        ))
        .bearer_auth(get_token().await?)
        .json(&UpdateDbRequest {
            is_public: Some(true),
            public_port: Some(rand::rng().random_range(10000..65535)),
            ..Default::default()
        })
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "failed to make db public: {}",
            resp.text().await?
        ));
    }

    Ok(())
}

async fn prepare_db(db: DatabaseInfo) -> Result<DatabaseInfo> {
    start_db(&db.uuid)
        .await
        .context("failed to start database")?;

    make_db_public(&db)
        .await
        .context("failed to make db public")?;

    Ok(db)
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisDetail {}

impl ResourceCreator {
    pub async fn create_postgres_db(
        self: Arc<Self>,
        env_name: String,
        db_name: String,
    ) -> Result<DatabaseInfo> {
        let databases = list_databases().await?;
        if let Some(db) = databases.iter().find(|db| db.name == db_name) {
            return prepare_db(db.clone()).await;
        }

        let resp = reqwest::Client::new()
            .post("https://app.coolify.io/api/v1/databases/postgresql")
            .bearer_auth(get_token().await?)
            .json(&CreatePostgresRequest {
                server_uuid: &self.server.uuid,
                project_uuid: &self.project.uuid,
                environment_name: &env_name,
                name: &db_name,
            })
            .send()
            .await
            .context("failed to create postgres db")?;

        let postgres_info: DatabaseInfo =
            resp.json().await.context("failed to parse postgres info")?;

        prepare_db(postgres_info.clone()).await
    }

    pub async fn create_redis(
        self: Arc<Self>,
        environemnt_name: String,
        redis_name: String,
    ) -> Result<DatabaseInfo> {
        let databases = list_databases().await?;

        if let Some(db) = databases.iter().find(|db| db.name == redis_name) {
            return prepare_db(db.clone()).await;
        }

        let resp = reqwest::Client::new()
            .post("https://app.coolify.io/api/v1/databases/redis")
            .bearer_auth(get_token().await?)
            .json(&CreateRedisRequest {
                server_uuid: &self.server.uuid,
                project_uuid: &self.project.uuid,
                environment_name: &environemnt_name,
                name: &redis_name,
            })
            .send()
            .await
            .context("failed to create redis")?;

        let redis_info: DatabaseInfo = resp.json().await.context("failed to parse redis info")?;

        prepare_db(redis_info.clone()).await
    }
}
