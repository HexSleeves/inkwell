# Plan 041: Enable Renovate for Cargo dependencies with grouping

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 0819727..HEAD -- renovate.json`
> If renovate.json changed, compare the "Current state" excerpt before proceeding.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `0819727`, 2026-06-26

## Why this matters

`renovate.json` is active (recent git log shows `chore(deps): update rust crate anyhow to v1.0.103`), but the config only groups `github-actions`. With `config:recommended`, Renovate does pick up Cargo crates — but each crate gets its own PR, producing review noise (~25 dependencies). Grouping related crates (the Axum/tower stack, the time crates, the AI/HTTP clients) and isolating semver-sensitive ones (SQLx) into deliberate PRs reduces churn and makes the SQLx major-version decision explicit rather than buried in a batch.

## Current state

**`renovate.json`** (entire file):
```json
{
  "$schema": "https://docs.renovatebot.com/renovate-schema.json",
  "extends": [
    "config:recommended"
  ],
  "packageRules": [
    {
      "matchManagers": ["github-actions"],
      "groupName": "github-actions"
    }
  ]
}
```

`config:recommended` already enables the `cargo` manager — so Cargo updates DO flow, just ungrouped. This plan adds grouping rules; it does not "enable" Cargo from scratch.

The dependencies (from `Cargo.toml`): `ammonia`, `anyhow`, `async-trait`, `axum`, `clap`, `comrak`, `dotenvy`, `form_urlencoded`, `futures-util`, `governor`, `http`, `reqwest`, `rmcp`, `schemars`, `serde`, `serde_json`, `sha2`, `subtle`, `syntect` (being removed in plan 035), `thiserror`, `time`, `tokio`, `tower`, `tower-http`, `tracing`, `tracing-subscriber`, `unicode-normalization`, `url`, `urlencoding`, `uuid`, `sqlx`.

## Commands you will need

| Purpose      | Command                                            | Expected on success |
|--------------|---------------------------------------------------|---------------------|
| Validate JSON | `python3 -m json.tool renovate.json > /dev/null` | exit 0 (valid JSON) |

(Renovate config cannot be fully validated locally without the Renovate CLI; JSON validity is the offline gate. The config takes effect on the next Renovate run.)

## Scope

**In scope** (only file):
- `renovate.json`

**Out of scope**:
- `Cargo.toml`, `Cargo.lock` — do not touch
- `.github/workflows/` — Renovate runs as a hosted app/bot, not a workflow here

## Git workflow

- Branch: `advisor/041-renovate-cargo`
- Commit: `chore(renovate): group Cargo crate updates to reduce PR noise`

## Steps

### Step 1: Add Cargo grouping packageRules

Replace the `packageRules` array in `renovate.json` with the following (keep the existing github-actions rule, add the Cargo rules):

```json
  "packageRules": [
    {
      "matchManagers": ["github-actions"],
      "groupName": "github-actions"
    },
    {
      "matchManagers": ["cargo"],
      "matchPackageNames": ["axum", "tower", "tower-http", "http"],
      "groupName": "axum stack"
    },
    {
      "matchManagers": ["cargo"],
      "matchPackageNames": ["tokio", "futures-util", "async-trait"],
      "groupName": "async runtime"
    },
    {
      "matchManagers": ["cargo"],
      "matchPackageNames": ["reqwest", "rmcp", "schemars"],
      "groupName": "http and mcp clients"
    },
    {
      "matchManagers": ["cargo"],
      "matchPackageNames": ["serde", "serde_json", "time", "uuid"],
      "groupName": "serde and core types"
    },
    {
      "matchManagers": ["cargo"],
      "matchPackageNames": ["tracing", "tracing-subscriber"],
      "groupName": "tracing"
    },
    {
      "matchManagers": ["cargo"],
      "matchPackageNames": ["sqlx"],
      "groupName": "sqlx (review major bumps carefully)"
    },
    {
      "matchManagers": ["cargo"],
      "matchUpdateTypes": ["minor", "patch"],
      "groupName": "cargo minor and patch"
    }
  ]
```

Rationale for the executor: the last rule sweeps any remaining crate's minor/patch updates into one PR; the specific rules above pull named crate families into focused PRs; SQLx is isolated because a major SQLx bump can require query/migration changes.

**Verify**: `python3 -m json.tool renovate.json > /dev/null` → exit 0

### Step 2: Confirm JSON is valid and schema reference intact

Confirm the `$schema` and `extends` keys are unchanged, and the file is valid JSON.

**Verify**: `python3 -m json.tool renovate.json` prints the formatted config with no error.

## Test plan

No code tests. The config is validated as JSON locally; its behavioral effect appears on the next Renovate run (creating grouped PRs instead of individual ones).

## Done criteria

- [ ] `renovate.json` is valid JSON (`python3 -m json.tool renovate.json` succeeds)
- [ ] `packageRules` includes `"matchManagers": ["cargo"]` entries
- [ ] The existing `github-actions` rule is preserved
- [ ] No other files modified
- [ ] `plans/README.md` status row updated

## STOP conditions

- The repo uses a `.github/renovate.json` or `renovate.json5` instead, or the config has moved. Find the active config and apply there; report the location.
- `Cargo.toml` lists a workspace with multiple members (it does not at `0819727` — it is a single crate). If it has become a workspace, the package names may need `matchPackageNames` adjustments.

## Maintenance notes

- When `syntect` is removed (plan 035) it drops out naturally — no Renovate rule references it.
- When adding a new dependency family, add a grouping rule so it does not fall into the generic minor/patch bucket if you want it isolated.
- If Renovate PR volume is still high, consider `"schedule"` (e.g. weekly) and `"prConcurrentLimit"` in the root config.
