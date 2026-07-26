use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use hiero_core::{
    config::HieronymusConfig,
    domain::{
        AddConceptFacetInput, AddMemoryInput, Concept, ConceptFacet, ConceptFilter,
        ConceptProposalStore, ConceptStore, CreateConceptPrimitiveInput, Crystal, CrystalStore,
        FeedbackStore, RuleFilter, TermProposal, Termbase, TranslationContext,
        UpdateConceptFacetInput, UpdateConceptInput, WorkspaceStore,
    },
    dreaming::{
        CycleOptions, DreamConfig, DreamPhaseError, DreamProviderResolver, DreamService,
        WorkflowProfile,
    },
    provider::{ModelCache, ProviderCatalog, ProviderRegistry, ReqwestTransportOptions},
    rag::{ImportOptions, RagStore, RetrievalMode, SearchOptions, SourceType},
    recall::RecallService,
    registry::SeriesRegistry,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::{api::events::AdminNotifier, daemon::DAEMON_DREAM_CLEANUP_DEADLINE};

use super::{SafeMcpError, tools::*};

#[async_trait]
pub trait McpBackend: Send + Sync {
    async fn call(&self, name: &str, arguments: Value) -> Result<Value>;
}

#[async_trait]
pub trait DreamRunner: Send + Sync {
    async fn run(&self, provider: Option<&str>, wait: bool) -> Result<Value>;
}

#[derive(Clone)]
pub struct StoreDreamRunner {
    pool: SqlitePool,
    config: Arc<HieronymusConfig>,
    components: Option<(DreamConfig, ProviderCatalog, Arc<dyn DreamProviderResolver>)>,
    notifier: Option<AdminNotifier>,
}

impl StoreDreamRunner {
    #[must_use]
    pub fn new(pool: SqlitePool, config: Arc<HieronymusConfig>) -> Self {
        Self {
            pool,
            config,
            components: None,
            notifier: None,
        }
    }

    #[must_use]
    pub fn with_notifier(mut self, notifier: AdminNotifier) -> Self {
        self.notifier = Some(notifier);
        self
    }

    #[must_use]
    pub fn with_components(
        mut self,
        dream_config: DreamConfig,
        catalog: ProviderCatalog,
        resolver: Arc<dyn DreamProviderResolver>,
    ) -> Self {
        self.components = Some((dream_config, catalog, resolver));
        self
    }

    async fn components(
        &self,
        provider: Option<&str>,
    ) -> Result<(DreamConfig, ProviderCatalog, Arc<dyn DreamProviderResolver>)> {
        if let Some(components) = &self.components {
            return Ok(components.clone());
        }
        let config = self.config.clone();
        let (dream_config, catalog) = tokio::task::spawn_blocking(move || {
            let dream_config = DreamConfig::load(&config)?;
            let catalog = ProviderCatalog::load(config.provider_config_path())
                .map_err(hiero_core::dreaming::DreamConfigError::File)?;
            Ok::<_, hiero_core::dreaming::DreamConfigError>((dream_config, catalog))
        })
        .await
        .context("dream configuration task failed")??;
        let registry = Arc::new(ProviderRegistry::production(
            ModelCache::new(128, 1_000_000, std::time::Duration::from_secs(24 * 60 * 60)),
            ReqwestTransportOptions::default(),
        ));
        let resolver_catalog = Arc::new(catalog.clone());
        let provider = provider.map(str::to_owned);
        let resolver = Arc::new(move |workflow: &WorkflowProfile| {
            registry
                .resolve(
                    &resolver_catalog,
                    provider.as_deref().unwrap_or(&workflow.provider),
                    &workflow.model,
                )
                .map(Arc::from)
                .map_err(DreamPhaseError::Provider)
        });
        Ok((dream_config, catalog, resolver))
    }
}

#[async_trait]
impl DreamRunner for StoreDreamRunner {
    async fn run(&self, provider: Option<&str>, wait: bool) -> Result<Value> {
        let (dream_config, catalog, resolver) = self.components(provider).await?;
        if let Some(notifier) = &self.notifier {
            notifier.notify_refresh();
        }
        let run = DreamService::new_with_catalog(
            &self.pool,
            &self.config,
            &dream_config,
            resolver,
            catalog,
        )
        .with_cleanup_deadline(DAEMON_DREAM_CLEANUP_DEADLINE)
        .run_all(CycleOptions {
            owner: "mcp".into(),
            wait,
            ..CycleOptions::default()
        })
        .await;
        if let Some(notifier) = &self.notifier {
            notifier.notify_refresh();
        }
        let run = run?;
        Ok(json!({
            "cycle_id": run.cycle_id,
            "status": run.status,
            "provider": run.provider,
            "input_count": run.input_count,
            "created_crystal_count": run.created_crystal_count,
            "proposal_count": run.proposal_count,
        }))
    }
}

#[derive(Clone)]
pub struct StoreMcpBackend {
    pool: SqlitePool,
    config: Arc<HieronymusConfig>,
    dream_runner: Option<Arc<dyn DreamRunner>>,
    notifier: Option<AdminNotifier>,
}

impl StoreMcpBackend {
    #[must_use]
    pub fn new(pool: SqlitePool, config: Arc<HieronymusConfig>) -> Self {
        Self {
            pool,
            config,
            dream_runner: None,
            notifier: None,
        }
    }

    #[must_use]
    pub fn with_notifier(mut self, notifier: AdminNotifier) -> Self {
        self.notifier = Some(notifier);
        self
    }

    #[must_use]
    pub fn with_dream_runner(mut self, runner: Arc<dyn DreamRunner>) -> Self {
        self.dream_runner = Some(runner);
        self
    }

    pub async fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        <Self as McpBackend>::call(self, name, arguments).await
    }

    async fn translation_context(
        &self,
        series_slug: &str,
        source_language: Option<&str>,
        target_language: Option<&str>,
        volume: &str,
        chapter: &str,
    ) -> Result<TranslationContext> {
        let series = SeriesRegistry::new(&self.pool)
            .get(series_slug)
            .await
            .with_context(|| format!("unknown series `{series_slug}`"))?;
        let source = source_language
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&series.series.default_source_language);
        let target = target_language
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&series.series.default_target_language);
        if source.trim().is_empty() || target.trim().is_empty() {
            bail!("series `{series_slug}` has no complete language direction");
        }
        let story_scopes = [
            (!volume.trim().is_empty()).then(|| format!("volume:{}", volume.trim())),
            (!chapter.trim().is_empty()).then(|| format!("chapter:{}", chapter.trim())),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        Ok(
            TranslationContext::new(series_slug, source, target).with_metadata(
                &series.language_tags,
                &story_scopes,
                &[],
                &[],
            ),
        )
    }
}

#[async_trait]
impl McpBackend for StoreMcpBackend {
    async fn call(&self, name: &str, arguments: Value) -> Result<Value> {
        macro_rules! input {
            ($type:ty) => {
                decode::<$type>(name, arguments.clone())?
            };
        }
        let registry = SeriesRegistry::new(&self.pool);
        let concepts = ConceptStore::new(&self.pool);
        let crystals = CrystalStore::new(&self.pool);
        let workspace = WorkspaceStore::new(&self.pool);
        let result = match name {
            "hieronymus_status" => {
                let _: NoArgs = input!(NoArgs);
                json_value(crate::api::system::status_report(&self.pool, &self.config).await?)
            }
            "hieronymus_series_create" | "hieronymus_series_init" => {
                let input: SeriesCreateInput = input!(SeriesCreateInput);
                let language_tags =
                    (!input.language_tags.is_empty()).then_some(input.language_tags.as_slice());
                let series = registry
                    .create_with_language_tags(
                        &input.slug,
                        &input.title,
                        &input.source_language,
                        &input.target_language,
                        language_tags,
                    )
                    .await?;
                json_value(series)
            }
            "hieronymus_series_list" => {
                let _: NoArgs = input!(NoArgs);
                json_value(registry.list().await?)
            }
            "hieronymus_series_set_language_tags" => {
                let input: SeriesTagsInput = input!(SeriesTagsInput);
                registry
                    .set_language_tags(input.series_id, &input.language_tags)
                    .await?;
                let series = registry
                    .list()
                    .await?
                    .into_iter()
                    .find(|series| series.series.id == input.series_id)
                    .ok_or_else(|| anyhow!("unknown series id: {}", input.series_id))?;
                json_value(series)
            }
            "hieronymus_concept_list" => {
                let input: ConceptListInput = input!(ConceptListInput);
                let mut rows = if let Some(slug) = input.series_slug {
                    let mut rows = concepts
                        .list(ConceptFilter {
                            scope_type: "series".into(),
                            scope_key: format!("series:{slug}"),
                            status: input.status.clone(),
                        })
                        .await?;
                    if input.include_global {
                        rows.extend(
                            concepts
                                .list(ConceptFilter {
                                    scope_type: "global".into(),
                                    scope_key: String::new(),
                                    status: input.status,
                                })
                                .await?,
                        );
                    }
                    rows
                } else {
                    concepts.list_all(input.status).await?
                };
                if let Some(tag) = input.semantic_tag {
                    rows.retain(|concept| concept.semantic_tags.contains(&tag));
                }
                rows.sort_by_key(|concept| concept.id);
                Ok(Value::Array(
                    rows.iter().map(concept_payload).collect::<Vec<_>>(),
                ))
            }
            "hieronymus_concept_get" => {
                let input: ConceptIdInput = input!(ConceptIdInput);
                Ok(concept_payload(
                    &concepts.get_enriched(input.concept_id).await?,
                ))
            }
            "hieronymus_concept_create" => {
                let input: ConceptCreateInput = input!(ConceptCreateInput);
                let (scope_type, scope_key) = if input.series_slug.is_empty() {
                    (input.scope_type, input.scope_key)
                } else {
                    registry.get(&input.series_slug).await?;
                    ("series".into(), format!("series:{}", input.series_slug))
                };
                let created = concepts
                    .create_primitive(CreateConceptPrimitiveInput {
                        canonical_name: input.canonical_name,
                        description: input.description,
                        status: input.status,
                        confidence: input.confidence,
                        scope_type,
                        scope_key,
                        semantic_tags: input.semantic_tags,
                    })
                    .await?;
                Ok(concept_payload(&created))
            }
            "hieronymus_concept_update" => {
                let input: ConceptUpdateInput = input!(ConceptUpdateInput);
                concepts
                    .update(
                        input.concept_id,
                        UpdateConceptInput {
                            description: input.description,
                            status: input.status,
                            confidence: input.confidence,
                        },
                    )
                    .await?;
                Ok(concept_payload(
                    &concepts.get_enriched(input.concept_id).await?,
                ))
            }
            "hieronymus_concept_archive" => {
                let input: ConceptArchiveInput = input!(ConceptArchiveInput);
                concepts.archive(input.concept_id, &input.reason).await?;
                Ok(concept_payload(
                    &concepts.get_enriched(input.concept_id).await?,
                ))
            }
            "hieronymus_concept_merge" => {
                let input: ConceptMergeInput = input!(ConceptMergeInput);
                concepts
                    .merge_concepts(
                        input.source_concept_id,
                        input.target_concept_id,
                        &input.reason,
                    )
                    .await?;
                Ok(json!({
                    "source": concept_payload(&concepts.get_enriched(input.source_concept_id).await?),
                    "target": concept_payload(&concepts.get_enriched(input.target_concept_id).await?),
                }))
            }
            "hieronymus_concept_rename" => {
                let input: ConceptRenameInput = input!(ConceptRenameInput);
                concepts
                    .rename_concept_with_source(
                        input.concept_id,
                        &input.new_label,
                        "mcp",
                        input.source_crystal_id,
                    )
                    .await?;
                Ok(concept_payload(
                    &concepts.get_enriched(input.concept_id).await?,
                ))
            }
            "hieronymus_concept_facet_add" => {
                let input: FacetAddInput = input!(FacetAddInput);
                let facet_type = input
                    .facet_type
                    .or(input.kind)
                    .unwrap_or_else(|| "name".into());
                let facet = concepts
                    .add_facet_typed(AddConceptFacetInput {
                        concept_id: input.concept_id,
                        language: input.language,
                        facet_type,
                        value: input.value,
                        language_tags: input.language_tags,
                        story_scopes: input.story_scopes,
                        semantic_tags: input.semantic_tags,
                        source_crystal_id: input.source_crystal_id,
                        confidence: input.confidence,
                        is_canonical: input.is_canonical,
                    })
                    .await?;
                let facet = concepts
                    .list_facets(facet.concept_id)
                    .await?
                    .into_iter()
                    .find(|candidate| candidate.id == facet.id)
                    .ok_or_else(|| anyhow!("created facet disappeared"))?;
                Ok(facet_payload(&facet))
            }
            "hieronymus_concept_facet_update" => {
                let input: FacetUpdateInput = input!(FacetUpdateInput);
                let facet = concepts
                    .update_facet_typed(
                        input.facet_id,
                        UpdateConceptFacetInput {
                            value: input.value,
                            language: input.language,
                            facet_type: input.facet_type.or(input.kind),
                            language_tags: input.language_tags,
                            story_scopes: input.story_scopes,
                            semantic_tags: input.semantic_tags,
                            source_crystal_id: input.source_crystal_id,
                            confidence: input.confidence,
                            is_canonical: input.is_canonical,
                        },
                    )
                    .await?;
                let facet = concepts
                    .list_facets(facet.concept_id)
                    .await?
                    .into_iter()
                    .find(|candidate| candidate.id == facet.id)
                    .ok_or_else(|| anyhow!("updated facet disappeared"))?;
                Ok(facet_payload(&facet))
            }
            "hieronymus_concept_facet_list" => {
                let input: ConceptIdInput = input!(ConceptIdInput);
                Ok(Value::Array(
                    concepts
                        .list_facets(input.concept_id)
                        .await?
                        .iter()
                        .map(facet_payload)
                        .collect(),
                ))
            }
            "hieronymus_concept_facet_set_canonical" => {
                let input: FacetCanonicalInput = input!(FacetCanonicalInput);
                let belongs_to_concept = concepts
                    .list_facets(input.concept_id)
                    .await?
                    .into_iter()
                    .any(|facet| facet.id == input.facet_id);
                if !belongs_to_concept {
                    bail!("facet does not belong to concept");
                }
                concepts.set_canonical_facet(input.facet_id).await?;
                let facet = concepts
                    .list_facets(input.concept_id)
                    .await?
                    .into_iter()
                    .find(|facet| facet.id == input.facet_id)
                    .ok_or_else(|| anyhow!("canonical facet disappeared after update"))?;
                Ok(facet_payload(&facet))
            }
            "hieronymus_concept_semantic_tags_set" => {
                let input: ConceptTagsInput = input!(ConceptTagsInput);
                concepts
                    .set_semantic_tags(input.concept_id, &input.semantic_tags)
                    .await?;
                Ok(concept_payload(
                    &concepts.get_enriched(input.concept_id).await?,
                ))
            }
            "hieronymus_crystal_link_concept" => {
                let input: CrystalLinkInput = input!(CrystalLinkInput);
                concepts
                    .link_crystal(
                        input.crystal_id,
                        input.concept_id,
                        &input.link_type,
                        input.confidence,
                    )
                    .await?;
                Ok(crystal_payload(&crystals.get(input.crystal_id).await?))
            }
            "hieronymus_crystal_story_scopes_set" => {
                let input: CrystalScopesInput = input!(CrystalScopesInput);
                crystals
                    .set_story_scopes_with_confidence(
                        input.crystal_id,
                        &input.story_scopes,
                        input.confidence,
                    )
                    .await?;
                Ok(crystal_payload(&crystals.get(input.crystal_id).await?))
            }
            "hieronymus_crystal_semantic_tags_set" => {
                let input: CrystalTagsInput = input!(CrystalTagsInput);
                crystals
                    .set_semantic_tags_with_confidence(
                        input.crystal_id,
                        &input.semantic_tags,
                        input.confidence,
                    )
                    .await?;
                Ok(crystal_payload(&crystals.get(input.crystal_id).await?))
            }
            "hieronymus_rule_crystals_list" => {
                let input: RuleListInput = input!(RuleListInput);
                json_value(
                    crystals
                        .list_rule_intent(RuleFilter {
                            status: input.status,
                            series_slug: input.series_slug,
                            limit: input.limit,
                        })
                        .await?,
                )
            }
            "hieronymus_rule_crystal_archive" => {
                let input: CrystalIdInput = input!(CrystalIdInput);
                json_value(crystals.archive(input.crystal_id).await?)
            }
            "hieronymus_rule_crystal_validate" => {
                let input: CrystalIdInput = input!(CrystalIdInput);
                json_value(crystals.validate_rule(input.crystal_id).await?)
            }
            "hieronymus_termbase_contract" => {
                let input: ContextTextInput = input!(ContextTextInput);
                let context = self
                    .translation_context(
                        &input.series_slug,
                        input.source_language.as_deref(),
                        input.target_language.as_deref(),
                        &input.volume,
                        &input.chapter,
                    )
                    .await?;
                json_value(
                    Termbase::new(&self.pool, context)
                        .contract(&input.raw_text)
                        .await?,
                )
            }
            "hieronymus_termbase_validate" => {
                let input: ValidateTextInput = input!(ValidateTextInput);
                let context = self
                    .translation_context(
                        &input.series_slug,
                        input.source_language.as_deref(),
                        input.target_language.as_deref(),
                        &input.volume,
                        &input.chapter,
                    )
                    .await?;
                json_value(
                    Termbase::new(&self.pool, context)
                        .validate(&input.translated_text, Some(&input.raw_text), None)
                        .await?,
                )
            }
            "hieronymus_termbase_propose" => {
                let input: TermProposeInput = input!(TermProposeInput);
                let context = self
                    .translation_context(
                        &input.series_slug,
                        input.source_language.as_deref(),
                        input.target_language.as_deref(),
                        &input.volume,
                        &input.chapter,
                    )
                    .await?;
                let id = Termbase::new(&self.pool, context.clone())
                    .propose(TermProposal {
                        series_slug: context.series_slug,
                        source_language: context.source_language,
                        target_language: context.target_language,
                        category: input.category,
                        source_text: input.source_text,
                        canonical_translation: input.canonical_translation,
                        tags: input.tags,
                        notes: input.notes,
                    })
                    .await?;
                Ok(json!({"term_id": id}))
            }
            "hieronymus_termbase_approve" => {
                let input: TermApproveInput = input!(TermApproveInput);
                let mut context = Termbase::candidate_context(&self.pool, input.term_id).await?;
                if context.series_slug != input.series_slug {
                    bail!("term candidate does not belong to requested series");
                }
                let series = registry.get(&context.series_slug).await?;
                let mut story_scopes = context.story_scopes.clone();
                story_scopes.extend(
                    [
                        (!input.volume.trim().is_empty())
                            .then(|| format!("volume:{}", input.volume.trim())),
                        (!input.chapter.trim().is_empty())
                            .then(|| format!("chapter:{}", input.chapter.trim())),
                    ]
                    .into_iter()
                    .flatten(),
                );
                let semantic_tags = context.semantic_tags.clone();
                let tags = context.tags.clone();
                context = context.with_metadata(
                    &series.language_tags,
                    &story_scopes,
                    &semantic_tags,
                    &tags,
                );
                Termbase::new(&self.pool, context)
                    .approve_term(input.term_id)
                    .await?;
                Ok(json!({"term_id": input.term_id, "approved": true}))
            }
            "hieronymus_memory_search" => {
                let input: MemorySearchInput = input!(MemorySearchInput);
                let context = self
                    .translation_context(
                        &input.series_slug,
                        input.source_language.as_deref(),
                        input.target_language.as_deref(),
                        "",
                        "",
                    )
                    .await?;
                let session_id = workspace.get_or_start_default_session(&context).await?.id;
                json_value(
                    RecallService::new(&self.pool)
                        .recall(session_id, &context, &input.query, input.limit)
                        .await?,
                )
            }
            "hieronymus_rag_import" => {
                let input: RagImportInput = input!(RagImportInput);
                registry.get(&input.series_slug).await?;
                let source_type = serde_json::from_value::<SourceType>(json!(input.source_type))
                    .context("invalid source_type")?;
                json_value(
                    RagStore::new(&self.pool)
                        .import_file(
                            &input.series_slug,
                            &input.path,
                            ImportOptions {
                                source_ref: input.source_ref,
                                source_type: Some(source_type),
                                language_tags: input.language_tags,
                                story_scopes: input.story_scopes,
                                semantic_tags: input.semantic_tags,
                                managed_root: Some(self.config.data_root.join("rag")),
                            },
                        )
                        .await?,
                )
            }
            "hieronymus_rag_search" => {
                let input: RagSearchInput = input!(RagSearchInput);
                registry.get(&input.series_slug).await?;
                json_value(
                    RagStore::new(&self.pool)
                        .search(
                            &input.series_slug,
                            &input.query,
                            SearchOptions {
                                limit: input.limit,
                                retrieval_mode: RetrievalMode::Lexical,
                                ..SearchOptions::default()
                            },
                        )
                        .await?,
                )
            }
            "hieronymus_memory_add" => {
                let input: MemoryAddInput = input!(MemoryAddInput);
                let context = self
                    .translation_context(
                        &input.series_slug,
                        input.source_language.as_deref(),
                        input.target_language.as_deref(),
                        "",
                        "",
                    )
                    .await?;
                let session_id = workspace.get_or_start_default_session(&context).await?.id;
                let result = workspace
                    .add_short_term(
                        session_id,
                        AddMemoryInput {
                            source_role: "user".into(),
                            kind: if matches!(input.kind.as_str(), "rule" | "correction") {
                                "correction".into()
                            } else {
                                "note".into()
                            },
                            text: input.text,
                            source_ref: input.source_ref,
                            metadata: json!({"legacy_kind": input.kind, "importance": input.importance}),
                            ..AddMemoryInput::default()
                        },
                    )
                    .await?;
                Ok(json!({"memory_id": result.memory.id, "storage": "short_term"}))
            }
            "hieronymus_session_start" => {
                let input: SessionStartInput = input!(SessionStartInput);
                let context = self
                    .translation_context(
                        &input.series_slug,
                        input.source_language.as_deref(),
                        input.target_language.as_deref(),
                        &input.volume,
                        &input.chapter,
                    )
                    .await?;
                let session = workspace
                    .start_session(&context, &input.task_type, &input.volume, &input.chapter)
                    .await?;
                Ok(json!({"session_id": session.id}))
            }
            "hieronymus_session_complete" => {
                let input: SessionIdInput = input!(SessionIdInput);
                let completed = workspace.complete_session(input.session_id).await?;
                Ok(json!({"session_id": input.session_id, "completed": completed}))
            }
            "hieronymus_short_term_add" => {
                let input: ShortAddInput = input!(ShortAddInput);
                let result = workspace
                    .add_short_term(input.session_id, short_memory(input))
                    .await?;
                Ok(json!({"memory_id": result.memory.id}))
            }
            "hieronymus_short_term_add_batch" => {
                let input: ShortBatchInput = input!(ShortBatchInput);
                let items = input.items.into_iter().map(short_item).collect::<Vec<_>>();
                let results = workspace
                    .add_short_term_batch(input.session_id, &items)
                    .await?;
                let ids = results
                    .into_iter()
                    .map(|result| result.memory.id)
                    .collect::<Vec<_>>();
                Ok(json!({"count": ids.len(), "memory_ids": ids}))
            }
            "hieronymus_recall" => {
                let input: RecallInput = input!(RecallInput);
                let session = workspace.get_session(input.session_id).await?;
                if session.series_slug != input.series_slug
                    || input
                        .source_language
                        .as_ref()
                        .is_some_and(|v| v != &session.source_language)
                    || input
                        .target_language
                        .as_ref()
                        .is_some_and(|v| v != &session.target_language)
                    || input
                        .task_type
                        .as_ref()
                        .is_some_and(|v| v != &session.task_type)
                    || input.volume.as_ref().is_some_and(|v| v != &session.volume)
                    || input
                        .chapter
                        .as_ref()
                        .is_some_and(|v| v != &session.chapter)
                {
                    bail!("session context mismatch");
                }
                let context = TranslationContext::new(
                    &session.series_slug,
                    &session.source_language,
                    &session.target_language,
                )
                .with_metadata(
                    &session.language_tags,
                    &session.story_scopes,
                    &session.semantic_tags,
                    &[],
                );
                json_value(
                    RecallService::new(&self.pool)
                        .recall(input.session_id, &context, &input.query, input.limit)
                        .await?,
                )
            }
            "hieronymus_feedback" => {
                let input: FeedbackInput = input!(FeedbackInput);
                let result = workspace
                    .add_short_term(
                        input.session_id,
                        AddMemoryInput {
                            source_role: "user".into(),
                            kind: "correction".into(),
                            text: input.correction_text,
                            ..AddMemoryInput::default()
                        },
                    )
                    .await?;
                Ok(json!({"memory_id": result.memory.id}))
            }
            "hieronymus_dream" => {
                let input: DreamInput = input!(DreamInput);
                self.dream_runner
                    .as_ref()
                    .context("dream provider is not configured")?
                    .run(input.provider.as_deref(), input.wait)
                    .await
            }
            "hieronymus_concept_proposals_list" => {
                let _: NoArgs = input!(NoArgs);
                json_value(ConceptProposalStore::new(&self.pool).list_pending().await?)
            }
            "hieronymus_recall_feedback" => {
                let input: RecallFeedbackInput = input!(RecallFeedbackInput);
                FeedbackStore::new(&self.pool)
                    .record_recall_outcome(input.session_id, &input.useful, &input.miss)
                    .await?;
                Ok(json!({"recorded": true}))
            }
            _ => bail!("unknown MCP tool: {name}"),
        };
        if result.is_ok()
            && is_mutating_tool(name)
            && let Some(notifier) = &self.notifier
        {
            notifier.notify_refresh();
        }
        result
    }
}

fn is_mutating_tool(name: &str) -> bool {
    !matches!(
        name,
        "hieronymus_status"
            | "hieronymus_series_list"
            | "hieronymus_concept_list"
            | "hieronymus_concept_get"
            | "hieronymus_concept_facet_list"
            | "hieronymus_rule_crystals_list"
            | "hieronymus_rule_crystal_validate"
            | "hieronymus_termbase_contract"
            | "hieronymus_termbase_validate"
            | "hieronymus_memory_search"
            | "hieronymus_recall"
            | "hieronymus_rag_search"
            | "hieronymus_concept_proposals_list"
    )
}

fn decode<T: DeserializeOwned>(name: &str, arguments: Value) -> Result<T> {
    serde_json::from_value(arguments).map_err(|error| {
        SafeMcpError(format!("invalid payload for MCP tool `{name}`: {error}")).into()
    })
}

fn json_value(value: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).context("failed to serialize MCP result")
}

fn concept_payload(concept: &Concept) -> Value {
    json!({
        "id": concept.id,
        "canonical_name": concept.canonical_name,
        "description": concept.description,
        "status": concept.status,
        "confidence": concept.confidence,
        "scope_type": concept.scope_type,
        "scope_key": concept.scope_key,
        "semantic_tags": concept.semantic_tags,
        "merged_into_concept_id": concept.merged_into_concept_id,
    })
}

fn facet_payload(facet: &ConceptFacet) -> Value {
    let kind = if matches!(facet.facet_type.as_str(), "alias" | "former_label") {
        "name"
    } else {
        facet.facet_type.as_str()
    };
    json!({
        "id": facet.id,
        "concept_id": facet.concept_id,
        "language": facet.language,
        "facet_type": facet.facet_type,
        "kind": kind,
        "value": facet.value,
        "confidence": facet.confidence,
        "source_crystal_id": facet.source_crystal_id,
        "language_tags": facet.language_tags,
        "story_scopes": facet.story_scopes,
        "semantic_tags": facet.semantic_tags,
        "is_canonical": facet.is_canonical,
    })
}

fn crystal_payload(crystal: &Crystal) -> Value {
    json!({
        "id": crystal.id,
        "crystal_type": crystal.crystal_type,
        "text": crystal.text,
        "title": crystal.title,
        "confidence": crystal.confidence,
        "strength": crystal.strength,
        "status": crystal.status,
        "source_credibility": crystal.source_credibility,
        "rule_intent": crystal.rule_intent,
        "story_scopes": crystal.story_scopes,
        "semantic_tags": crystal.semantic_tags,
        "concept_ids": crystal.concept_ids,
    })
}

fn short_memory(input: ShortAddInput) -> AddMemoryInput {
    AddMemoryInput {
        source_role: input.source_role,
        kind: input.kind,
        text: input.text,
        source_ref: input.source_ref,
        metadata: input.metadata,
        language_tags: input.language_tags,
        story_scopes: input.story_scopes,
        semantic_tags: input.semantic_tags,
        source_credibility: input.source_credibility,
        rule_intent: input.rule_intent,
        soft_origin: (!input.soft_origin.trim().is_empty()).then_some(input.soft_origin),
    }
}

fn short_item(input: ShortItemInput) -> AddMemoryInput {
    AddMemoryInput {
        source_role: input.source_role,
        kind: input.kind,
        text: input.text,
        source_ref: input.source_ref,
        metadata: input.metadata,
        language_tags: input.language_tags,
        story_scopes: input.story_scopes,
        semantic_tags: input.semantic_tags,
        source_credibility: input.source_credibility,
        rule_intent: input.rule_intent,
        soft_origin: (!input.soft_origin.trim().is_empty()).then_some(input.soft_origin),
    }
}
