//! Opaque scoped-token format, generation, and hashing (ADR 0009, plan 023,
//! slice 2).
//!
//! A token is `ink_<prefix>_<secret>`:
//! - `prefix` is a public, non-secret lookup handle stored verbatim
//!   (`author_tokens.prefix`, unique) so resolution is a single indexed lookup.
//! - `secret` is the high-entropy part; it is **never** stored. Only a SHA-256
//!   hash of the *whole* token (`ink_<prefix>_<secret>`) is persisted as
//!   `token_hash`, and resolution recomputes that hash and compares it in
//!   constant time.
//!
//! Randomness comes from v4 UUIDs (122 bits each, CSPRNG-backed via `getrandom`),
//! so no extra dependency is pulled in just to mint tokens.

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Marker that identifies an inkwell scoped token on the wire and in storage.
/// Resolution only touches the database when a presented key starts with this,
/// so anonymous and shared-key requests never pay a token lookup.
pub const TOKEN_PREFIX: &str = "ink_";

/// Hex length of a `prefix` (half a UUID's 32 hex chars). 48 bits is ample for a
/// collision-resistant *handle* — it is not a secret, and the DB `UNIQUE`
/// constraint is the real guarantee; mint retries on the astronomically rare clash.
const PREFIX_HEX_LEN: usize = 12;

/// A freshly minted token. The full [`token`](Self::token) is shown to the
/// operator exactly once at creation; only [`prefix`](Self::prefix) and
/// [`token_hash`](Self::token_hash) are persisted.
#[derive(Debug, Clone)]
pub struct GeneratedToken {
    /// The complete `ink_<prefix>_<secret>` string. Return to the caller once,
    /// then drop it — it is unrecoverable afterwards.
    pub token: String,
    /// Public lookup handle (`author_tokens.prefix`).
    pub prefix: String,
    /// SHA-256 hex of the whole token, persisted as `author_tokens.token_hash`.
    pub token_hash: String,
}

/// Mint a new scoped token with a fresh prefix and secret.
pub fn generate() -> GeneratedToken {
    let prefix = random_hex()[..PREFIX_HEX_LEN].to_string();
    // 64 hex chars (256 bits) of secret, from two independent v4 UUIDs.
    let secret = format!("{}{}", random_hex(), random_hex());
    let token = format!("{TOKEN_PREFIX}{prefix}_{secret}");
    let token_hash = sha256_hex(&token);
    GeneratedToken {
        token,
        prefix,
        token_hash,
    }
}

/// Extract the lookup `prefix` from a presented token, or `None` when the value
/// is not a well-formed `ink_<prefix>_<secret>` (empty prefix or secret fails).
pub fn parse_prefix(token: &str) -> Option<&str> {
    let rest = token.strip_prefix(TOKEN_PREFIX)?;
    let (prefix, secret) = rest.split_once('_')?;
    if prefix.is_empty() || secret.is_empty() {
        return None;
    }
    Some(prefix)
}

/// SHA-256 of `input`, lowercase hex. Used to derive `token_hash`; the stored
/// and recomputed hexes are compared in constant time during resolution.
pub fn sha256_hex(input: &str) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 32 lowercase hex chars from a fresh v4 UUID.
fn random_hex() -> String {
    Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_has_expected_shape() {
        let minted = generate();
        assert!(minted.token.starts_with("ink_"));
        // ink_ + 12 (prefix) + _ + 64 (secret)
        assert_eq!(minted.token.len(), 4 + PREFIX_HEX_LEN + 1 + 64);
        assert_eq!(minted.prefix.len(), PREFIX_HEX_LEN);
        assert_eq!(minted.token_hash.len(), 64);
        assert!(minted.token_hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn parse_prefix_round_trips_the_minted_prefix() {
        let minted = generate();
        assert_eq!(parse_prefix(&minted.token), Some(minted.prefix.as_str()));
    }

    #[test]
    fn parse_prefix_rejects_malformed_tokens() {
        assert_eq!(parse_prefix("nope"), None);
        assert_eq!(parse_prefix("ink_"), None); // no prefix or secret
        assert_eq!(parse_prefix("ink_abc"), None); // no separator -> no secret
        assert_eq!(parse_prefix("ink__secret"), None); // empty prefix
        assert_eq!(parse_prefix("ink_abc_"), None); // empty secret
        assert_eq!(parse_prefix("ink_abc_def"), Some("abc"));
    }

    #[test]
    fn sha256_hex_is_stable_and_distinct() {
        // Known SHA-256("") prefix guards the hex encoding.
        assert!(sha256_hex("").starts_with("e3b0c44298fc1c149afbf4c8996fb924"));
        assert_ne!(sha256_hex("a"), sha256_hex("b"));
    }

    #[test]
    fn each_token_is_unique() {
        let a = generate();
        let b = generate();
        assert_ne!(a.token, b.token);
        assert_ne!(a.prefix, b.prefix);
        assert_ne!(a.token_hash, b.token_hash);
    }
}
