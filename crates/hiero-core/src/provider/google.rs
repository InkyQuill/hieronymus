use super::{
    Dialect, DreamOutput, DreamProvider, HttpProvider, PassName, ProviderProfile,
    ProviderTransport, Result, dream_prompt, parse_output,
};
use crate::domain::{ShortTermMemory, TranslationContext};
use async_trait::async_trait;
use std::sync::Arc;
pub struct GoogleProvider(HttpProvider);
impl GoogleProvider {
    pub fn new(
        transport: Arc<dyn ProviderTransport>,
        profile: ProviderProfile,
        model: impl Into<String>,
    ) -> Result<Self> {
        HttpProvider::new(transport, profile, model, Dialect::Google).map(Self)
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
            Dialect::Google,
        )
        .map(Self)
    }
}
#[async_trait]
impl DreamProvider for GoogleProvider {
    fn name(&self) -> &str {
        "google"
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
        p: PassName,
        c: &TranslationContext,
        m: &[ShortTermMemory],
    ) -> Result<serde_json::Value> {
        serde_json::from_str(
            &self
                .0
                .generate(
                    &format!("Pass: {}\n{}", p.as_str(), dream_prompt(c, m)?),
                    false,
                )
                .await?,
        )
        .map_err(|_| super::ProviderError::MalformedJson)
    }
}
