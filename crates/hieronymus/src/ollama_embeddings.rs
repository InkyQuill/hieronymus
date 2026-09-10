//! Explicit Ollama text embeddings. The pinned local tokenizer remains the
//! chunk segmentation policy; its numeric IDs never reach Ollama.
use crate::provider_http::{BlockingHttpTransport, HttpResponse, ProviderTransport};
use crate::semantic_embeddings::{EmbeddingIdentity, EmbeddingProvider};
use crate::semantic_error::SemanticError;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_DIMENSIONS: usize = 16_384;
const TIMEOUT: Duration = Duration::from_secs(30);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);

fn invalid(message: impl Into<String>) -> SemanticError {
    SemanticError::InvalidEmbedding(format!("Ollama embeddings: {}", message.into()))
}

/// Validate without contacting the service or exposing credentials in status.
pub fn validate_endpoint(url: &str, model: &str) -> Result<(), String> {
    if url.len() > 2048 || url.chars().any(char::is_control) {
        return Err(
            "Ollama base URL exceeds its length bound or contains control characters".into(),
        );
    }
    let parsed = crate::tls::parse_outbound_url(url)?;
    if parsed.path != "/" || url.contains(['?', '#', '@']) || url.chars().any(char::is_whitespace) {
        return Err("Ollama base URL must be an HTTP(S) origin without credentials, query, fragment or path".into());
    }
    if model.trim().is_empty()
        || model.len() > 256
        || model.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(
            "Ollama model must be an explicit nonempty installed model name (at most 256 bytes)"
                .into(),
        );
    }
    Ok(())
}

pub struct OllamaEmbeddingProvider {
    url: String,
    model: String,
    transport: Arc<dyn ProviderTransport>,
    identity: EmbeddingIdentity,
}
impl OllamaEmbeddingProvider {
    pub fn load(url: &str, model: &str) -> Result<Self, SemanticError> {
        Self::with_transport(url, model, Arc::new(BlockingHttpTransport::new(MAX_BYTES)))
    }
    pub fn with_transport(
        url: &str,
        model: &str,
        transport: Arc<dyn ProviderTransport>,
    ) -> Result<Self, SemanticError> {
        validate_endpoint(url, model).map_err(invalid)?;
        let mut provider = Self {
            url: url.trim_end_matches('/').into(),
            model: model.into(),
            transport,
            identity: Self::unresolved_identity(model),
        };
        let digest = provider.digest()?;
        let vector = provider.request("Hieronymus embedding identity probe")?;
        if provider.digest()? != digest {
            return Err(invalid("model changed during arming; retry configuration"));
        }
        provider.identity = Self::identity_for(model, &digest, vector.len());
        Ok(provider)
    }
    /// This sentinel is only a queue target before arming; it cannot activate.
    pub fn unresolved_identity(model: &str) -> EmbeddingIdentity {
        Self::identity_for(model, "unresolved", 1)
    }
    fn identity_for(model: &str, digest: &str, dimensions: usize) -> EmbeddingIdentity {
        EmbeddingIdentity::new(
            "ollama",
            model,
            digest,
            dimensions,
            "l2-client-f64-v1",
            format!(
                "exact-text-v1;truncate=false;max-bytes={MAX_TEXT_BYTES};segmentation={}",
                crate::semantic_tokenizer::MINILM_TOKENIZER_ID
            ),
            // Local segmentation cap only; Ollama enforces its own text context
            // limit through truncate:false. The full text is never token-truncated.
            crate::semantic_tokenizer::MAX_TOKENS,
            1,
        )
        .expect("validated Ollama identity")
    }
    fn response(&self, response: HttpResponse) -> Result<Value, SemanticError> {
        if !(200..300).contains(&response.status) {
            return Err(invalid(format!(
                "HTTP {}; check installed model, service and input length (truncate:false)",
                response.status
            )));
        }
        if response.body.len() > MAX_BYTES {
            return Err(invalid("response exceeds 1 MiB limit"));
        }
        serde_json::from_str(&response.body).map_err(|_| invalid("invalid JSON response"))
    }
    fn digest(&self) -> Result<String, SemanticError> {
        let response = self
            .transport
            .get_json(&format!("{}/api/tags", self.url), &[], DISCOVERY_TIMEOUT)
            .map_err(|e| invalid(e.to_string()))?;
        let payload = self.response(response)?;
        let models = payload["models"]
            .as_array()
            .ok_or_else(|| invalid("model discovery has no models array"))?;
        let canonical = if self.model.contains(':') {
            self.model.clone()
        } else {
            format!("{}:latest", self.model)
        };
        let matches: Vec<_> = models
            .iter()
            .filter(|entry| {
                ["name", "model"].iter().any(|key| {
                    entry[*key]
                        .as_str()
                        .is_some_and(|name| name == self.model || name == canonical)
                })
            })
            .collect();
        if matches.len() != 1 {
            return Err(invalid(
                "configured model is missing or ambiguous in /api/tags; install it explicitly",
            ));
        }
        let digest = matches[0]["digest"]
            .as_str()
            .ok_or_else(|| invalid("model discovery has no digest"))?;
        let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("model discovery digest is not SHA-256"));
        }
        Ok(hex.to_ascii_lowercase())
    }
    fn request(&self, text: &str) -> Result<Vec<f32>, SemanticError> {
        if text.is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(invalid(
                "input must contain 1..65536 UTF-8 bytes; no truncation is permitted",
            ));
        }
        let response = self
            .transport
            .post_json(
                &format!("{}/api/embed", self.url),
                &[],
                &json!({"model":self.model,"input":text,"truncate":false}),
                TIMEOUT,
            )
            .map_err(|e| invalid(e.to_string()))?;
        let payload = self.response(response)?;
        let embeddings = payload["embeddings"]
            .as_array()
            .ok_or_else(|| invalid("response has no embeddings array"))?;
        if embeddings.len() != 1 {
            return Err(invalid("expected exactly one embedding"));
        }
        let values = embeddings[0]
            .as_array()
            .ok_or_else(|| invalid("embedding is not an array"))?;
        if values.is_empty() || values.len() > MAX_DIMENSIONS {
            return Err(invalid("embedding dimensions must be 1..16384"));
        }
        let mut vector = Vec::with_capacity(values.len());
        for value in values {
            let number = value
                .as_f64()
                .ok_or_else(|| invalid("embedding contains a non-number"))?;
            let component = number as f32;
            if !number.is_finite() || !component.is_finite() {
                return Err(invalid("embedding contains a non-finite component"));
            }
            vector.push(component);
        }
        let norm = vector
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
        if norm <= f64::EPSILON || !norm.is_finite() {
            return Err(invalid("embedding has an invalid norm"));
        }
        for v in &mut vector {
            *v = (f64::from(*v) / norm) as f32;
        }
        Ok(vector)
    }
    fn embed_text(&self, text: &str) -> Result<Vec<f32>, SemanticError> {
        self.verify_identity()?;
        let vector = self.request(text)?;
        self.verify_identity()?;
        if vector.len() != self.identity.dimensions() {
            return Err(invalid(
                "embedding dimensions changed; reconfigure and rebuild",
            ));
        }
        Ok(vector)
    }
}
impl EmbeddingProvider for OllamaEmbeddingProvider {
    fn identity(&self) -> &EmbeddingIdentity {
        &self.identity
    }
    fn verify_identity(&self) -> Result<(), SemanticError> {
        if self.digest()? != self.identity.revision() {
            return Err(invalid(
                "installed model digest changed; reconfigure and rebuild",
            ));
        }
        Ok(())
    }
    fn embed_document(&mut self, _: &[u32]) -> Result<Vec<f32>, SemanticError> {
        Err(invalid("exact document text is required"))
    }
    fn embed_query(&mut self, _: &[u32]) -> Result<Vec<f32>, SemanticError> {
        Err(invalid("exact query text is required"))
    }
    fn embed_document_text(&mut self, text: &str, _: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.embed_text(text)
    }
    fn embed_query_text(&mut self, text: &str, _: &[u32]) -> Result<Vec<f32>, SemanticError> {
        self.embed_text(text)
    }
}
