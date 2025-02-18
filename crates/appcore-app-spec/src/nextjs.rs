use schemars::JsonSchema;
use serde_derive::Deserialize;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct NextJsApp {
    #[serde(default)]
    pub translation: Option<NextJsTranslationConfig>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "service", rename_all = "snake_case", deny_unknown_fields)]
pub enum NextJsTranslationConfig {
    #[serde(rename = "deepl")]
    Deepl(NextJsDeeplTranslationConfig),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct NextJsDeeplTranslationConfig {
    pub lib: String,
}
