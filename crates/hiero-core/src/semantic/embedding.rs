use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;

use super::{Result, SemanticError};

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn provider(&self) -> &str;
    fn model(&self) -> &str;
    fn dimensions(&self) -> usize;
    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}

/// Deterministic test provider. It is deliberately named and never selected by production config.
pub struct FakeEmbeddingProvider {
    dimensions: usize,
}

impl FakeEmbeddingProvider {
    #[must_use]
    pub const fn new(dimensions: usize) -> Self {
        Self { dimensions }
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut vector = vec![0.0; self.dimensions];
        for (index, byte) in text.bytes().enumerate() {
            if self.dimensions != 0 {
                vector[index % self.dimensions] += f32::from(byte) / 255.0;
            }
        }
        vector
    }
}

#[async_trait]
impl EmbeddingProvider for FakeEmbeddingProvider {
    fn provider(&self) -> &str {
        "fake"
    }
    fn model(&self) -> &str {
        "deterministic-test-embedding"
    }
    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if self.dimensions == 0 {
            return Err(SemanticError::InvalidVector {
                reason: "embedding dimensions must be non-zero".into(),
            });
        }
        Ok(texts.iter().map(|text| self.embed(text)).collect())
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_documents(&[text.to_owned()])
            .await
            .map(|mut values| values.remove(0))
    }
}

type BlockingEmbed = dyn Fn(&[String]) -> Result<Vec<Vec<f32>>> + Send + Sync;

/// Lazy local ONNX provider. Model files are not touched until the first embedding call.
///
/// `ort` 2.0.0-rc.12 has a Rust 1.88 minimum; this workspace's Rust 1.94 floor is stricter.
pub struct OrtEmbeddingProvider {
    model_dir: PathBuf,
    dimensions: usize,
    inference: Option<Arc<BlockingEmbed>>,
    allow_download: bool,
    download_lock: Arc<tokio::sync::Mutex<()>>,
}

impl OrtEmbeddingProvider {
    pub const MODEL_NAME: &'static str = "paraphrase-multilingual-MiniLM-L12-v2";

    #[must_use]
    pub fn new(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            dimensions: 384,
            inference: None,
            allow_download: true,
            download_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Creates an offline provider that returns [`SemanticError::MissingModel`] without network.
    #[must_use]
    pub fn new_offline(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            dimensions: 384,
            inference: None,
            allow_download: false,
            download_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Supplies the already-tokenizing ORT inference boundary used by the model loader.
    #[must_use]
    pub fn with_inference(
        model_dir: impl Into<PathBuf>,
        dimensions: usize,
        inference: Arc<BlockingEmbed>,
    ) -> Self {
        Self {
            model_dir: model_dir.into(),
            dimensions,
            inference: Some(inference),
            allow_download: false,
            download_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    async fn embed_many(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let model = self.model_dir.join("model.onnx");
        let tokenizer = self.model_dir.join("tokenizer.json");
        if (!artifact_ready(&model).await || !artifact_ready(&tokenizer).await)
            && self.allow_download
        {
            let _download = self.download_lock.lock().await;
            if !artifact_ready(&model).await || !artifact_ready(&tokenizer).await {
                download_model_artifacts(&self.model_dir).await?;
            }
        }
        if !artifact_ready(&model).await || !artifact_ready(&tokenizer).await {
            return Err(SemanticError::MissingModel {
                path: model.display().to_string(),
            });
        }
        let input = texts.to_vec();
        let vectors = if let Some(inference) = self.inference.clone() {
            tokio::task::spawn_blocking(move || inference(&input)).await??
        } else {
            let dimensions = self.dimensions;
            tokio::task::spawn_blocking(move || run_ort(&model, &tokenizer, &input, dimensions))
                .await??
        };
        if vectors.len() != texts.len() {
            return Err(SemanticError::InvalidVector {
                reason: "embedding batch cardinality mismatch".into(),
            });
        }
        if vectors.iter().any(|vector| {
            vector.len() != self.dimensions || vector.iter().any(|value| !value.is_finite())
        }) {
            return Err(SemanticError::InvalidVector {
                reason: "embedding output has wrong dimensions or non-finite values".into(),
            });
        }
        Ok(vectors)
    }
}

async fn download_model_artifacts(model_dir: &std::path::Path) -> Result<()> {
    const BASE: &str = "https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/main";
    const MAX_MODEL_BYTES: u64 = 1_000_000_000;
    tokio::fs::create_dir_all(model_dir)
        .await
        .map_err(SemanticError::Io)?;
    for (relative, name) in [
        ("onnx/model.onnx", "model.onnx"),
        ("tokenizer.json", "tokenizer.json"),
    ] {
        let target = model_dir.join(name);
        if artifact_ready(&target).await {
            continue;
        }
        let mut response = reqwest::get(format!("{BASE}/{relative}"))
            .await
            .map_err(|error| SemanticError::Download(error.to_string()))?
            .error_for_status()
            .map_err(|error| SemanticError::Download(error.to_string()))?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_MODEL_BYTES)
        {
            return Err(SemanticError::Download(format!(
                "model artifact {name} exceeds {MAX_MODEL_BYTES} bytes"
            )));
        }
        let temporary = model_dir.join(format!(".{name}.{}", uuid::Uuid::new_v4()));
        let mut file = tokio::fs::File::create(&temporary)
            .await
            .map_err(SemanticError::Io)?;
        let mut written = 0_u64;
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| SemanticError::Download(error.to_string()))?
        {
            written = written.saturating_add(chunk.len() as u64);
            if written > MAX_MODEL_BYTES {
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(SemanticError::Download(format!(
                    "model artifact {name} exceeds {MAX_MODEL_BYTES} bytes"
                )));
            }
            if let Err(error) = file.write_all(&chunk).await {
                drop(file);
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(SemanticError::Io(error));
            }
        }
        if written == 0 {
            drop(file);
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(SemanticError::Download(format!(
                "model artifact {name} was empty"
            )));
        }
        if let Err(error) = file.flush().await {
            drop(file);
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(SemanticError::Io(error));
        }
        drop(file);
        if artifact_ready(&target).await {
            let _ = tokio::fs::remove_file(&temporary).await;
        } else if let Err(error) = tokio::fs::rename(&temporary, target).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(SemanticError::Io(error));
        }
    }
    Ok(())
}

async fn artifact_ready(path: &std::path::Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

fn run_ort(
    model: &std::path::Path,
    tokenizer_path: &std::path::Path,
    texts: &[String],
    dimensions: usize,
) -> Result<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    // Hugging Face does not publish stable checksums in the proposal. Integrity is therefore
    // bounded by HTTPS/status/size/atomic-file checks here and by fully parsing both the tokenizer
    // and ONNX graph before an artifact is accepted for inference.
    let mut tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
        .map_err(|error| SemanticError::Ort(format!("load tokenizer: {error}")))?;
    tokenizer
        .with_truncation(Some(tokenizers::TruncationParams {
            max_length: 256,
            ..Default::default()
        }))
        .map_err(|error| SemanticError::Ort(format!("configure tokenizer truncation: {error}")))?;
    let encodings = tokenizer
        .encode_batch(texts.to_vec(), true)
        .map_err(|error| SemanticError::Ort(format!("tokenize input: {error}")))?;
    let sequence = encodings
        .iter()
        .map(tokenizers::Encoding::len)
        .max()
        .unwrap_or(1)
        .max(1);
    let batch = encodings.len();
    let mut input_ids = vec![0_i64; batch * sequence];
    let mut attention = vec![0_i64; batch * sequence];
    let mut token_types = vec![0_i64; batch * sequence];
    for (row, encoding) in encodings.iter().enumerate() {
        for (column, &id) in encoding.get_ids().iter().enumerate() {
            input_ids[row * sequence + column] = i64::from(id);
            attention[row * sequence + column] = i64::from(encoding.get_attention_mask()[column]);
            token_types[row * sequence + column] = i64::from(encoding.get_type_ids()[column]);
        }
    }
    let ids = ort::value::Tensor::from_array(([batch, sequence], input_ids))
        .map_err(|error| SemanticError::Ort(error.to_string()))?;
    let mask = ort::value::Tensor::from_array(([batch, sequence], attention.clone()))
        .map_err(|error| SemanticError::Ort(error.to_string()))?;
    let types = ort::value::Tensor::from_array(([batch, sequence], token_types))
        .map_err(|error| SemanticError::Ort(error.to_string()))?;
    let mut session = ort::session::Session::builder()
        .map_err(|error| SemanticError::Ort(error.to_string()))?
        .commit_from_file(model)
        .map_err(|error| SemanticError::Ort(error.to_string()))?;
    let has_token_types = session
        .inputs()
        .iter()
        .any(|input| input.name() == "token_type_ids");
    let outputs = if has_token_types {
        session.run(
            ort::inputs!["input_ids" => ids, "attention_mask" => mask, "token_type_ids" => types],
        )
    } else {
        session.run(ort::inputs!["input_ids" => ids, "attention_mask" => mask])
    }
    .map_err(|error| SemanticError::Ort(error.to_string()))?;
    let (shape, data) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|error| SemanticError::Ort(error.to_string()))?;
    let shape = shape.to_vec();
    let expected_batch = i64::try_from(batch).map_err(|_| SemanticError::InvalidVector {
        reason: "embedding batch is too large".into(),
    })?;
    let expected_sequence = i64::try_from(sequence).map_err(|_| SemanticError::InvalidVector {
        reason: "embedding sequence is too large".into(),
    })?;
    let expected_dimensions =
        i64::try_from(dimensions).map_err(|_| SemanticError::InvalidVector {
            reason: "embedding dimensions are too large".into(),
        })?;
    let mut vectors = if shape.as_slice() == [expected_batch, expected_dimensions] {
        data.chunks_exact(dimensions)
            .map(<[f32]>::to_vec)
            .collect::<Vec<_>>()
    } else if shape.as_slice() == [expected_batch, expected_sequence, expected_dimensions] {
        (0..batch)
            .map(|row| {
                let mut vector = vec![0.0_f32; dimensions];
                let mut weight = 0.0_f32;
                for token in 0..sequence {
                    let mask = attention[row * sequence + token] as f32;
                    weight += mask;
                    let start = (row * sequence + token) * dimensions;
                    for (target, value) in vector.iter_mut().zip(&data[start..start + dimensions]) {
                        *target += *value * mask;
                    }
                }
                if weight > 0.0 {
                    for value in &mut vector {
                        *value /= weight;
                    }
                }
                vector
            })
            .collect()
    } else {
        return Err(SemanticError::InvalidVector {
            reason: format!("unexpected ONNX output shape {shape:?}"),
        });
    };
    for vector in &mut vectors {
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if norm > 0.0 {
            for value in vector {
                *value /= norm;
            }
        }
    }
    Ok(vectors)
}

#[async_trait]
impl EmbeddingProvider for OrtEmbeddingProvider {
    fn provider(&self) -> &str {
        "ort"
    }
    fn model(&self) -> &str {
        Self::MODEL_NAME
    }
    fn dimensions(&self) -> usize {
        self.dimensions
    }
    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed_many(texts).await
    }
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_many(&[text.to_owned()])
            .await
            .map(|mut values| values.remove(0))
    }
}
