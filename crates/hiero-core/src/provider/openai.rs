use super::{
    Dialect, DreamOutput, DreamProvider, HttpProvider, ProviderProfile, Result, dream_prompt,
    parse_output,
};
use crate::domain::{ShortTermMemory, TranslationContext};
use async_trait::async_trait;

pub struct OpenAiProvider(HttpProvider);
impl OpenAiProvider {
    pub fn new(client: reqwest::Client, profile: ProviderProfile) -> Result<Self> {
        HttpProvider::new(client, profile, Dialect::OpenAi).map(Self)
    }
}
#[async_trait]
impl DreamProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }
    async fn crystallize(
        &self,
        context: &TranslationContext,
        memories: &[ShortTermMemory],
    ) -> Result<DreamOutput> {
        parse_output(
            &self
                .0
                .generate(&dream_prompt(context, memories)?, false)
                .await?,
        )
    }
    async fn run_pass(
        &self,
        pass: &str,
        context: &TranslationContext,
        memories: &[ShortTermMemory],
    ) -> Result<serde_json::Value> {
        let prompt = format!("Pass: {pass}\n{}", dream_prompt(context, memories)?);
        serde_json::from_str(&self.0.generate(&prompt, false).await?)
            .map_err(|_| super::ProviderError::MalformedJson)
    }
}
