# ADR 0015 — Rejected credentials on public read routes

**Status:** Accepted (rule stated; header-path change scheduled post-`v0.2.0`)
**Date:** 2026-07-25
**Context ticket:** CYP-52 (observation from the CYP-50 pre-release QA sweep)
**Supersedes nothing. Refines:** [ADR 0009](0009-scoped-author-tokens.md), [ADR 0010](0010-browser-login.md)

## Context

`http::auth::authenticate` returns `Option<Principal>`. That single `None` is
used for two materially different situations:

1. **No credential was presented.** An anonymous reader hitting `GET /`.
2. **A credential was presented and we rejected it.** A revoked token, an
   unknown prefix, a hash mismatch, a duplicated or non-ASCII `x-api-key`
   header, or an expired session cookie.

On mutating and admin routes the distinction does not matter: `require_principal`
maps `None` to `401` either way. On **public read** routes (`GET /documents`,
`GET /:slug`, feeds, search, graph, `/ask`) `resolve_visibility` maps `None` to
`Visibility::Public`, so case 2 silently succeeds with `200` and public-only
content.

Measured against a running instance with a freshly revoked `read,write` token:

| Probe | Result |
|---|---|
| `POST /documents` | `401` |
| `POST /media` | `401` |
| `GET /admin/tokens` | `401` |
| `POST /auth/login` | `401` |
| `GET /documents` | `200`, 0 drafts visible |

No privilege leaks — revocation removes every privilege. The only open question
was whether a caller whose credential we rejected should be **told** (`401`) or
**silently downgraded** to anonymous (`200`).

## Decision

**The credential channel decides.** Rejection is reported on the channel a client
only ever populates deliberately, and absorbed on the channel a browser
populates automatically.

| Channel | Presented and rejected | Rationale |
|---|---|---|
| `X-Api-Key` header | **`401 Unauthorized`** | Only ever set on purpose — by the `inkwell author` CLI, the MCP server, or a script. Nothing sets this header by accident: the name is Inkwell-specific and no browser, proxy, or extension emits it. Silence here hides a real client misconfiguration. |
| `inkwell_session` cookie | **downgrade to anonymous (`200`)** | Sent automatically on every request by any browser that once logged in, including plain reads of public pages. An expired or unknown session cookie must never break the public site for a reader who cannot even see the cookie, let alone clear it. Fail-open-to-public is correct here. |

Two clarifications that fall out of the rule and are part of the decision:

- **A valid credential without the `read` scope stays `200`/public.** A
  write-only token authenticated successfully; it simply sees nothing but
  published documents. That is authorization, not a rejected credential, and it
  is unchanged.
- **A database error during token lookup is not a rejected credential.** Today
  `find_token_by_prefix(...).ok()??` collapses an infrastructure failure into the
  same `None`. Under this ADR it becomes `503 Service Unavailable`, not `401` and
  not `200`.

Out of scope: preview tokens (`GET /documents/:slug/preview?token=…`) already
return `401` on every failure and deliberately never reveal a draft's existence;
the `/metrics` scrape token (`Authorization: Bearer`) already returns `401`.

## Why this and not the alternatives

**Why not keep silent downgrade everywhere?** An author whose token was revoked
— or who typo'd it — currently gets a `200` with their drafts quietly missing.
That reads as "my content is gone", not "my credential is bad", and it makes
credential failures invisible in CLI and MCP-agent logs. RFC 7235 is explicit
that a request carrying invalid credentials warrants `401`.

**Why not `401` everywhere, cookie included?** That would let a stale
`inkwell_session` cookie break the public website for an ordinary reader. The
"fail open so public pages stay reachable" argument in the original report is a
genuinely good argument — it is just an argument about the *ambient* channel, and
this ADR keeps it exactly there.

## Consequences

- **Wire contract change, header path only.** Any client that sends a stale or
  wrong `X-Api-Key` to a read route moves from `200` to `401`. This is a
  behavior change requiring a `CHANGELOG.md` migration note and a
  `docs/COMPATIBILITY.md` entry.
- **Deliberately excluded from `v0.2.0`.** The rule is documented now; the code
  change lands after the `v0.2.0` tag is cut so the release stays behavior-stable.
  Until then, `docs/API.md` and the `http::auth` module docs describe current
  behavior as the shipped rule and this ADR as the accepted target.
- **`authenticate` gains a third state.** `Option<Principal>` cannot express
  "rejected", so the header path needs an explicit outcome type
  (`Authenticated` / `Anonymous` / `Rejected` / `Unavailable`), and
  `resolve_visibility` becomes fallible (`Result<Visibility, AppError>`) across
  its ~12 call sites in `documents`, `search`, `graph`, `ai`, and `preview`.
  `require_principal` keeps its current behavior: both `Anonymous` and
  `Rejected` are `401`.
- **No new privilege surface.** Every case that authenticates today still
  authenticates, with identical scopes. The change only makes an existing
  rejection louder on one channel.
