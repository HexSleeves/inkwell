# ADR 0009: Scoped Author Tokens and Write Audit

Status: accepted (implemented in v0.2.0 — see docs/RELEASE-NOTES-v0.2.0.md)

## Context

Inkwell currently protects every write route with a single shared `INKWELL_API_KEY`. That behavior is simple and fails closed when the key is missing, but it treats every caller as the same all-powerful actor. There is no per-author identity, no scoped permissions, no targeted revocation, no document ownership, and no durable audit trail for create, update, delete, publish, or unpublish actions.

This leaves two gaps:

- product: multi-author publishing and human-oriented tooling cannot attribute work to specific people;
- security and operations: a leaked key requires whole-site rotation and gives no evidence about what it touched.

The next auth model should preserve the operational safety of the existing shared key while introducing author identity, scoped bearer tokens, ownership, and write audit records.

## Options

### Option A: keep the shared site key and add only logging

Pros:

- smallest schema change;
- keeps current operator workflow untouched;
- avoids ownership decisions for existing documents.

Cons:

- still no per-author revocation or scopes;
- no clean path to multi-author tooling;
- audit records remain weak because every actor is still the same shared principal.

### Option B: add author records plus scoped API tokens, while keeping the shared key as a bootstrap/admin fallback

Pros:

- supports named authors, scoped access, targeted revocation, and audit attribution;
- preserves a recovery path when the database token surface is unavailable or an operator needs initial setup;
- aligns with future CLI or admin tooling without forcing browser sessions now.

Cons:

- requires schema work across authors, tokens, documents, and audit tables;
- introduces ownership semantics that the current data model does not have;
- requires careful migration for existing documents and operators.

### Option C: skip bearer tokens and move directly to full user accounts plus interactive sessions

Pros:

- strongest long-term story for browser admin features;
- can unify human identity and authorization under one model.

Cons:

- much larger product and implementation scope than the current need;
- delays practical author tooling for CLI or automation;
- still needs service-to-service or CLI credentials for non-browser flows.

## Decision

Take option B. Add first-class authors and scoped API tokens backed by database state, but retain the current shared `INKWELL_API_KEY` as a bootstrap/admin fallback until the new tooling is proven.

This keeps the near-term goal narrow: make write actions attributable and revocable without blocking on full user-account UX. The shared key remains useful for initial provisioning, emergency recovery, and environments that have not yet minted author tokens, but it should become the exception rather than the everyday authoring credential.

## Proposed model

### Bootstrap/admin fallback

Keep the current shared-key behavior with a narrower interpretation:

- when `INKWELL_API_KEY` is configured and matches, treat the request as a synthetic `bootstrap-admin` actor;
- bootstrap-admin bypasses document ownership checks and has implicit `admin` scope;
- bootstrap-admin actions must still emit audit events so operators can distinguish fallback use from normal author-token use;
- documentation should position the shared key as a setup and break-glass credential, not the default authoring path.

### Authors table

Add a durable `authors` table for principals that can own documents or tokens.

Suggested fields:

- `id` UUID primary key;
- `handle` text unique, stable operator-facing identifier;
- `display_name` text;
- `role` text constrained to values such as `author` and `admin`;
- `created_at` timestamptz not null;
- `disabled_at` timestamptz nullable for soft deactivation.

Notes:

- one author is the canonical owner of each document in the first slice;
- admin is a capability class for people, separate from token scopes that can be rotated independently;
- future profile fields can stay out of scope until a public author page or richer UI needs them.

### API tokens table

Add a dedicated `api_tokens` table owned by `authors`.

Suggested fields:

- `id` UUID primary key;
- `author_id` UUID references `authors(id)`;
- `label` text for operator-facing identification such as "laptop" or "ci";
- `token_prefix` text not null for partial display and support workflows;
- `secret_hash` bytea or text not null;
- `scopes` text[] not null;
- `created_at` timestamptz not null;
- `last_used_at` timestamptz nullable;
- `last_used_ip` inet nullable if the deployment wants IP capture;
- `last_used_user_agent` text nullable;
- `revoked_at` timestamptz nullable;
- `revoked_reason` text nullable.

Token format should separate lookup identity from bearer secret, for example `ikw_<token_id>_<secret>`.

Validation flow:

1. parse `token_id` from the presented token;
2. fetch the token row by `id`;
3. reject revoked or disabled principals before authorization;
4. hash the presented secret;
5. compare against `secret_hash` in constant time;
6. update last-used metadata asynchronously or in the same transaction, depending on operational simplicity.

### No plaintext token storage

The database must never store recoverable bearer secrets.

Requirements:

- generate high-entropy random secrets;
- show the full token only once at creation time;
- store only `token_prefix`, `token_id`, and `secret_hash`;
- never log full presented tokens;
- redact authorization material from tracing and error output;
- operator UX should explicitly warn that a lost token must be replaced, not recovered.

Because token secrets are high-entropy random values rather than user-chosen passwords, a single cryptographic hash of the secret is sufficient for the first slice. A server-side pepper is an optional hardening layer but should not block the design.

### Scopes

Support small, explicit scopes rather than a boolean "can write".

Initial scope set:

- `documents:write` for create, update, and delete on owned drafts;
- `documents:publish` for publish and unpublish on owned documents;
- `admin` for cross-author overrides plus token and author management.

Rules:

- `admin` implies all document scopes;
- a token may hold more than one scope;
- publish should require `documents:publish` even if `documents:write` is present;
- future scopes such as media upload or preview sharing should be additive, not encoded into roles.

### Draft visibility and ownership

Draft visibility should stay private by default.

First-slice rules:

- unauthenticated callers continue to see only published documents;
- non-admin tokens may read or mutate only documents they own;
- admin tokens and bootstrap-admin may read or mutate any draft;
- each document gets one `owner_author_id` field rather than a collaborator list;
- collaborative editing stays out of scope until there is evidence the product needs shared draft ownership.

This keeps authorization understandable and matches the current single-writer product shape.

### Document ownership migration

Existing documents have no owner metadata, so backfill must be explicit.

Recommended path:

1. create a bootstrap author record such as `system-bootstrap` or an operator-chosen admin author;
2. add nullable `owner_author_id` to `documents`;
3. backfill all existing documents to that bootstrap/admin author;
4. make `owner_author_id` non-null once the backfill is complete;
5. let later tooling reassign ownership document by document if the initial backfill is too coarse.

This chooses administrative correctness over trying to infer history that does not exist.

### Write audit events

Add a separate append-only `write_audit_events` table.

Suggested fields:

- `id` UUID primary key;
- `occurred_at` timestamptz not null;
- `actor_author_id` UUID nullable when the bootstrap key is used;
- `actor_token_id` UUID nullable for bootstrap-admin actions;
- `actor_kind` text not null, such as `author_token` or `bootstrap_admin`;
- `action` text not null, such as `document.create`, `document.update`, `document.delete`, `document.publish`, `document.unpublish`, `token.create`, `token.revoke`;
- `document_id` UUID nullable;
- `document_slug` text nullable snapshot for easier audit review;
- `request_id` text nullable when request correlation exists;
- `metadata` jsonb not null default '{}'::jsonb.

Audit events should capture the actor, action, target document when applicable, and enough context to investigate misuse without storing secrets or whole request bodies.

## Rotation and revocation requirements

Rotation and revocation are first-class requirements, not admin afterthoughts.

Rules:

- a token can be revoked independently without touching other tokens for the same author;
- revoked tokens stop working immediately based on `revoked_at`;
- authors may hold multiple active tokens so operators can rotate one device at a time;
- token creation and revocation must themselves emit audit events;
- bootstrap-admin should remain available for recovery if all scoped tokens are revoked accidentally;
- operator-facing tooling should encourage replace-then-revoke rotation to avoid downtime.

## Follow-up implementation slices

1. schema slice: add `authors`, `api_tokens`, `write_audit_events`, and document ownership columns plus migration/backfill;
2. auth slice: replace shared-key-only request auth with bootstrap fallback plus scoped token lookup and constant-time secret verification;
3. authorization slice: enforce ownership and scope checks across create, update, delete, publish, unpublish, and draft reads;
4. audit slice: emit append-only audit records for all write mutations and token lifecycle events;
5. tooling slice: add bootstrap token minting, token rotation, and author-management commands to the future CLI or admin surface;
6. cleanup slice: narrow documentation so the shared `INKWELL_API_KEY` is clearly described as bootstrap and break-glass only.

## Product questions

- Should document ownership transfer be admin-only in the first release, or can an author delegate a draft to another author?
- Do we need a distinct read scope for private draft review, or is owner-or-admin visibility enough until preview links exist?
- Should token last-used IP and user-agent capture be optional configuration for privacy-sensitive deployments?
- When the first authoring CLI lands, should bootstrap-admin token minting happen through a CLI command, a migration seed path, or both?
