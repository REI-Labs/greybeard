# greybeard

<img src="assets/avatar.png" alt="Greybeard" width="120" align="right" />

The mythical senior engineer who's seen every failure mode.

Greybeard is a PR-review service: six specialist review lenses run in parallel over a
pre-fetched context pack, every candidate finding is adversarially verified by an
independent low-effort pass on the same strong model, and the survivors land as
**one** PR comment that is updated in place on re-review. Target wall time: under 5 minutes (typically 2–3.5).

## How it works

```
Stage 0   context pack (one GraphQL query + diff + file contents, ~1-2s)
          deterministic eligibility: closed / draft / already-reviewed-SHA → skip
Stage 0b  cache warm (max_tokens:0 prefill) ∥ trivial-PR check (fast model)
Stage 1   6 lenses in parallel (strong model, zero tools, pack is a cached prefix):
            ci-config · claude-md · bugs · history(blame) · prior-feedback · code-comments
Stage 2   per-finding adversarial verify (lens model at low effort, REFUTE-framed, 0-100 rubric)
          — pipelined per lens, no barrier; keep real=true && confidence ≥ 80
Stage 3   dedupe → render → upsert the single Greybeard comment (marker = state)
```

The context pack is rendered **byte-deterministically** — it is the shared
prompt-cache prefix for every model call. Watch `cache_rd` in the telemetry table:
zero on repeat calls means a nondeterminism bug crept into the pack renderer.

## Usage

```sh
greybeard review https://github.com/OWNER/REPO/pull/N [--dry-run] [--force]
greybeard pack   https://github.com/OWNER/REPO/pull/N     # print the pack, no model calls
```

`--dry-run` prints the comment instead of posting. `--force` reviews even if the PR
is closed / draft / already reviewed at this SHA / judged trivial.

### Service mode

```sh
greybeard serve --port 8080     # GitHub App webhook -> automatic reviews
```

Subscribed events: `pull_request` (opened / synchronize / ready_for_review /
reopened) and `issue_comment` (`@greybeard-bot review` forces a re-review).
Deliveries are HMAC-verified (`GREYBEARD_WEBHOOK_SECRET`), deduped by delivery
GUID, debounced per PR (a new push aborts the in-flight review and restarts on
the newest head), and logged to `GREYBEARD_LOG_FILE` (JSONL). `GET /health`
for probes. Deploy artifacts: `Dockerfile` + `deploy/k8s.yaml`
(greybeard.apps.govorch.ai behind the shared ALB); CI in
`.github/workflows/ci.yml` — test on every PR, sha7 image to ECR + rollout
from master once the repo vars documented in the workflow header are set.

## Configuration (env)

| Var | Meaning | Default |
| --- | --- | --- |
| `GREYBEARD_PROVIDER` | `anthropic` or `bedrock` | `anthropic` if `ANTHROPIC_API_KEY` set, else `bedrock` |
| `ANTHROPIC_API_KEY` | Anthropic API key (provider=anthropic) | — |
| `GREYBEARD_LENS_MODEL` | strong model for the 6 lenses | `claude-opus-5` (anthropic); **required** for bedrock, e.g. `us.anthropic.claude-opus-5` |
| `GREYBEARD_VERIFY_MODEL` | model for eligibility + per-finding verification (runs at low effort) | the lens model — a weak verifier suppresses real findings (see docs/BAKEOFF-2026-08-17.md) |
| `AWS_REGION` / `AWS_DEFAULT_REGION` | Bedrock region | `us-east-2` |
| `GITHUB_TOKEN` / `GH_TOKEN` | GitHub user token | falls back to `gh auth token` |
| `GREYBEARD_APP_ID` + `GREYBEARD_APP_PRIVATE_KEY` (pem path) + `GREYBEARD_APP_INSTALLATION_ID` | GitHub App identity — when all three are set they take precedence and comments post as the app bot | unset (user token) |

Provider notes:
- **anthropic**: structured output is schema-enforced server-side
  (`output_config.format`); lenses run at `effort: high`.
- **bedrock**: legacy InvokeModel wire shape (SigV4, service `bedrock`);
  no server-side schema enforcement — JSON shape is enforced by prompt +
  parse-with-retry, and missing optional fields are defaulted rather than fatal.

## GitHub transport note (corporate proxy)

In this environment the proxy blocks several REST endpoints (`/pulls/N/files`,
`/pulls/N/commits`, `/issues/N/comments`, check-runs) while GraphQL, the diff media
type, and the contents API pass. Greybeard is therefore **GraphQL-first**: one query
fetches PR metadata + changed files + comments + CI rollup; blame and prior-PR
feedback are GraphQL; comments are posted via `addComment`/`updateIssueComment`
mutations. Only the unified diff and file contents use REST.

## Build & test (no local toolchain needed)

```sh
docker run --rm -v "$PWD":/work -v greybeard-cargo:/usr/local/cargo/registry \
  -w /work rust:1-bookworm cargo test

# run a review from the container (Bedrock via mounted AWS creds):
docker run --rm -v "$PWD":/work -v greybeard-cargo:/usr/local/cargo/registry \
  -v ~/.aws:/root/.aws:ro -w /work \
  -e GREYBEARD_PROVIDER=bedrock \
  -e GREYBEARD_LENS_MODEL=us.anthropic.claude-opus-5 \
  -e GITHUB_TOKEN=$(gh auth token) \
  rust:1-bookworm ./target/debug/greybeard review <pr-url> --dry-run --force
```

## Voice

How comments are written — persona, comment anatomy, severity vocabulary, hard
rules — is specified in [docs/VOICE.md](docs/VOICE.md). `src/prompts.rs` and
`src/pipeline/compose.rs` implement that contract; change them together.

The avatar lives at `assets/avatar.png` (alternates in `assets/candidates/`) —
use it for the GitHub App identity in Phase 3.

## Roadmap

- Phase 2 remaining: SSE streaming for lens calls, per-lens retry-at-lower-effort,
  cost line in telemetry, dedupe window tuning.
- Phase 3 service: axum webhook (`pull_request` opened/synchronize) behind a GitHub
  App identity, per-PR debounce, SQLite run log. The CLI and the webhook share the
  same `pipeline::review` entry point.
