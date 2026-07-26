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

    pub(crate) fn with_credential_resolver(
        transport: Arc<dyn ProviderTransport>,
        credentials: Arc<dyn super::CredentialResolver>,
        profile: ProviderProfile,
        model: impl Into<String>,
    ) -> Result<Self> {
        HttpProvider::new_with_credential_resolver(
            transport,
            credentials,
            profile,
            model,
            Dialect::OpenAi,
        )
        .map(Self)
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
            self.0
                .generate(&dream_prompt(context, memories)?, false)
                .await?,
        )
        .await
    }
    async fn run_pass(
        &self,
        pass: PassName,
        context: &TranslationContext,
        memories: &[ShortTermMemory],
    ) -> Result<super::ProviderPassOutput> {
        super::parse_pass_output(
            self.0
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
        .await
    }

    async fn run_pass_with_prompt(
        &self,
        _pass: PassName,
        _context: &TranslationContext,
        _memories: &[ShortTermMemory],
        prompt: &str,
    ) -> Result<super::ProviderPassOutput> {
        super::parse_pass_output(self.0.generate(prompt, false).await?).await
    }
}
