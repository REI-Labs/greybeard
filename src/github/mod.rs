pub mod comment;
pub mod pack;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

const API: &str = "https://api.github.com";

/// PR coordinates parsed from a URL like https://github.com/owner/repo/pull/123
#[derive(Debug, Clone)]
pub struct PrRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

impl PrRef {
    pub fn parse(url: &str) -> Result<Self> {
        let trimmed = url.trim_end_matches('/');
        let parts: Vec<&str> = trimmed
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .collect();
        // github.com / owner / repo / pull / N
        if parts.len() < 5 || !parts[0].ends_with("github.com") || parts[3] != "pull" {
            bail!("not a PR URL: {url} (expected https://github.com/<owner>/<repo>/pull/<n>)");
        }
        Ok(Self {
            owner: parts[1].to_string(),
            repo: parts[2].to_string(),
            number: parts[4].parse().context("PR number")?,
        })
    }
}

/// Thin GitHub client over plain reqwest — REST + GraphQL.
pub struct Github {
    http: reqwest::Client,
    token: String,
}

impl Github {
    pub async fn new() -> Result<Self> {
        let token = match std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")) {
            Ok(t) if !t.is_empty() => t,
            _ => {
                let out = tokio::process::Command::new("gh")
                    .args(["auth", "token"])
                    .output()
                    .await
                    .context("no GITHUB_TOKEN and `gh auth token` failed to run")?;
                if !out.status.success() {
                    bail!("no GITHUB_TOKEN/GH_TOKEN set and `gh auth token` returned an error");
                }
                String::from_utf8(out.stdout)?.trim().to_string()
            }
        };
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            token,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{API}{path}")
        };
        self.http
            .request(method, url)
            .header("authorization", format!("Bearer {}", self.token))
            .header("user-agent", "greybeard")
            .header("x-github-api-version", "2022-11-28")
    }

    /// GET returning JSON, mapping 404 to None instead of an error.
    pub async fn get_optional(&self, path: &str) -> Result<Option<Value>> {
        let r = self
            .request(reqwest::Method::GET, path)
            .header("accept", "application/vnd.github+json")
            .send()
            .await?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let status = r.status();
        let v: Value = r.json().await?;
        if !status.is_success() {
            bail!("GET {path}: {status}: {}", v["message"].as_str().unwrap_or("?"));
        }
        Ok(Some(v))
    }

    /// GET with a raw media type (e.g. the unified diff).
    pub async fn get_raw(&self, path: &str, accept: &str) -> Result<String> {
        let r = self
            .request(reqwest::Method::GET, path)
            .header("accept", accept)
            .send()
            .await?;
        let status = r.status();
        let text = r.text().await?;
        if !status.is_success() {
            bail!("GET {path} ({accept}): {status}");
        }
        Ok(text)
    }

    pub async fn graphql(&self, query: &str, variables: Value) -> Result<Value> {
        let r = self
            .request(reqwest::Method::POST, "/graphql")
            .json(&serde_json::json!({"query": query, "variables": variables}))
            .send()
            .await?;
        let status = r.status();
        let v: Value = r.json().await?;
        if !status.is_success() {
            bail!("graphql: {status}");
        }
        if let Some(errors) = v["errors"].as_array() {
            if !errors.is_empty() {
                bail!("graphql errors: {}", errors[0]["message"].as_str().unwrap_or("?"));
            }
        }
        Ok(v["data"].clone())
    }

    /// Fetch a file's contents at a ref. None for 404 / non-file / too large.
    pub async fn file_contents(
        &self,
        pr: &PrRef,
        path: &str,
        git_ref: &str,
    ) -> Result<Option<String>> {
        let encoded = urlencode_path(path);
        let url = format!(
            "/repos/{}/{}/contents/{}?ref={}",
            pr.owner, pr.repo, encoded, git_ref
        );
        let Some(v) = self.get_optional(&url).await? else {
            return Ok(None);
        };
        let Some(content) = v["content"].as_str() else {
            return Ok(None);
        };
        use base64::Engine;
        let cleaned: String = content.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(cleaned)
            .map_err(|e| anyhow!("base64 for {path}: {e}"))?;
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

fn urlencode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            seg.bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        (b as char).to_string()
                    }
                    _ => format!("%{b:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}
