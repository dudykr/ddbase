use std::sync::Arc;

use anyhow::{Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use cached::proc_macro::cached;
use serde_derive::{Deserialize, Serialize};

#[derive(Debug)]
pub struct LogtoManagementApiConfig {
    pub endpoint: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[cached(result = true)]
pub async fn get_logto_management_api_config() -> Result<Arc<LogtoManagementApiConfig>> {
    let endpoint =
        std::env::var("LOGTO_ENDPOINT").unwrap_or_else(|_| "https://auth.dudy.app".to_string());
    let application_id =
        std::env::var("LOGTO_APPLICATION_ID").context("LOGTO_APPLICATION_ID is not set")?;
    let application_secret =
        std::env::var("LOGTO_APPLICATION_SECRET").context("LOGTO_APPLICATION_SECRET is not set")?;

    let response = reqwest::Client::new()
        .post(format!("{}/oidc/token", endpoint))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header(
            "Authorization",
            format!(
                "Basic {}",
                BASE64_STANDARD.encode(format!("{application_id}:{application_secret}"))
            ),
        )
        .body(
            "grant_type=client_credentials&resource=https://default.logto.app/api&scope=all"
                .to_string(),
        )
        .send()
        .await?;

    let token_response: TokenResponse = response.json().await?;

    Ok(Arc::new(LogtoManagementApiConfig {
        endpoint,
        api_key: token_response.access_token,
    }))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct App {
    /// Client ID
    pub id: String,
    pub name: String,
    /// Client Secret
    pub secret: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateAppRequest<'a> {
    r#type: &'a str,
    oidc_client_metadata: OidcClientMetadata,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OidcClientMetadata {
    redirect_uris: Vec<String>,
    post_logout_redirect_uris: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecretItem {
    value: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateAppRequest<'a> {
    r#type: &'a str,
    name: &'a str,
    description: &'a str,
    oidc_client_metadata: OidcClientMetadata,
}

pub async fn create_or_get_logto_application(
    api_config: Arc<LogtoManagementApiConfig>,
    app_name: &str,
    app_domain: &str,
    dev_port: u16,
) -> Result<App> {
    let LogtoManagementApiConfig { endpoint, api_key } = &*api_config;

    let response = reqwest::Client::new()
        .get(format!("{endpoint}/api/applications"))
        .bearer_auth(api_key)
        .send()
        .await?;

    let apps: Vec<App> = response.json().await?;

    if let Some(app) = apps.iter().find(|app| app.name == app_name) {
        // Update the existing app
        let response = reqwest::Client::new()
            .patch(format!("{endpoint}/api/applications/{}", app.id))
            .bearer_auth(api_key)
            .json(&UpdateAppRequest {
                r#type: "Traditional",
                oidc_client_metadata: OidcClientMetadata {
                    redirect_uris: vec![
                        format!("https://{app_domain}/api/auth/callback"),
                        format!("http://localhost:{dev_port}/api/auth/callback"),
                    ],
                    post_logout_redirect_uris: vec![
                        format!("https://{app_domain}"),
                        format!("http://localhost:{dev_port}"),
                    ],
                },
            })
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "failed to update app: {}",
                response.text().await?
            ));
        }

        let secrets = reqwest::Client::new()
            .get(format!("{endpoint}/api/applications/{}/secrets", app.id))
            .bearer_auth(api_key)
            .send()
            .await?;

        let secrets: Vec<SecretItem> = secrets.json().await?;

        return Ok(App {
            secret: secrets[0].value.clone(),
            ..app.clone()
        });
    }

    let response = reqwest::Client::new()
        .post(format!("{endpoint}/api/applications"))
        .bearer_auth(api_key)
        .json(&CreateAppRequest {
            r#type: "Traditional",
            name: app_name,
            description: "Dudy Web App",
            oidc_client_metadata: OidcClientMetadata {
                redirect_uris: vec![
                    format!("https://{app_domain}/api/auth/callback"),
                    format!("http://localhost:{dev_port}/api/auth/callback"),
                ],
                post_logout_redirect_uris: vec![
                    format!("https://{app_domain}"),
                    format!("http://localhost:{dev_port}"),
                ],
            },
        })
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "failed to create app: {}",
            response.text().await?
        ));
    }

    Box::pin(create_or_get_logto_application(
        api_config, app_name, app_domain, dev_port,
    ))
    .await
}
