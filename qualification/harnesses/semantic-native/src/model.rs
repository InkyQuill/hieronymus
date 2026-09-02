use anyhow::{Context, ensure};
use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::Tensor,
};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

/// Embedding width of the pinned qualification model.
pub const EMBEDDING_DIMENSIONS: usize = 384;
/// WordPiece vocabulary size of `sentence-transformers/all-MiniLM-L6-v2`.
/// Qualification token streams are folded into this range so the ONNX graph
/// always gathers in-bounds rows without weakening the run-to-run contract.
const MODEL_VOCABULARY_SIZE: u32 = 30_522;
/// The exported model accepts at most 512 positions per sequence.
const MODEL_MAX_SEQUENCE: usize = 512;
/// Preferred output of the sentence-transformers export; mean pooling runs on
/// the token-level hidden states.
const PREFERRED_OUTPUT: &str = "last_hidden_state";

/// Verified handle over the acquired ONNX runtime and model.
///
/// `load` authenticates the model by SHA-256 before anything is executed and
/// loads the runtime through a checked dynamic link; `embed` mean-pools the
/// token-level hidden states and L2-normalizes the result. All ndarray values
/// stay internal to ort's re-exports; the only value crossing this boundary is
/// an owned `Vec<f32>`.
pub struct OnnxEmbeddingProvider {
    session: Session,
    output_name: String,
    model_checksum: String,
}

impl OnnxEmbeddingProvider {
    /// Loads the shared runtime and the model after verifying the model
    /// checksum.
    ///
    /// `expected_sha256` must match the SHA-256 of `model`, mirroring
    /// `qualification/prerequisites.json`. The runtime is authenticated by
    /// checked dynamic load: `ort::init_from` dlopens the shared library and
    /// rejects any binary that fails to export a compatible ONNX Runtime API,
    /// so a corrupted runtime cannot silently pass. (The release checksum in
    /// `prerequisites.json` covers the downloaded archive, which acquisition
    /// verifies before extraction.) The dynamic library is loaded before any
    /// other ort API is touched, as required by `ort::init_from`.
    pub fn load(runtime: &Path, model: &Path, expected_sha256: &str) -> anyhow::Result<Self> {
        ensure!(
            runtime.is_file(),
            "onnx runtime shared library {} does not exist",
            runtime.display()
        );

        let model_checksum = sha256_hex(model)
            .with_context(|| format!("could not hash the model at {}", model.display()))?;
        let expected = expected_sha256.trim().to_ascii_lowercase();
        ensure!(
            model_checksum == expected,
            "model checksum mismatch for {}: expected {expected}, got {model_checksum}",
            model.display()
        );

        ort::init_from(runtime)
            .with_context(|| {
                format!(
                    "could not load the verified onnx runtime from {}",
                    runtime.display()
                )
            })?
            .commit();

        let session = Session::builder()
            .map_err(|error| anyhow::anyhow!("could not create the session builder: {error}"))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|error| {
                anyhow::anyhow!("could not set the graph optimization level: {error}")
            })?
            .with_intra_threads(2)
            .map_err(|error| anyhow::anyhow!("could not set intra-op threads: {error}"))?
            .commit_from_file(model)
            .with_context(|| format!("could not load the model at {}", model.display()))?;

        let output_name = if session
            .outputs()
            .iter()
            .any(|outlet| outlet.name() == PREFERRED_OUTPUT)
        {
            PREFERRED_OUTPUT.to_string()
        } else {
            session
                .outputs()
                .first()
                .context("model exposes no outputs")?
                .name()
                .to_string()
        };

        Ok(Self {
            session,
            output_name,
            model_checksum,
        })
    }

    /// The verified model checksum (lower-case hexadecimal SHA-256).
    pub fn model_checksum(&self) -> &str {
        &self.model_checksum
    }

    /// Embeds one token sequence: mean-pool over positions, then L2-normalize.
    pub fn embed(&mut self, token_ids: &[u32]) -> anyhow::Result<Vec<f32>> {
        ensure!(
            !token_ids.is_empty(),
            "cannot embed an empty token sequence"
        );
        ensure!(
            token_ids.len() <= MODEL_MAX_SEQUENCE,
            "token sequence of {} exceeds the model limit of {MODEL_MAX_SEQUENCE}",
            token_ids.len()
        );

        let sequence = token_ids.len() as i64;
        let shape = vec![1_i64, sequence];
        let input_ids: Vec<i64> = token_ids
            .iter()
            .map(|token| (token % MODEL_VOCABULARY_SIZE) as i64)
            .collect();
        let attention_mask = vec![1_i64; token_ids.len()];
        let token_type_ids = vec![0_i64; token_ids.len()];

        let outputs = self.session.run(ort::inputs![
            "input_ids" => Tensor::from_array((shape.clone(), input_ids))?,
            "attention_mask" => Tensor::from_array((shape.clone(), attention_mask))?,
            "token_type_ids" => Tensor::from_array((shape, token_type_ids))?,
        ])?;

        let output = outputs
            .get(self.output_name.as_str())
            .with_context(|| format!("model output {} is missing", self.output_name))?;
        let (shape, hidden_states) = output.try_extract_tensor::<f32>()?;
        let dims: &[i64] = &shape;
        ensure!(
            dims.len() == 3 && dims[0] == 1,
            "unexpected hidden state shape {dims:?}"
        );
        ensure!(
            dims[2] as usize == EMBEDDING_DIMENSIONS,
            "unexpected hidden state width {}: expected {EMBEDDING_DIMENSIONS}",
            dims[2]
        );
        let positions = dims[1] as usize;
        ensure!(
            hidden_states.len() == positions * EMBEDDING_DIMENSIONS,
            "hidden state payload is truncated"
        );

        // Attention mask is all ones for a single unpadded sequence, so the
        // mean pool divides by the position count.
        let mut pooled = vec![0.0_f32; EMBEDDING_DIMENSIONS];
        for position in 0..positions {
            let row = &hidden_states[position * EMBEDDING_DIMENSIONS..][..EMBEDDING_DIMENSIONS];
            for (sum, value) in pooled.iter_mut().zip(row) {
                *sum += value;
            }
        }
        for value in &mut pooled {
            *value /= positions as f32;
        }

        let norm = pooled.iter().map(|value| value * value).sum::<f32>().sqrt();
        ensure!(norm > f32::EPSILON, "cannot normalize a zero embedding");
        for value in &mut pooled {
            *value /= norm;
        }
        Ok(pooled)
    }
}

fn sha256_hex(path: &Path) -> anyhow::Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
