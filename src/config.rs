use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Provider {
    Anthropic,
    Bedrock,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub provider: Provider,
    /// Strong model for the review lenses.
    pub lens_model: String,
    /// Model for eligibility + per-finding verification. Defaults to the lens
    /// model (at low effort): the 2026-08-17 bake-off showed a weak verifier
    /// invents wrong rejection rationales and suppresses real findings.
    pub verify_model: String,
    pub aws_region: String,
    /// Findings below this verifier confidence are dropped (0-100).
    pub confidence_threshold: u8,
    /// Per-lens hard timeout.
    pub lens_timeout_secs: u64,
    pub verify_timeout_secs: u64,
    pub lens_max_tokens: u32,
    pub verify_max_tokens: u32,
    /// Cap on file content included in the pack, per file.
    pub max_file_lines: usize,
    /// Cap on changed files fully inlined into the pack.
    pub max_pack_files: usize,
    /// Cap on the rendered pack size; past it, remaining file contents are
    /// omitted (their diffs stay) and the pack says so.
    pub max_pack_chars: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let provider = match std::env::var("GREYBEARD_PROVIDER").ok().as_deref() {
            Some("anthropic") => Provider::Anthropic,
            Some("bedrock") => Provider::Bedrock,
            Some(other) => bail!("GREYBEARD_PROVIDER must be 'anthropic' or 'bedrock', got '{other}'"),
            // Default by what credentials are present.
            None => {
                if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                    Provider::Anthropic
                } else {
                    Provider::Bedrock
                }
            }
        };

        let lens_default = match provider {
            Provider::Anthropic => "claude-opus-5",
            // Bedrock model/inference-profile IDs vary by account setup — require them explicitly.
            Provider::Bedrock => "",
        };
        let lens_model =
            std::env::var("GREYBEARD_LENS_MODEL").unwrap_or_else(|_| lens_default.to_string());
        if lens_model.is_empty() {
            bail!(
                "provider=bedrock requires GREYBEARD_LENS_MODEL \
                 (a Bedrock inference-profile ID, e.g. us.anthropic.claude-...)"
            );
        }
        let verify_model =
            std::env::var("GREYBEARD_VERIFY_MODEL").unwrap_or_else(|_| lens_model.clone());

        Ok(Self {
            provider,
            lens_model,
            verify_model,
            aws_region: std::env::var("AWS_DEFAULT_REGION")
                .or_else(|_| std::env::var("AWS_REGION"))
                .unwrap_or_else(|_| "us-east-2".to_string()),
            confidence_threshold: 80,
            lens_timeout_secs: 240,
            verify_timeout_secs: 120,
            lens_max_tokens: 16_000,
            verify_max_tokens: 1_500,
            max_file_lines: 2_000,
            max_pack_files: 60,
            max_pack_chars: 600_000,
        })
    }
}
