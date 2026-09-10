//! Explicit asset acquisition and configuration, under the live root owner.
use super::super::DaemonRuntime;
use super::*;
use serde::Deserialize;
use serde_json::{Value, json};

type Configure = hieronymus::semantic_arming::SemanticConfiguration;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Acquisition {
    url: Option<String>,
    sha256: Option<String>,
    bytes: Option<String>,
    #[serde(default)]
    tokenizer_only: bool,
}

// Final promotion is serialized. The enclosing connection worker remains
// supervised by the daemon, including when its requesting CLI disconnects.
static ACQUISITION: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn handle(request: &Request, runtime: &DaemonRuntime, acquire: bool) -> Response {
    if request.method != "POST" {
        return not_found();
    }
    if !host_is_valid(request, runtime) {
        return invalid_host();
    }
    if !bearer_matches(request, runtime) {
        return unauthorized();
    }
    if !origin_is_absent_or_valid(request, runtime) {
        return forbidden_origin();
    }
    let result = if acquire {
        serde_json::from_slice::<Acquisition>(&request.body)
            .map_err(|e| e.to_string())
            .and_then(|settings| acquire_assets(&runtime.config, &settings))
    } else {
        serde_json::from_slice::<Configure>(&request.body)
            .map_err(|e| e.to_string())
            .and_then(|settings| {
                let acknowledgement = runtime.semantic.configure_settings(settings)?;
                use crate::daemon::semantic_worker::RequiredSemanticState;
                let (state, detail) = match acknowledgement.state {
                    RequiredSemanticState::Acquiring => ("acquiring", Value::Null),
                    RequiredSemanticState::Rebuilding => ("rebuilding", Value::Null),
                    RequiredSemanticState::Ready => ("ready", Value::Null),
                    RequiredSemanticState::Failed(reason) => ("failed", json!(reason)),
                };
                Ok(json!({"state": state, "detail": detail, "configuration_revision": acknowledgement.configuration_revision}))
            })
    };
    match result {
        Ok(value) => Response::json(200, &value),
        Err(error) => Response::json(400, &json!({"error": error})),
    }
}

fn acquire_assets(
    config: &hieronymus::data_root::HieronymusConfig,
    settings: &Acquisition,
) -> Result<Value, String> {
    let _guard = ACQUISITION.lock().map_err(|error| error.to_string())?;
    use hieronymus::semantic_model::{
        DEFAULT_MODEL_URL, HttpModelTransport, MODEL_BYTES, MODEL_SHA256, ModelStatus,
    };

    let store = hieronymus::semantic_store::SemanticStore::open(config)
        .map_err(|error| error.to_string())?;
    if settings.tokenizer_only
        && (settings.url.is_some() || settings.sha256.is_some() || settings.bytes.is_some())
    {
        return Err("tokenizer_only cannot be combined with model artifact overrides".into());
    }
    let url = settings.url.as_deref().unwrap_or(DEFAULT_MODEL_URL);
    let expected_sha = settings.sha256.as_deref().unwrap_or(MODEL_SHA256);
    let expected_bytes = match &settings.bytes {
        None => MODEL_BYTES,
        Some(text) => text
            .parse::<u64>()
            .map_err(|_| format!("--bytes requires a byte count, got {text}"))?,
    };
    // With an explicit `--bytes` override, availability is judged solely
    // against that expectation and the user's checksum: a file that merely
    // matches the pinned size says nothing about the requested artifact, so
    // skipping the download would report an unverified `--sha256` as
    // available. The pinned default keeps Task 7's cheap size pre-check
    // (cryptographic verification happens at provider load).
    let local_status = || match &settings.bytes {
        Some(_) => match std::fs::metadata(store.model_path()) {
            Ok(metadata) if metadata.len() == expected_bytes => {
                match hieronymus::semantic_model::sha256_file(&store.model_path()) {
                    Ok(digest) if digest == expected_sha.trim().to_ascii_lowercase() => {
                        ModelStatus::Available
                    }
                    Ok(digest) => ModelStatus::Invalid(format!(
                        "model file checksum mismatch: expected {expected_sha}, got {digest}"
                    )),
                    Err(error) => {
                        ModelStatus::Invalid(format!("model file could not be hashed: {error}"))
                    }
                }
            }
            Ok(metadata) => ModelStatus::Invalid(format!(
                "model file is {} bytes, expected {expected_bytes}",
                metadata.len()
            )),
            Err(_) => ModelStatus::Missing,
        },
        None => store.model_status(),
    };

    // Explicit acquisition: download only when the local file is missing or
    // fails its size pre-check; a healthy model is never re-fetched.
    let mut downloaded = false;
    if !settings.tokenizer_only && local_status() != ModelStatus::Available {
        let transport = HttpModelTransport::new(std::time::Duration::from_secs(600));
        store
            .acquire_model_verifying(&transport, url, expected_sha, expected_bytes)
            .map_err(|error| error.to_string())?;
        downloaded = true;
    }
    // The tokenizer asset rides the same explicit acquisition with the same
    // discipline, always pinned: with explicit artifact overrides in play the
    // caller manages artifacts themselves (and the loopback test harness must
    // never egress), so the tokenizer is fetched only in pinned-default mode.
    let pinned_defaults =
        settings.url.is_none() && settings.sha256.is_none() && settings.bytes.is_none();
    let mut tokenizer_downloaded = false;
    if pinned_defaults && store.tokenizer_status() != ModelStatus::Available {
        let transport = HttpModelTransport::new(std::time::Duration::from_secs(600));
        store
            .acquire_tokenizer(
                &transport,
                hieronymus::semantic_model::DEFAULT_TOKENIZER_URL,
            )
            .map_err(|error| error.to_string())?;
        tokenizer_downloaded = true;
    }
    let final_status = local_status();

    Ok(json!({
        "model_status": match final_status { ModelStatus::Available => "available", ModelStatus::Invalid(_) => "invalid", ModelStatus::Missing => "missing" },
        "downloaded": downloaded,
        "tokenizer_downloaded": tokenizer_downloaded,
        "model_path": store.model_path(),
        "tokenizer_path": store.tokenizer_path(),
    }))
}
