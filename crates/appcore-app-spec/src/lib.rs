use schemars::JsonSchema;
use serde_derive::Deserialize;

pub use crate::nextjs::*;

mod nextjs;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AppSpec {
    pub name: String,

    /// The fully qualified domain name of the app.
    pub domain: String,

    pub dev: DevConfig,

    pub app: AppDetails,

    pub secrets: AppSecretsConfig,

    #[serde(default)]
    pub auth: Option<AppAuthConfig>,

    #[serde(default)]
    pub db: Option<DatabaseConfig>,

    #[serde(default)]
    pub redis: Option<RedisConfig>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppDetails {
    #[serde(rename = "nextjs-app")]
    NextJsApp(NextJsApp),
    #[serde(rename = "nodejs-api-server")]
    NodeJsApiServer(NodeJsApiServer),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct NodeJsApiServer {}

/// Configuration for the development environment.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DevConfig {
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppSecretsConfig {
    #[serde(rename = "vercel")]
    Vercel(VercelSecretsConfig),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct VercelSecretsConfig {
    pub org: String,
    pub project: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppAuthConfig {
    #[serde(rename = "logto")]
    Logto(LogtoAuthConfig),
}

/// Configuration for the Logto authentication provider.
///
/// This configuration assumes `https://$domain/api/auth/callback` and
/// `http://localhost:$port/api/auth/callback` is the callback url for the
/// application.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct LogtoAuthConfig {
    pub app_name: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub enum DatabaseConfig {
    #[serde(rename = "neon")]
    Neon(NeonDatabaseConfig),
    #[serde(rename = "coolify")]
    Coolify(CoolifyDatabaseConfig),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct NeonDatabaseConfig {
    pub project_name: String,
}

/// Configuration for the Coolify database provider.
///
/// This configuration assumes the environment for the database is `production`
/// and `development`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CoolifyDatabaseConfig {
    pub project_name: String,
    pub server_name: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub enum RedisConfig {
    #[serde(rename = "coolify")]
    Coolify(CoolifyRedisConfig),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CoolifyRedisConfig {
    pub project_name: String,
    pub server_name: String,
}
