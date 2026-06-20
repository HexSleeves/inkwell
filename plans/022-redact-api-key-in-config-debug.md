# Plan 022: Stop `Config`'s `Debug` from exposing the API key

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c7b0a46..HEAD -- src/config.rs`
> If `src/config.rs` changed since this plan was written, compare the "Current
> state" excerpt against the live code before proceeding; on a mismatch, treat
> it as a STOP condition.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `c7b0a46`, 2026-06-20

## Why this matters

`Config` derives `Debug` and holds `api_key: Option<String>` — the single shared
write credential for the whole service. The derived `Debug` prints the key
verbatim. Nothing logs the whole `Config` today, but any future
`tracing::debug!("{config:?}")`, a `.expect(&format!("{config:?}"))`, or a
panic that formats a struct transitively containing `Config` would write the
live key into logs. A leaked god-key is full write compromise and the only
recovery is rotation + redeploy. Redacting the field in `Debug` is a cheap,
permanent guardrail so the secret cannot leak through diagnostics by accident.

## Current state

`src/config.rs:1-10`:

```rust
use anyhow::{Result, anyhow};

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub site_url: Option<String>,
}
```

`api_key` is set from the `INKWELL_API_KEY` env var in `Config::from_env`
(trimmed, empty filtered to `None`). The struct is held in `AppState`
(`src/http/mod.rs` / `src/http/router.rs`) and used by the auth layer.

Conventions: this is plain Rust; no secrets crate is in use (`Cargo.toml` has no
`secrecy`/`zeroize`). The fix should stay dependency-free — a hand-written
`Debug` impl.

## Commands you will need

| Purpose   | Command                                                          | Expected on success |
|-----------|------------------------------------------------------------------|---------------------|
| Format    | `cargo fmt --check`                                              | exit 0              |
| Lint      | `cargo clippy --all-targets --all-features -- -D warnings`       | exit 0, no warnings |
| Tests     | `cargo test --all`                                               | all pass            |

## Scope

**In scope** (the only file you should modify):
- `src/config.rs` — replace the derived `Debug` with a redacting manual impl.

**Out of scope** (do NOT touch):
- `database_url` — it can contain a password in the DSN; redact it too if you
  print it, or omit it from the manual `Debug` output entirely.
- The auth layer, `AppState`, and `from_env` logic — no behavior change.
- Adding any new dependency (no `secrecy`/`zeroize`).

## Git workflow

- Branch: `advisor/022-redact-api-key-in-config-debug`
- Commit message style: conventional commits, e.g.
  `fix(config): redact secrets in Config Debug impl`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Replace derived `Debug` with a redacting impl

In `src/config.rs`, remove `Debug` from the `#[derive(...)]` (keep `Clone`) and
add a manual impl that never prints the secret values. Redact both `api_key`
(presence only) and `database_url` (it may embed a password):

```rust
#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
    pub site_url: Option<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("database_url", &"<redacted>")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("site_url", &self.site_url)
            .finish()
    }
}
```

This keeps `Config: Debug` (so `AppState` and any struct containing `Config` that
derives `Debug` still compile) while printing `Some("<redacted>")` / `None` for
the key and `"<redacted>"` for the DSN.

**Verify**: `cargo build` → exit 0; `cargo clippy --all-targets --all-features -- -D warnings` → exit 0.

### Step 2: Add a test that the key never appears in Debug output

Add a unit test (a `#[cfg(test)] mod tests` block at the bottom of
`src/config.rs`) that builds a `Config` with a sentinel `api_key` and asserts the
sentinel does not appear in `format!("{config:?}")`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_api_key() {
        let config = Config {
            database_url: "postgres://user:supersecret@localhost/db".to_string(),
            host: "0.0.0.0".to_string(),
            port: 3000,
            api_key: Some("sentinel-key-value".to_string()),
            site_url: None,
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("sentinel-key-value"));
        assert!(!rendered.contains("supersecret"));
        assert!(rendered.contains("<redacted>"));
    }
}
```

**Verify**: `cargo test --all` (or `cargo test config`) → the new test passes.

### Step 3: Full check

**Verify**:
- `cargo fmt --check` → exit 0
- `cargo clippy --all-targets --all-features -- -D warnings` → exit 0
- `cargo test --all` → all pass

## Test plan

- New unit test `debug_does_not_leak_api_key` in `src/config.rs`: asserts the
  api_key value and the DSN password are absent from `Debug` output and that
  `<redacted>` is present.
- Verification: `cargo test --all` → all pass, 1 new test.

## Done criteria

ALL must hold:

- [ ] `src/config.rs` no longer derives `Debug` for `Config`; a manual
      `impl std::fmt::Debug for Config` exists
- [ ] `cargo test --all` passes including `debug_does_not_leak_api_key`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] Only `src/config.rs` is modified (`git status`)
- [ ] `plans/README.md` status row for 022 updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:

- Removing the derived `Debug` causes a compile error elsewhere that the manual
  impl does not resolve (means something relies on a field-by-field derived
  format — report where).
- `src/config.rs` does not match the "Current state" excerpt (drift).

## Maintenance notes

- If new secret-bearing fields are added to `Config`, redact them in the manual
  `Debug` impl too — a derived impl would silently re-expose them, which is why
  this plan replaces the derive rather than annotating fields.
- Pairs conceptually with the scoped-token direction (ADR added by former plan
  015): once per-author tokens exist, the same redaction discipline applies to
  any token-bearing struct.
- A reviewer should confirm no `tracing`/`println` site prints the key directly
  (separate from the struct `Debug`).
