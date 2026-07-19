//! LLM provider profiles, bounded model discovery, and dreaming adapters.

mod anthropic;
mod cache;
mod catalog;
mod google;
mod ollama;
mod openai;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::domain::{ShortTermMemory, TranslationContext};

pub use anthropic::AnthropicProvider;
pub use cache::{ModelCache, ModelCacheEntry};
pub use catalog::{CredentialSource, ProviderCatalog, ProviderProfile};
pub use google::GoogleProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAiProvider;

#[derive(Debug, Clone, Copy)]
enum Dialect {
    OpenAi,
    Anthropic,
    Google,
    Ollama,
}

#[derive(Clone)]
struct HttpProvider {
    client: reqwest::Client,
    profile: ProviderProfile,
    dialect: Dialect,
}

impl HttpProvider {
    fn new(_client: reqwest::Client, profile: ProviderProfile, dialect: Dialect) -> Result<Self> {
        profile.validate()?;
        // Custom API-key headers survive reqwest's cross-host redirect cleanup.
        // Rebuild the transport with redirects disabled so caller policy cannot leak them.
        let client = reqwest::Client::builder()
            .connect_timeout(profile.timeout().min(std::time::Duration::from_secs(10)))
            .read_timeout(profile.timeout().min(std::time::Duration::from_secs(30)))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(classify_reqwest)?;
        Ok(Self {
            client,
            profile,
            dialect,
        })
    }
    fn base_url(&self) -> &str {
        self.profile.base_url().unwrap_or(match self.dialect {
            Dialect::OpenAi => "https://api.openai.com/v1",
            Dialect::Anthropic => "https://api.anthropic.com",
            Dialect::Google => "https://generativelanguage.googleapis.com",
            Dialect::Ollama => "http://127.0.0.1:11434",
        })
    }
    fn secret(&self) -> Result<Option<secrecy::SecretString>> {
        if self.profile.provider() == "ollama"
            && matches!(self.profile.credential(), CredentialSource::None)
        {
            Ok(matches!(self.dialect, Dialect::OpenAi)
                .then(|| secrecy::SecretString::from("ollama".to_owned())))
        } else {
            self.profile.credential().resolve().map(Some)
        }
    }
    async fn generate(&self, prompt: &str, check: bool) -> Result<String> {
        let secret = self.secret()?;
        let base = self.base_url().trim_end_matches('/');
        let model = self.profile.model();
        let (url, payload) = match self.dialect {
            Dialect::OpenAi => (
                format!("{base}/chat/completions"),
                serde_json::json!({"model":model,"messages":[{"role":"user","content":prompt}], if check {"max_tokens"} else {"temperature"}: if check { serde_json::json!(1) } else { serde_json::json!(0.1) }, "response_format":{"type":"json_object"}}),
            ),
            Dialect::Anthropic => (
                format!("{base}/v1/messages"),
                serde_json::json!({"model":model,"max_tokens":if check {1} else {2000},"temperature":0.1,"messages":[{"role":"user","content":prompt}]}),
            ),
            Dialect::Google => (
                format!("{base}/v1beta/models/{model}:generateContent"),
                serde_json::json!({"contents":[{"role":"user","parts":[{"text":prompt}]}],"generationConfig":{"temperature":0.1,"responseMimeType":"application/json"}}),
            ),
            Dialect::Ollama => (
                format!("{base}/api/chat"),
                serde_json::json!({"model":model,"messages":[{"role":"user","content":prompt}],"stream":false,"format":"json","options":{"temperature":0.1}}),
            ),
        };
        let mut request = self
            .client
            .post(url)
            .json(&payload)
            .timeout(self.profile.timeout());
        if let Some(secret) = secret.as_ref() {
            request = match self.dialect {
                Dialect::OpenAi => request.bearer_auth(secret.expose_secret()),
                Dialect::Anthropic => request
                    .header("x-api-key", secret.expose_secret())
                    .header("anthropic-version", "2023-06-01"),
                Dialect::Google => request.header("x-goog-api-key", secret.expose_secret()),
                Dialect::Ollama => request,
            };
        }
        let response = request.send().await.map_err(classify_reqwest)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ProviderError::Http {
                status: status.as_u16(),
            });
        }
        let value = parse_json(&bounded_body(response).await?)?;
        extract_text(self.dialect, &value).map(str::to_owned)
    }
    async fn list_models(&self) -> Result<Vec<String>> {
        if matches!(self.dialect, Dialect::Anthropic) {
            return Ok(vec!["claude-sonnet-4-5".into(), "claude-haiku-4-5".into()]);
        }
        let secret = self.secret()?;
        let base = self.base_url().trim_end_matches('/');
        let url = match self.dialect {
            Dialect::OpenAi => format!("{base}/models"),
            Dialect::Google => format!("{base}/v1beta/models"),
            Dialect::Ollama => format!("{base}/api/tags"),
            Dialect::Anthropic => unreachable!(),
        };
        let mut request = self.client.get(url).timeout(self.profile.timeout());
        if let Some(secret) = secret.as_ref() {
            request = match self.dialect {
                Dialect::OpenAi => request.bearer_auth(secret.expose_secret()),
                Dialect::Google => request.header("x-goog-api-key", secret.expose_secret()),
                _ => request,
            };
        }
        let response = request.send().await.map_err(classify_reqwest)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ProviderError::Http {
                status: status.as_u16(),
            });
        }
        let value = parse_json(&bounded_body(response).await?)?;
        let array = match self.dialect {
            Dialect::OpenAi => value.get("data"),
            Dialect::Google | Dialect::Ollama => value.get("models"),
            Dialect::Anthropic => None,
        }
        .and_then(serde_json::Value::as_array)
        .ok_or(ProviderError::MissingOutput)?;
        let mut models: Vec<String> = array
            .iter()
            .filter_map(|item| {
                match self.dialect {
                    Dialect::OpenAi => item.get("id"),
                    Dialect::Google => item.get("name"),
                    Dialect::Ollama => item.get("model"),
                    Dialect::Anthropic => None,
                }
                .and_then(serde_json::Value::as_str)
            })
            .map(|name| name.strip_prefix("models/").unwrap_or(name).to_owned())
            .collect();
        models.sort();
        models.dedup();
        if models.is_empty() {
            return Err(ProviderError::MissingOutput);
        }
        Ok(models)
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

/// Typed provider failures. Display strings deliberately contain no response bodies or keys.
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
    #[error("provider profile was not found: {0}")]
    MissingProfile(String),
    #[error("unsupported provider type: {0}")]
    Unsupported(String),
    #[error("provider file operation failed")]
    Io(#[source] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ProviderError>;

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
        pass: &str,
        context: &TranslationContext,
        memories: &[ShortTermMemory],
    ) -> Result<serde_json::Value>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionCheck {
    pub ok: bool,
    pub detail: String,
}

pub(crate) const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn dream_prompt(
    context: &TranslationContext,
    memories: &[ShortTermMemory],
) -> Result<String> {
    serde_json::to_string(&serde_json::json!({
        "instruction": "Return JSON with crystals and concepts.",
        "context": context,
        "memories": memories,
    }))
    .map_err(|_| ProviderError::Config("dream input could not be encoded".into()))
}

pub(crate) async fn bounded_body(response: reqwest::Response) -> Result<Vec<u8>> {
    use futures::StreamExt;
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(classify_reqwest)?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ProviderError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(crate) fn classify_reqwest(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Transport
    }
}

pub(crate) fn parse_json(bytes: &[u8]) -> Result<serde_json::Value> {
    serde_json::from_slice(bytes).map_err(|_| ProviderError::MalformedJson)
}

pub(crate) fn parse_output(text: &str) -> Result<DreamOutput> {
    serde_json::from_str(text).map_err(|_| ProviderError::MalformedJson)
}

pub struct ProviderRegistry {
    client: reqwest::Client,
    cache: ModelCache,
}

impl ProviderRegistry {
    #[must_use]
    pub fn new(client: reqwest::Client, cache: ModelCache) -> Self {
        Self { client, cache }
    }
    pub fn with_default_client(cache: ModelCache) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(classify_reqwest)?;
        Ok(Self::new(client, cache))
    }
    pub fn resolve(&self, catalog: &ProviderCatalog, name: &str) -> Result<Box<dyn DreamProvider>> {
        let profile = catalog
            .get(name)
            .ok_or_else(|| ProviderError::MissingProfile(name.into()))?
            .clone();
        match profile.provider() {
            "openai" => Ok(Box::new(OpenAiProvider::new(self.client.clone(), profile)?)),
            "anthropic" => Ok(Box::new(AnthropicProvider::new(
                self.client.clone(),
                profile,
            )?)),
            "google" => Ok(Box::new(GoogleProvider::new(self.client.clone(), profile)?)),
            "ollama" => Ok(Box::new(OllamaProvider::new(self.client.clone(), profile)?)),
            other => Err(ProviderError::Unsupported(other.into())),
        }
    }
    pub async fn check(&self, profile: &ProviderProfile) -> Result<ConnectionCheck> {
        let provider = self.http(profile.clone())?;
        provider.generate("Reply with ok.", true).await?;
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
        let identity = profile_cache_identity(profile);
        if let Some(models) = self.cache.get(&identity, now) {
            return Ok(models);
        }
        let models = self.http(profile.clone())?.list_models().await?;
        self.cache.insert(&identity, models.clone(), now)?;
        Ok(models)
    }
    fn http(&self, profile: ProviderProfile) -> Result<HttpProvider> {
        let dialect = dialect_for_profile(&profile)?;
        HttpProvider::new(self.client.clone(), profile, dialect)
    }
}

fn dialect_for_profile(profile: &ProviderProfile) -> Result<Dialect> {
    match profile.provider() {
        "openai" => Ok(Dialect::OpenAi),
        "anthropic" => Ok(Dialect::Anthropic),
        "google" => Ok(Dialect::Google),
        "ollama"
            if profile
                .base_url()
                .is_some_and(|url| url.trim_end_matches('/').ends_with("/v1")) =>
        {
            Ok(Dialect::OpenAi)
        }
        "ollama" => Ok(Dialect::Ollama),
        other => Err(ProviderError::Unsupported(other.into())),
    }
}

fn profile_cache_identity(profile: &ProviderProfile) -> String {
    format!(
        "{}|{}|{}",
        profile.id(),
        profile.provider(),
        profile.base_url().unwrap_or_default().trim_end_matches('/')
    )
}
