# v0.1 Release Checklist

A short, repeatable checklist for cutting an Inkwell release. v0.1 is the first
tagged release: a self-hostable, API-first Markdown publishing core.

## Scope of v0.1

- [ ] Document CRUD over the JSON HTTP API (`/documents`, `/documents/:slug`).
- [ ] Markdown → sanitized HTML rendering pipeline.
- [ ] PostgreSQL persistence with migrations.
- [ ] Public web frontend (index + per-document reading pages).
- [ ] Runnable server entrypoint (`npm start`) configured via environment.

## Pre-release verification

- [ ] `npm ci` installs cleanly from the lockfile.
- [ ] `npm run ci` is green (lint + format check + typecheck + tests + build).
- [ ] Fresh-clone smoke test: follow the README ["Run Inkwell"](../README.md#run-inkwell)
      steps end to end and confirm you can publish, view, update, and delete a
      document with nothing but the README.
- [ ] `npm run db:migrate` applies cleanly against an empty database, and
      `npm run db:rollback` reverses the latest migration.
- [ ] No secrets, credentials, or real connection strings committed (grep the
      diff and tracked files).

## Documentation

- [ ] README product overview, setup, environment variables, API endpoint
      table, and publish walkthrough are accurate against the current code.
- [ ] All ADRs in [`docs/adr/`](adr/) reflect the shipped design.
- [ ] `LICENSE` present and correct (MIT).

## Versioning & tagging

- [ ] `package.json` `version` is `0.1.0`.
- [ ] Changes since the last tag are summarized (commit log or release notes).
- [ ] Create an annotated git tag `v0.1.0` on the release commit.

## Post-release

- [ ] Confirm the tag is pushed and the release is visible.
- [ ] Open follow-up issues for known gaps deferred past v0.1.

---

When all boxes are checked, v0.1 is ready to tag and announce.
