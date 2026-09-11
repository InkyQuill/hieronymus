//! Strict split-release transport identity; the legacy decoder is separate.
use hieronymus::semantic_model::{MODEL_NAME, MODEL_REVISION, MODEL_SHA256, TOKENIZER_SHA256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub archive: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelArtifact {
    pub name: String,
    pub revision: String,
    pub archive: String,
    pub sha256: String,
    #[serde(deserialize_with = "unique_members")]
    pub members: BTreeMap<String, String>,
}
pub(crate) fn unique_members<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    struct Unique;
    impl<'de> serde::de::Visitor<'de> for Unique {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique member digest map")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if result.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate model member"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Unique)
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseV2 {
    pub format_version: u32,
    pub version: String,
    pub target: String,
    pub channel: String,
    pub platform: Artifact,
    pub model: ModelArtifact,
    pub signature: serde_json::Value,
}
pub const TARGETS: [&str; 4] = [
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
];
pub fn metadata_name(target: &str) -> String {
    format!("release-{target}.json")
}
pub fn platform_name(version: &str, target: &str) -> String {
    format!(
        "hieronymus-{version}-{target}{}",
        if target == TARGETS[1] {
            ".zip"
        } else {
            ".tar.gz"
        }
    )
}
pub fn model_name() -> String {
    format!("hieronymus-model-{MODEL_NAME}-{MODEL_REVISION}.tar.gz")
}
pub fn model_members() -> BTreeMap<String, String> {
    [
        ("model.onnx", MODEL_SHA256),
        ("tokenizer.json", TOKENIZER_SHA256),
        (
            "LICENSE",
            "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
        ),
        (
            "README.md",
            "1e98ea05b0de579fcaad3d625b62ea55647142ed674d5f5ebf1440e4bbbb6f23",
        ),
    ]
    .into_iter()
    .map(|(name, digest)| (format!("models/minilm/{name}"), digest.into()))
    .collect()
}
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl ReleaseV2 {
    pub fn parse(bytes: &[u8], target: &str) -> Result<Self, String> {
        if bytes.len() > 65536 {
            return Err("release metadata exceeds byte bound".into());
        }
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|e| format!("invalid v2 release metadata: {e}"))?;
        value.validate(target)?;
        Ok(value)
    }
    pub fn validate(&self, target: &str) -> Result<(), String> {
        let base_version = self.version.split(['-', '+']).next().unwrap_or_default();
        let suffix_valid = self.version.find(['-', '+']).is_none_or(|index| {
            let suffix = &self.version[index + 1..];
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
        });
        if !suffix_valid
            || self.format_version != 2
            || !TARGETS.contains(&target)
            || self.target != target
            || base_version.split('.').count() != 3
            || base_version
                .split('.')
                .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
            || !self
                .version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
            || !["stable", "dev"].contains(&self.channel.as_str())
            || !self.signature.is_null()
        {
            return Err("release format/version/target/channel/signature mismatch".into());
        }
        if self.platform.archive != platform_name(&self.version, target)
            || self.model.archive != model_name()
            || self.model.name != MODEL_NAME
            || self.model.revision != MODEL_REVISION
            || self.model.members != model_members()
            || !valid_digest(&self.platform.sha256)
            || !valid_digest(&self.model.sha256)
        {
            return Err("release artifact/model identity mismatch".into());
        }
        Ok(())
    }
}
