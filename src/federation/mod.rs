//! Federation: W3C Webmention support, security-first (card T11, P3).
//!
//! Inkwell can RECEIVE Webmentions (`POST /webmention`) and, when explicitly
//! opted in (`INKWELL_WEBMENTION_SEND=true`), SEND them when a published note
//! links out. Both directions share one ironclad rule: **every** outbound HTTP
//! request goes through the single SSRF-hardened fetch helper in [`fetch`], which
//! enforces an http(s) scheme allowlist, rejects private/loopback/link-local/
//! unique-local/cloud-metadata addresses, pins the connection to validated IPs,
//! follows redirects manually with per-hop re-validation, and caps body size and
//! timeout. Send is fully inert unless the flag is on.
//!
//! Layering:
//!   - [`ssrf`]  — pure, network-free IP/host classifier (unit-tested directly).
//!   - [`fetch`] — the one guarded fetch all federation HTTP flows through.
//!   - [`webmention`] — receive-verification + send-discovery built on `fetch`.

pub mod fetch;
pub mod ssrf;
pub mod webmention;
