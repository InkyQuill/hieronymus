use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::{
    domain::{ShortTermMemory, TranslationContext},
    provider::{DreamOutput, DreamProvider},
};

use super::{DreamPhase, DreamPhaseError};

pub struct Crystallizer<'a> {
    provider: &'a dyn DreamProvider,
}

impl<'a> Crystallizer<'a> {
    #[must_use]
    pub const fn new(provider: &'a dyn DreamProvider) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl DreamPhase for Crystallizer<'_> {
    type Input = (TranslationContext, Vec<ShortTermMemory>);
    type Output = DreamOutput;

    async fn run(
        &self,
        _pool: &SqlitePool,
        (context, memories): Self::Input,
    ) -> Result<Self::Output, DreamPhaseError> {
        self.provider
            .crystallize(&context, &memories)
            .await
            .map_err(DreamPhaseError::Provider)
    }
}

pub use crate::dreaming::phases::{KnowledgeCrystalsPhase, RuleCrystalsPhase};
