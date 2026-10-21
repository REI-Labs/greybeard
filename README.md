# greybeard

<img src="assets/avatar.png" alt="Greybeard" width="120" align="right" />

The mythical senior engineer who's seen every failure mode.

Greybeard is a PR-review service. Six specialist review lenses run in parallel over a
pre-fetched context pack. An independent low-effort pass on the same strong model then
tries to refute every candidate finding. The survivors land as **one** PR comment, which
is updated in place on re-review. Target wall time is under 5 minutes (typically 2–3.5).

## Quick start

1. **Create the GitHub App.** Follow [docs/GITHUB_APP.md](docs/GITHUB_APP.md). A ready-made
   manifest lives at [`deploy/github-app-manifest.json`](deploy/github-app-manifest.json).
2. **Deploy the service.** Follow [docs/DEPLOY.md](docs/DEPLOY.md). It covers the Helm chart
   with the Anthropic API or with Amazon Bedrock on EKS, plus a plain `docker run` path.
3. **Open a pull request** in a repo where the App is installed. To force a re-review,
   comment `@<app-slug> review` on the PR.

Images are published to `ghcr.io/rei-labs/greybeard` for `linux/amd64` and `linux/arm64`.

## How it works

```
Stage 0   context pack (one GraphQL query + diff + file contents, ~1-2s)
          deterministic eligibility: closed / draft / already-reviewed-SHA → skip
Stage 0b  cache warm (max_tokens:0 prefill) ∥ trivial-PR check (fast model)
Stage 1   6 lenses in parallel (strong model, zero tools, pack is a cached prefix):
            ci-config · claude-md · bugs · history(blame) · prior-feedback · code-comments
Stage 2   per-finding adversarial verify (lens model at low effort, REFUTE-framed, 0-100 rubric)
          — pipelined per lens, no barrier; banded gate (docs/GATE.md):
            real && ≥80 → findings, real && ≥60 → collapsed Minor notes
Stage 3   dedupe → render → upsert the single Greybeard comment (marker = state)
```

The context pack is rendered **byte-deterministically**, because it is the shared
prompt-cache prefix for every model call. Watch `cache_rd` in the telemetry table. If it
reads zero on repeat calls, a nondeterminism bug has crept into the pack renderer.

## Usage

```sh
greybeard review https://github.com/OWNER/REPO/pull/N [--dry-run] [--force]
greybeard pack   https://github.com/OWNER/REPO/pull/N     # print the pack, no model calls
greybeard auth-check                                      # show which GitHub identity is in use
```

- `--dry-run` prints the comment instead of posting it.
- `--force` reviews even if the PR is closed, a draft, already reviewed at this SHA, or
  judged trivial.

### Service mode

```sh
greybeard serve --port 8080     # GitHub App webhook -> automatic reviews
```

Greybeard subscribes to two events:

- **`pull_request`**: `opened`, `synchronize`, `ready_for_review`, and `reopened` start a review.
- **`issue_comment`**: a PR comment that starts with `@<app-slug> review` forces a re-review.

Deliveries are HMAC-verified (`GREYBEARD_WEBHOOK_SECRET`) and deduped by delivery GUID.
They are also debounced per PR: a new push aborts the in-flight review and restarts it on
the newest head, and the aborted run's spend is still counted. The installation ID comes
from each webhook payload, so one deployment serves every installation of the App.

#### Observability

- **Run log.** Every finished run writes one structured `review.done` JSON line to stdout,
  with outcome, duration, finding counts, tokens, and optional cost. Ship stdout with your
  log pipeline, because that line is the durable run record. `GREYBEARD_LOG_FILE` is only
  a local JSONL copy.
- **Metrics.** `GET /metrics` serves Prometheus text: reviews by outcome (posted, skipped,
  error, daily-limit, superseded), duration, tokens by kind, cost, an inflight gauge, and
  consecutive failures.
- **Health.** `GET /health` reports `degraded` after 3 consecutive review failures. It
  always returns HTTP 200, so a failing review never gets the pod pulled from a load
  balancer. The `degraded` status is for dashboards. Only a completed run resets the
  streak, because a skipped PR proves nothing about the model path.

#### Guardrails

- PRs opened by bots are skipped before any model call. Set `GREYBEARD_REVIEW_BOT_PRS=true`
  to review them.
- At most `GREYBEARD_MAX_CONCURRENT` (2) reviews run at once.
- A daily circuit breaker caps runs at `GREYBEARD_DAILY_REVIEW_LIMIT` (50) per UTC day.
- The @-mention command has a 10-minute cooldown per person.
- Throttled or transient model errors retry with backoff.
- Pack content is treated as untrusted data, and the prompts forbid it from lowering
  scrutiny. See the trust-boundary paragraph in `src/prompts.rs`.

## Configuration (env)

| Var | Meaning | Default |
| --- | --- | --- |
| `GREYBEARD_PROVIDER` | `anthropic` or `bedrock` | `anthropic` if `ANTHROPIC_API_KEY` is set, else `bedrock` |
| `ANTHROPIC_API_KEY` | Anthropic API key (provider=anthropic) | — |
| `GREYBEARD_LENS_MODEL` | Strong model for the 6 lenses | `claude-opus-5-5` (anthropic); **required** for bedrock, e.g. `us.anthropic.claude-opus-5-5` |
| `GREYBEARD_VERIFY_MODEL` | Model for eligibility and per-finding verification (runs at low effort; must support `effort`) | The lens model. A weak verifier suppresses real findings (see docs/BAKEOFF-2026-08-17.md). |
| `AWS_DEFAULT_REGION` / `AWS_REGION` | Bedrock region (`AWS_DEFAULT_REGION` wins if both are set) | `us-east-1` |
| `GITHUB_TOKEN` / `GH_TOKEN` | GitHub user token | Falls back to `gh auth token` |
| `GREYBEARD_APP_ID` + `GREYBEARD_APP_PRIVATE_KEY` (pem path) + `GREYBEARD_APP_INSTALLATION_ID` | GitHub App identity. When set, it takes precedence and comments post as the App bot. In serve mode, the webhook payload's installation ID overrides the env value. | Unset (user token) |
| `GREYBEARD_WEBHOOK_SECRET` | Webhook HMAC secret. Must match the App setting byte for byte. | **Required** in serve mode |
| `GREYBEARD_BOT_LOGIN` | The App's bot login, `<app-slug>[bot]`. Used to ignore its own comments and to define the `@<app-slug> review` command. | `greybeard-bot[bot]` |
| `GREYBEARD_FEEDBACK_URL` | "bugs/ideas" link in the comment footer. Set it to an empty string to drop the link. | Unset: the upstream issues page, `https://github.com/REI-Labs/greybeard/issues` |
| `GREYBEARD_REVIEW_BOT_PRS` | Review PRs opened by bot accounts (`true` to enable) | `false` |
| `GREYBEARD_MAX_CONCURRENT` / `GREYBEARD_DAILY_REVIEW_LIMIT` | Serve-mode spend guardrails | 2 / 50 per UTC day |
| `GREYBEARD_LOG_FILE` | Local JSONL run log | `greybeard-runs.jsonl` |
| `GREYBEARD_PRICE_IN` / `_OUT` / `_CACHE_READ` / `_CACHE_WRITE` | $ per million tokens for cost telemetry. All four are required. Tokens are reported either way. | Unset (cost omitted) |

Provider notes:

- **Both providers** send `output_config.effort`: `high` for lenses, `low` for
  verification. Both models must support `effort` (Opus 4.5+, Sonnet 4.6+, Haiku 5.5);
  models without it, such as Haiku 4.5 or Sonnet 4.5, are rejected.
- **anthropic**: Structured output is schema-enforced server-side
  (`output_config.format`).
- **bedrock**: Uses the legacy InvokeModel wire shape (SigV4, service `bedrock`).
  There's no server-side schema enforcement, so the JSON shape is enforced by the prompt
  plus parse-with-retry. Missing optional fields get defaults instead of failing the run.

## GitHub transport note

Greybeard is **GraphQL-first**. One query fetches PR metadata, changed files, comments,
and the CI rollup. Blame and prior-PR feedback are GraphQL too, and comments are posted
with the `addComment` and `updateIssueComment` mutations. Only the unified diff and file
contents use REST.

There are two reasons for this:

- It collapses about 6 REST round-trips into one query.
- It proved the more resilient surface during the 2026-08-17 GitHub partial outage, when
  several REST sub-resource endpoints returned 404s for hours.

## Build & test (no local toolchain needed)

```sh
docker run --rm -v "$PWD":/work -v greybeard-cargo:/usr/local/cargo/registry \
  -w /work rust:1-bookworm cargo test

# run a review from the container (Bedrock via mounted AWS creds):
docker run --rm -v "$PWD":/work -v greybeard-cargo:/usr/local/cargo/registry \
  -v ~/.aws:/root/.aws:ro -w /work \
  -e GREYBEARD_PROVIDER=bedrock \
  -e GREYBEARD_LENS_MODEL=us.anthropic.claude-opus-5-5 \
  -e GITHUB_TOKEN=$(gh auth token) \
  rust:1-bookworm ./target/debug/greybeard review <pr-url> --dry-run --force
```

## Voice

[docs/VOICE.md](docs/VOICE.md) specifies how comments are written: persona, comment
anatomy, severity vocabulary, and hard rules. `src/prompts.rs` and
`src/pipeline/compose.rs` implement that contract, so change them together.

The comment's hidden marker carries machine-readable findings (marker v2). Fix-loop
tooling can read it to drive a fix → push → re-review cycle until the verdict is clean.

The avatar lives at `assets/avatar.png`, with alternates in `assets/candidates/`. The
GitHub App avatar is `assets/avatar-app.png`.

## Roadmap

Shipped:

- **1.0.0**: CLI and webhook service.
- **1.1.0**: marker v2 machine contract for fix-loop tooling.
- **1.2.0**: ops and reliability. Run events, `/metrics`, degraded health, and
  panic-safe accounting.
- **1.3.0**: open-source packaging. A Helm chart, an OpenTofu module for AWS, and
  multi-arch images on GHCR.

Next:

- **Trust & learning.** Feedback harvesting (reactions plus a `wrong:` reply channel),
  an optional check-run mode so repos can require Greybeard in branch protection, and
  per-repo `.greybeard.md` review standards.
- **Review quality.** A bounded exec-check tool for blocker-class verification (the known
  recall ceiling), plus a dedupe window and threshold re-tune based on feedback data.
- **Performance polish**, if usage calls for it. Per-lens retry at lower effort, SSE
  streaming for long lens calls, and migrating to the Bedrock Mantle endpoint (which
  would unlock server-side schema enforcement on Bedrock).
