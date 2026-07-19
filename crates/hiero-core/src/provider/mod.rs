//! LLM provider profiles, bounded model discovery, and dreaming adapters.

mod anthropic;
mod cache;
mod catalog;
mod google;
mod ollama;
mod openai;

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use futures::StreamExt;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::domain::{ShortTermMemory, TranslationContext};

pub use anthropic::AnthropicProvider;
pub use cache::{ModelCache, ModelCacheEntry};
pub use catalog::{CredentialSource, ProviderCatalog, ProviderDefaults, ProviderProfile};
pub use google::GoogleProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAiProvider;

pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_DISCOVERY_PAGES: usize = 8;
const MAX_DISCOVERY_MODELS: usize = 1_000;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider configuration is invalid: {0}")]
    Config(String),
    #[error("provider credential is unavailable: {0}")]
    Credential(String),
    #[error("provider endpoint is not permitted: {0}")]
    UnsafeEndpoint(String),
    #[error("provider request timed out")]
    Timeout,
    #[error("provider transport failed")]
    Transport,
    #[error("provider returned HTTP {status}")]
    Http { status: u16 },
    #[error("provider response exceeded {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("provider returned malformed JSON")]
    MalformedJson,
    #[error("provider response did not contain generated text")]
    MissingOutput,
    #[error("provider discovery pagination loop")]
    PaginationLoop,
    #[error("provider discovery exceeded its page or model cap")]
    PaginationLimit,
    #[error("provider profile was not found: {0}")]
    MissingProfile(String),
    #[error("unsupported provider type: {0}")]
    Unsupported(String),
    #[error("provider file operation failed")]
    Io(#[source] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ProviderError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassName {
    Concepts,
    TerminologyCandidates,
    RuleCrystals,
    KnowledgeCrystals,
    Relations,
    Reinforcement,
    CoverageAudit,
}

impl PassName {
    pub const ALL: [Self; 7] = [
        Self::Concepts,
        Self::TerminologyCandidates,
        Self::RuleCrystals,
        Self::KnowledgeCrystals,
        Self::Relations,
        Self::Reinforcement,
        Self::CoverageAudit,
    ];
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Concepts => "concepts",
            Self::TerminologyCandidates => "terminology_candidates",
            Self::RuleCrystals => "rule_crystals",
            Self::KnowledgeCrystals => "knowledge_crystals",
            Self::Relations => "relations",
            Self::Reinforcement => "reinforcement",
            Self::CoverageAudit => "coverage_audit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamOutput {
    pub crystals: Vec<CandidateCrystal>,
    pub concepts: Vec<ConceptCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCrystal {
    pub crystal_type: String,
    pub title: String,
    pub text: String,
    pub source_credibility: String,
    pub rule_intent: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptCandidate {
    pub canonical_name: String,
    pub facets: Vec<(String, String, String)>,
}

#[async_trait]
pub trait DreamProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn crystallize(
        &self,
        context: &TranslationContext,
        memories: &[ShortTermMemory],
    ) -> Result<DreamOutput>;
    async fn run_pass(
        &self,
        pass: PassName,
        context: &TranslationContext,
        memories: &[ShortTermMemory],
    ) -> Result<serde_json::Value>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionCheck {
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Clone)]
pub struct ProviderRequest {
    method: HttpMethod,
    url: Url,
    headers: BTreeMap<String, String>,
    payload: Option<serde_json::Value>,
    timeout: Duration,
    response_limit: usize,
}

impl std::fmt::Debug for ProviderRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &"[REDACTED]")
            .field("payload", &"[REDACTED]")
            .field("timeout", &self.timeout)
            .field("response_limit", &self.response_limit)
            .finish()
    }
}

impl ProviderRequest {
    pub fn method(&self) -> HttpMethod {
        self.method
    }
    pub fn url(&self) -> &Url {
        &self.url
    }
    pub fn headers(&self) -> &BTreeMap<String, String> {
        &self.headers
    }
    pub fn payload(&self) -> Option<&serde_json::Value> {
        self.payload.as_ref()
    }
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
    pub fn response_limit(&self) -> usize {
        self.response_limit
    }
}

#[derive(Clone)]
pub struct ProviderResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for ProviderResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderResponse")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .finish()
    }
}

#[async_trait]
pub trait ProviderTransport: Send + Sync {
    async fn execute(&self, request: ProviderRequest) -> Result<ProviderResponse>;
}

pub trait ProviderTransportFactory: Send + Sync {
    fn create(&self, profile: &ProviderProfile) -> Result<Arc<dyn ProviderTransport>>;
}

#[async_trait]
pub trait CredentialResolver: Send + Sync {
    async fn resolve(&self, profile: &ProviderProfile) -> Result<Option<SecretString>>;
}

#[derive(Debug, Default)]
pub struct DefaultCredentialResolver;

#[async_trait]
impl CredentialResolver for DefaultCredentialResolver {
    async fn resolve(&self, profile: &ProviderProfile) -> Result<Option<SecretString>> {
        match profile.credential() {
            CredentialSource::None if profile.provider_type() == "ollama" => Ok(None),
            _ => Ok(Some(profile.resolve_credential_async().await?)),
        }
    }
}

async fn resolve_operation_credential(
    resolver: &dyn CredentialResolver,
    profile: &ProviderProfile,
) -> Result<Option<SecretString>> {
    let credential = resolver.resolve(profile).await?;
    if credential.is_none() && profile.provider_type() != "ollama" {
        return Err(ProviderError::Credential(
            "provider credential resolver returned no credential".into(),
        ));
    }
    Ok(credential)
}

#[derive(Clone, Default)]
pub struct ReqwestTransportOptions {
    pub trusted_proxy: Option<String>,
    pub custom_ca_pem: Vec<Vec<u8>>,
}

impl std::fmt::Debug for ReqwestTransportOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let custom_ca_bytes = self.custom_ca_pem.iter().map(Vec::len).sum::<usize>();
        formatter
            .debug_struct("ReqwestTransportOptions")
            .field("trusted_proxy_present", &self.trusted_proxy.is_some())
            .field("custom_ca_count", &self.custom_ca_pem.len())
            .field("custom_ca_bytes", &custom_ca_bytes)
            .finish()
    }
}

pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn for_profile(
        profile: &ProviderProfile,
        options: &ReqwestTransportOptions,
    ) -> Result<Self> {
        profile.validate()?;
        let timeout = profile.timeout()?;
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .read_timeout(timeout.min(Duration::from_secs(30)))
            .redirect(reqwest::redirect::Policy::none());
        if profile.provider_type() == "ollama" {
            if options.trusted_proxy.is_some() {
                return Err(ProviderError::Config(
                    "native Ollama does not permit a trusted proxy".into(),
                ));
            }
        } else if let Some(proxy) = &options.trusted_proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|_| ProviderError::Config("trusted proxy URL is invalid".into()))?,
            );
        }
        for pem in &options.custom_ca_pem {
            let certificate = reqwest::Certificate::from_pem(pem)
                .map_err(|_| ProviderError::Config("custom CA certificate is invalid".into()))?;
            builder = builder.add_root_certificate(certificate);
        }
        Ok(Self {
            client: builder.build().map_err(classify_reqwest)?,
        })
    }
}

#[async_trait]
impl ProviderTransport for ReqwestTransport {
    async fn execute(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        let mut builder = match request.method {
            HttpMethod::Get => self.client.get(request.url),
            HttpMethod::Post => self.client.post(request.url),
        }
        .timeout(request.timeout);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if let Some(payload) = request.payload {
            builder = builder.json(&payload);
        }
        let response = builder.send().await.map_err(classify_reqwest)?;
        let status = response.status().as_u16();
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(classify_reqwest)?;
            if body.len().saturating_add(chunk.len()) > request.response_limit {
                return Err(ProviderError::ResponseTooLarge {
                    limit: request.response_limit,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(ProviderResponse { status, body })
    }
}

#[derive(Clone)]
struct ReqwestTransportFactory {
    options: ReqwestTransportOptions,
}
impl ProviderTransportFactory for ReqwestTransportFactory {
    fn create(&self, profile: &ProviderProfile) -> Result<Arc<dyn ProviderTransport>> {
        Ok(Arc::new(ReqwestTransport::for_profile(
            profile,
            &self.options,
        )?))
    }
}

#[derive(Clone)]
struct StaticTransportFactory {
    transport: Arc<dyn ProviderTransport>,
}
impl ProviderTransportFactory for StaticTransportFactory {
    fn create(&self, _profile: &ProviderProfile) -> Result<Arc<dyn ProviderTransport>> {
        Ok(self.transport.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    OpenAi,
    Anthropic,
    Google,
    Ollama,
}

#[derive(Clone)]
struct HttpProvider {
    transport: Arc<dyn ProviderTransport>,
    credentials: Arc<dyn CredentialResolver>,
    profile: ProviderProfile,
    model: String,
    dialect: Dialect,
}

impl HttpProvider {
    fn new(
        transport: Arc<dyn ProviderTransport>,
        profile: ProviderProfile,
        model: impl Into<String>,
        dialect: Dialect,
    ) -> Result<Self> {
        Self::new_with_credential_resolver(
            transport,
            Arc::new(DefaultCredentialResolver),
            profile,
            model,
            dialect,
        )
    }

    fn new_with_credential_resolver(
        transport: Arc<dyn ProviderTransport>,
        credentials: Arc<dyn CredentialResolver>,
        profile: ProviderProfile,
        model: impl Into<String>,
        dialect: Dialect,
    ) -> Result<Self> {
        profile.validate()?;
        let model = normalize_model(model.into())?;
        let model = if dialect == Dialect::Google {
            normalize_google_model(&model)?
        } else {
            model
        };
        Ok(Self {
            transport,
            credentials,
            profile,
            model,
            dialect,
        })
    }

    async fn generate(&self, prompt: &str, health: bool) -> Result<String> {
        let credential =
            resolve_operation_credential(self.credentials.as_ref(), &self.profile).await?;
        let request = self.generation_request(prompt, health, credential.as_ref())?;
        let value = self.execute_json(request).await?;
        extract_text(self.dialect, &value).map(str::to_owned)
    }

    async fn health(&self) -> Result<()> {
        let credential =
            resolve_operation_credential(self.credentials.as_ref(), &self.profile).await?;
        let request = self.generation_request("Reply with ok.", true, credential.as_ref())?;
        let response = self.execute_response(request).await?;
        ensure_success(response.status)
    }

    fn generation_request(
        &self,
        prompt: &str,
        health: bool,
        credential: Option<&SecretString>,
    ) -> Result<ProviderRequest> {
        let base = self.profile.base_url().trim_end_matches('/');
        let mut headers = self.auth_headers(credential)?;
        let (url, payload) = match self.dialect {
            Dialect::OpenAi => {
                let mut payload = serde_json::json!({"model":self.model,"messages":[{"role":"user","content":prompt}]});
                if health {
                    payload["max_tokens"] = 1.into();
                } else {
                    payload["temperature"] = 0.1.into();
                    payload["response_format"] = serde_json::json!({"type":"json_object"});
                }
                (parse_url(&format!("{base}/chat/completions"))?, payload)
            }
            Dialect::Anthropic => {
                headers.insert("anthropic-version".into(), "2023-06-01".into());
                (
                    parse_url(&format!("{base}/v1/messages"))?,
                    serde_json::json!({"model":self.model,"max_tokens":if health {1} else {2000},"temperature":0.1,"messages":[{"role":"user","content":prompt}]}),
                )
            }
            Dialect::Google => {
                let mut generation =
                    serde_json::json!({"temperature":0.1,"responseMimeType":"application/json"});
                if health {
                    generation["maxOutputTokens"] = 1.into();
                }
                (
                    google_generate_url(base, &self.model)?,
                    serde_json::json!({"contents":[{"role":"user","parts":[{"text":prompt}]}],"generationConfig":generation}),
                )
            }
            Dialect::Ollama => {
                let mut options = serde_json::json!({"temperature":0.1});
                if health {
                    options["num_predict"] = 1.into();
                }
                (
                    parse_url(&format!("{base}/api/chat"))?,
                    serde_json::json!({"model":self.model,"messages":[{"role":"user","content":prompt}],"stream":false,"format":"json","options":options}),
                )
            }
        };
        Ok(ProviderRequest {
            method: HttpMethod::Post,
            url,
            headers,
            payload: Some(payload),
            timeout: self.profile.timeout()?,
            response_limit: MAX_RESPONSE_BYTES,
        })
    }

    fn auth_headers(&self, credential: Option<&SecretString>) -> Result<BTreeMap<String, String>> {
        let mut headers = BTreeMap::new();
        if let Some(secret) = credential {
            let value = secret.expose_secret();
            match self.dialect {
                Dialect::OpenAi | Dialect::Ollama => {
                    headers.insert("authorization".into(), format!("Bearer {value}"));
                }
                Dialect::Anthropic => {
                    headers.insert("x-api-key".into(), value.to_owned());
                }
                Dialect::Google => {
                    headers.insert("x-goog-api-key".into(), value.to_owned());
                }
            }
        }
        Ok(headers)
    }

    async fn list_models(&self, credential: Option<&SecretString>) -> Result<Vec<String>> {
        let mut url = match self.dialect {
            Dialect::OpenAi | Dialect::Anthropic => parse_url(&format!(
                "{}/v1/models",
                self.profile
                    .base_url()
                    .trim_end_matches('/')
                    .trim_end_matches("/v1")
            ))?,
            Dialect::Google => parse_url(&format!(
                "{}/v1beta/models",
                self.profile.base_url().trim_end_matches('/')
            ))?,
            Dialect::Ollama => parse_url(&format!(
                "{}/api/tags",
                self.profile.base_url().trim_end_matches('/')
            ))?,
        };
        let mut headers = self.auth_headers(credential)?;
        if matches!(self.dialect, Dialect::Anthropic) {
            headers.insert("anthropic-version".into(), "2023-06-01".into());
        }
        let mut seen = HashSet::new();
        let mut models = Vec::new();
        for _ in 0..MAX_DISCOVERY_PAGES {
            if !seen.insert(url.to_string()) {
                return Err(ProviderError::PaginationLoop);
            }
            let value = self
                .execute_json(ProviderRequest {
                    method: HttpMethod::Get,
                    url: url.clone(),
                    headers: headers.clone(),
                    payload: None,
                    timeout: self.profile.timeout()?,
                    response_limit: MAX_RESPONSE_BYTES,
                })
                .await?;
            let (page, next) = discovery_page(self.dialect, &value, &url)?;
            models.extend(page);
            models.sort();
            models.dedup();
            if models.len() > MAX_DISCOVERY_MODELS {
                return Err(ProviderError::PaginationLimit);
            }
            let Some(next) = next else {
                if models.is_empty() {
                    return Err(ProviderError::MissingOutput);
                }
                return Ok(models);
            };
            url = next;
        }
        Err(ProviderError::PaginationLimit)
    }

    async fn execute_json(&self, request: ProviderRequest) -> Result<serde_json::Value> {
        let response = self.execute_response(request).await?;
        ensure_success(response.status)?;
        serde_json::from_slice(&response.body).map_err(|_| ProviderError::MalformedJson)
    }

    async fn execute_response(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        let response_limit = request.response_limit();
        if response_limit == 0 {
            return Err(ProviderError::Config(
                "provider response limit must be positive".into(),
            ));
        }
        let response = self.transport.execute(request).await?;
        if response.body.len() > response_limit {
            return Err(ProviderError::ResponseTooLarge {
                limit: response_limit,
            });
        }
        Ok(response)
    }
}

fn discovery_page(
    dialect: Dialect,
    value: &serde_json::Value,
    current: &Url,
) -> Result<(Vec<String>, Option<Url>)> {
    let mut models = Vec::new();
    let mut next = None;
    match dialect {
        Dialect::OpenAi | Dialect::Anthropic => {
            let data = value
                .get("data")
                .and_then(serde_json::Value::as_array)
                .ok_or(ProviderError::MissingOutput)?;
            models.extend(
                data.iter()
                    .filter_map(|item| item.get("id").and_then(serde_json::Value::as_str))
                    .map(str::to_owned),
            );
            if value
                .get("has_more")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                let last = value
                    .get("last_id")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        data.last()
                            .and_then(|item| item.get("id"))
                            .and_then(serde_json::Value::as_str)
                    })
                    .ok_or(ProviderError::PaginationLoop)?;
                let mut next_url = current.clone();
                let cursor_name = if dialect == Dialect::Anthropic {
                    "after_id"
                } else {
                    "after"
                };
                next_url
                    .query_pairs_mut()
                    .clear()
                    .append_pair(cursor_name, last);
                next = Some(next_url);
            }
        }
        Dialect::Google => {
            let data = value
                .get("models")
                .and_then(serde_json::Value::as_array)
                .ok_or(ProviderError::MissingOutput)?;
            models.extend(
                data.iter()
                    .filter(|item| {
                        item.get("supportedGenerationMethods")
                            .and_then(serde_json::Value::as_array)
                            .is_some_and(|methods| {
                                methods
                                    .iter()
                                    .any(|method| method.as_str() == Some("generateContent"))
                            })
                    })
                    .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str))
                    .filter_map(|name| name.strip_prefix("models/"))
                    .map(str::to_owned),
            );
            if let Some(token) = value
                .get("nextPageToken")
                .and_then(serde_json::Value::as_str)
                .filter(|token| !token.is_empty())
            {
                let mut next_url = current.clone();
                next_url
                    .query_pairs_mut()
                    .clear()
                    .append_pair("pageToken", token);
                next = Some(next_url);
            }
        }
        Dialect::Ollama => {
            let data = value
                .get("models")
                .and_then(serde_json::Value::as_array)
                .ok_or(ProviderError::MissingOutput)?;
            models.extend(
                data.iter()
                    .filter_map(|item| item.get("model").and_then(serde_json::Value::as_str))
                    .map(str::to_owned),
            );
        }
    }
    Ok((models, next))
}

fn google_generate_url(base: &str, model: &str) -> Result<Url> {
    let model = normalize_google_model(model)?;
    let mut url = parse_url(&format!("{}/v1beta/models/", base.trim_end_matches('/')))?;
    url.path_segments_mut()
        .map_err(|()| ProviderError::Config("Google base URL cannot be a base".into()))?
        .push(&model);
    let path = format!("{}:generateContent", url.path());
    url.set_path(&path);
    Ok(url)
}

fn normalize_google_model(model: &str) -> Result<String> {
    let model = model.strip_prefix("models/").unwrap_or(model);
    if model.is_empty() || model.contains(['/', '?', '#']) {
        return Err(ProviderError::Config(
            "Google model must be exactly one path segment".into(),
        ));
    }
    Ok(model.to_owned())
}

fn parse_url(value: &str) -> Result<Url> {
    Url::parse(value).map_err(|_| ProviderError::Config("provider request URL is invalid".into()))
}
fn normalize_model(model: String) -> Result<String> {
    let model = model.trim().to_owned();
    if model.is_empty() {
        Err(ProviderError::Config(
            "provider model must not be empty".into(),
        ))
    } else {
        Ok(model)
    }
}
fn ensure_success(status: u16) -> Result<()> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(ProviderError::Http { status })
    }
}
fn extract_text(dialect: Dialect, value: &serde_json::Value) -> Result<&str> {
    let text = match dialect {
        Dialect::OpenAi => value.pointer("/choices/0/message/content"),
        Dialect::Anthropic => value.pointer("/content/0/text"),
        Dialect::Google => value.pointer("/candidates/0/content/parts/0/text"),
        Dialect::Ollama => value.pointer("/message/content"),
    };
    text.and_then(serde_json::Value::as_str)
        .ok_or(ProviderError::MissingOutput)
}
fn classify_reqwest(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Transport
    }
}

pub(crate) fn dream_prompt(
    context: &TranslationContext,
    memories: &[ShortTermMemory],
) -> Result<String> {
    serde_json::to_string(&serde_json::json!({"instruction":"Return JSON with crystals and concepts.","context":context,"memories":memories})).map_err(|_| ProviderError::Config("dream input could not be encoded".into()))
}
pub(crate) fn parse_output(text: &str) -> Result<DreamOutput> {
    serde_json::from_str(text).map_err(|_| ProviderError::MalformedJson)
}

pub struct ProviderRegistry {
    factory: Arc<dyn ProviderTransportFactory>,
    cache: ModelCache,
    credentials: Arc<dyn CredentialResolver>,
}

impl ProviderRegistry {
    pub fn with_transport(transport: Arc<dyn ProviderTransport>, cache: ModelCache) -> Self {
        Self {
            factory: Arc::new(StaticTransportFactory { transport }),
            cache,
            credentials: Arc::new(DefaultCredentialResolver),
        }
    }
    pub fn with_transport_and_credential_resolver(
        transport: Arc<dyn ProviderTransport>,
        cache: ModelCache,
        credentials: Arc<dyn CredentialResolver>,
    ) -> Self {
        Self {
            factory: Arc::new(StaticTransportFactory { transport }),
            cache,
            credentials,
        }
    }
    pub fn production(cache: ModelCache, options: ReqwestTransportOptions) -> Self {
        Self {
            factory: Arc::new(ReqwestTransportFactory { options }),
            cache,
            credentials: Arc::new(DefaultCredentialResolver),
        }
    }
    pub fn resolve(
        &self,
        catalog: &ProviderCatalog,
        name: &str,
        model: &str,
    ) -> Result<Box<dyn DreamProvider>> {
        let profile = catalog
            .get(name)
            .ok_or_else(|| ProviderError::MissingProfile(name.into()))?
            .clone();
        let transport = self.factory.create(&profile)?;
        match profile.provider_type() {
            "openai" => Ok(Box::new(OpenAiProvider::with_credential_resolver(
                transport,
                self.credentials.clone(),
                profile,
                model,
            )?)),
            "anthropic" => Ok(Box::new(AnthropicProvider::with_credential_resolver(
                transport,
                self.credentials.clone(),
                profile,
                model,
            )?)),
            "google" => Ok(Box::new(GoogleProvider::with_credential_resolver(
                transport,
                self.credentials.clone(),
                profile,
                model,
            )?)),
            "ollama" => Ok(Box::new(OllamaProvider::with_credential_resolver(
                transport,
                self.credentials.clone(),
                profile,
                model,
            )?)),
            other => Err(ProviderError::Unsupported(other.into())),
        }
    }
    pub async fn check(&self, profile: &ProviderProfile, model: &str) -> Result<ConnectionCheck> {
        let provider = self.http(profile.clone(), model)?;
        provider.health().await?;
        Ok(ConnectionCheck {
            ok: true,
            detail: "connection succeeded".into(),
        })
    }
    pub async fn suggest_models(
        &self,
        profile: &ProviderProfile,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<String>> {
        let credential = resolve_operation_credential(self.credentials.as_ref(), profile).await?;
        let identity = profile_cache_identity(profile, credential.as_ref());
        if let Some(models) = self.cache.get(&identity, now) {
            return Ok(models);
        }
        let models = self
            .http(profile.clone(), "discovery")?
            .list_models(credential.as_ref())
            .await?;
        self.cache.insert(&identity, models.clone(), now)?;
        Ok(models)
    }
    fn http(&self, profile: ProviderProfile, model: &str) -> Result<HttpProvider> {
        let transport = self.factory.create(&profile)?;
        let dialect = match profile.provider_type() {
            "openai" => Dialect::OpenAi,
            "anthropic" => Dialect::Anthropic,
            "google" => Dialect::Google,
            "ollama" => Dialect::Ollama,
            other => return Err(ProviderError::Unsupported(other.into())),
        };
        HttpProvider::new_with_credential_resolver(
            transport,
            self.credentials.clone(),
            profile,
            model,
            dialect,
        )
    }
}

fn profile_cache_identity(profile: &ProviderProfile, credential: Option<&SecretString>) -> String {
    let secret_hash = Sha256::digest(
        credential
            .map(ExposeSecret::expose_secret)
            .unwrap_or_default()
            .as_bytes(),
    );
    let mut digest = Sha256::new();
    digest.update(profile.id().as_bytes());
    digest.update([0]);
    digest.update(profile.provider_type().as_bytes());
    digest.update([0]);
    digest.update(profile.base_url().trim_end_matches('/').as_bytes());
    digest.update([0]);
    digest.update(secret_hash);
    format!("{:x}", digest.finalize())
}
