# ADR 0008: First-Class Authoring CLI

Status: accepted (Option A)

> Accepted (Option A) and implemented in [CYP-38](/CYP/issues/CYP-38): the
> `inkwell` binary gained an `author` subcommand group (`new`, `push`,
> `publish`, `unpublish`) that speaks the existing authenticated HTTP write API.
> See the README "Authoring" section for the workflow.

## Context

Inkwell already exposes authenticated write routes for create, update, delete, publish, and unpublish, but authoring still means hand-writing JSON and issuing raw HTTP requests. The Rust binary only supports `serve` and `db` subcommands today, so there is no supported human authoring surface even though the backend contract exists. We need a first slice that gives authors a practical Markdown workflow without coupling this decision to a full admin UI.

## Options

### Option A: extend the existing `inkwell` binary

Add authoring subcommands such as `inkwell author new`, `inkwell author push`, and `inkwell author publish` to the current Rust binary.

Pros:

- reuses the existing release artifact and Rust toolchain;
- can share config loading with `Config::from_env` and keep auth behavior aligned with the server;
- keeps local automation simple because the same binary can migrate, serve, and author.

Cons:

- mixes server-operator concerns and author workflows into one command tree;
- makes future packaging harder if authors should install a lightweight client without database or server concerns;
- risks forcing client-side UX choices into the server crate layout too early.

### Option B: build a separate client CLI

Ship a dedicated authoring client that speaks the existing HTTP API and treats the server as a remote publishing target.

Pros:

- keeps author ergonomics isolated from server runtime concerns;
- makes it easier to distribute a client-focused tool later, including different release cadence or language choice;
- creates a clean boundary for future features such as draft sync, media upload, or scoped author tokens.

Cons:

- introduces packaging and release overhead sooner;
- duplicates some configuration and auth plumbing that already exists in the server binary;
- is heavier than needed for the first validated slice.

### Option C: defer to a web admin UI

Skip CLI work and invest next in a browser-based authoring surface.

Pros:

- lowers the barrier for non-technical authors;
- avoids local tooling install and shell experience requirements;
- aligns naturally with future media management and richer editorial workflows.

Cons:

- requires more product and UI design before even basic authoring ships;
- expands scope into sessions, browser auth, and interface states;
- leaves the existing API without any practical first-party author workflow in the near term.

## Decision

Take option A for the first slice: extend the existing `inkwell` binary with a narrowly scoped authoring workflow, but keep the command surface and data model explicitly client-oriented so it can be split into a separate CLI later if needed.

The immediate need is not a general admin surface. It is a supported way to author Markdown files locally and publish them through the existing authenticated HTTP API. Reusing the current binary gets that path in front of users fastest while preserving the option to extract the author commands into a standalone client once the workflow proves itself.

## Recommended first slice

Implement a minimal round-trip around local Markdown files with front matter:

- `inkwell author init` writes a starter document template;
- `inkwell author push <path>` parses front matter plus body Markdown, then creates or updates a remote draft through the existing write API;
- `inkwell author publish <path-or-slug>` publishes an already-pushed draft;
- `inkwell author pull <slug>` can wait for a later plan unless draft reconciliation becomes immediately necessary.

This keeps the first implementation small: one file format, one auth model, and direct reuse of the current HTTP contract. It also avoids inventing local draft persistence before we know authors need it.

## File format

Use a single Markdown file with YAML front matter followed by the body:

```md
---
title: Example title
slug: example-title
tags:
  - rust
  - notes
status: draft
---

# Heading

Body Markdown lives here.
```

Rules for the first slice:

- `title` required;
- `slug` optional on create and defaults to the server slugification behavior when omitted;
- `tags` optional;
- `status` accepted as advisory metadata, but publish still happens through an explicit command so local file writes do not accidentally publish content;
- body content maps directly to `bodyMarkdown`.

## Authentication handling

Do not require authors to paste secrets directly into command arguments. For the first slice:

- prefer `INKWELL_API_KEY` from the environment or an interactive prompt when a TTY is available;
- allow a config file path later only if multiple targets become necessary;
- never document `--api-key <value>` as the primary path because it leaks into shell history and process lists;
- permit `stdin` secret input as a non-interactive fallback if interactive prompting is needed in CI-style workflows.

The CLI should also require an explicit server base URL so authoring does not silently target the wrong deployment.

## Follow-up plans

After the first slice, plan follow-up work separately for:

- scoped author tokens and audit logging so the CLI does not depend forever on the shared site-wide API key;
- pull/reconcile behavior for editing existing drafts across machines;
- richer validation and preview flows;
- media and asset upload;
- extraction into a standalone client binary if author usage diverges from server-operator workflows.

## Open questions

- Should the first slice support multiple named remote targets, or is a single explicit base URL enough?
- Do we want local preview to remain out of scope until there is evidence authors need it?
- When scoped tokens land, should publish require a stronger permission than draft create/update?
