use super::{
    Dialect, DreamOutput, DreamProvider, HttpProvider, PassName, ProviderProfile,
    ProviderTransport, Result, dream_prompt, parse_output,
};
use crate::domain::{ShortTermMemory, TranslationContext};
use async_trait::async_trait;
use std::sync::Arc;

pub struct OpenAiProvider(HttpProvider);
impl OpenAiProvider {
    pub fn new(
        transport: Arc<dyn ProviderTransport>,
        profile: ProviderProfile,
        model: impl Into<String>,
    ) -> Result<Self> {
        HttpProvider::new(transport, profile, model, Dialect::OpenAi).map(Self)
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
        pass: PassName,
        context: &TranslationContext,
        memories: &[ShortTermMemory],
    ) -> Result<serde_json::Value> {
        serde_json::from_str(
            &self
                .0
                .generate(
                    &format!(
                        "Pass: {}\n{}",
                        pass.as_str(),
                        dream_prompt(context, memories)?
                    ),
                    false,
                )
                .await?,
        )
        .map_err(|_| super::ProviderError::MalformedJson)
    }
}
