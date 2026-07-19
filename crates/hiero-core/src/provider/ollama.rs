use super::{
    DreamOutput, DreamProvider, HttpProvider, ProviderProfile, Result, dialect_for_profile,
    dream_prompt, parse_output,
};
use crate::domain::{ShortTermMemory, TranslationContext};
use async_trait::async_trait;
pub struct OllamaProvider(HttpProvider);
impl OllamaProvider {
    pub fn new(client: reqwest::Client, profile: ProviderProfile) -> Result<Self> {
        let dialect = dialect_for_profile(&profile)?;
        HttpProvider::new(client, profile, dialect).map(Self)
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
