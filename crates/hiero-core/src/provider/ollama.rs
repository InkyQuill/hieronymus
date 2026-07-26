use super::{
    Dialect, DreamOutput, DreamProvider, HttpProvider, PassName, ProviderProfile,
    ProviderTransport, Result, dream_prompt, parse_output,
};
use crate::domain::{ShortTermMemory, TranslationContext};
use async_trait::async_trait;
use std::sync::Arc;
pub struct OllamaProvider(HttpProvider);
impl OllamaProvider {
    pub fn new(
        transport: Arc<dyn ProviderTransport>,
        profile: ProviderProfile,
        model: impl Into<String>,
    ) -> Result<Self> {
        HttpProvider::new(transport, profile, model, Dialect::Ollama).map(Self)
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
            Dialect::Ollama,
        )
        .map(Self)
    }
}
#[async_trait]
impl DreamProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }
    async fn crystallize(
        &self,
        c: &TranslationContext,
        m: &[ShortTermMemory],
    ) -> Result<DreamOutput> {
        parse_output(self.0.generate(&dream_prompt(c, m)?, false).await?).await
    }
    async fn run_pass(
        &self,
        p: PassName,
        c: &TranslationContext,
        m: &[ShortTermMemory],
    ) -> Result<super::ProviderPassOutput> {
        super::parse_pass_output(
            self.0
                .generate(
                    &format!("Pass: {}\n{}", p.as_str(), dream_prompt(c, m)?),
                    false,
                )
                .await?,
        )
        .await
    }
    async fn run_pass_with_prompt(
        &self,
        _p: PassName,
        _c: &TranslationContext,
        _m: &[ShortTermMemory],
        prompt: &str,
    ) -> Result<super::ProviderPassOutput> {
        super::parse_pass_output(self.0.generate(prompt, false).await?).await
    }
}
