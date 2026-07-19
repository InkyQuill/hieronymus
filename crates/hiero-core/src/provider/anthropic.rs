use super::{
    Dialect, DreamOutput, DreamProvider, HttpProvider, ProviderProfile, Result, dream_prompt,
    parse_output,
};
use crate::domain::{ShortTermMemory, TranslationContext};
use async_trait::async_trait;
pub struct AnthropicProvider(HttpProvider);
impl AnthropicProvider {
    pub fn new(client: reqwest::Client, profile: ProviderProfile) -> Result<Self> {
        HttpProvider::new(client, profile, Dialect::Anthropic).map(Self)
    }
}
#[async_trait]
impl DreamProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }
    async fn crystallize(
        &self,
        c: &TranslationContext,
        m: &[ShortTermMemory],
    ) -> Result<DreamOutput> {
        parse_output(&self.0.generate(&dream_prompt(c, m)?, false).await?)
    }
    async fn run_pass(
        &self,
        p: &str,
        c: &TranslationContext,
        m: &[ShortTermMemory],
    ) -> Result<serde_json::Value> {
        serde_json::from_str(
            &self
                .0
                .generate(&format!("Pass: {p}\n{}", dream_prompt(c, m)?), false)
                .await?,
        )
        .map_err(|_| super::ProviderError::MalformedJson)
    }
}
