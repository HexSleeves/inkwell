---
name: ai
description: Semantic search, RAG pipeline, embeddings, and the /ask endpoint. Load when working on AI features, Voyage integration, or Anthropic Claude synthesis.
triggers:
  - "embedding"
  - "semantic"
  - "RAG"
  - "ask"
  - "voyage"
  - "anthropic"
  - "claude"
  - "vector"
  - "chunk"
  - "note_chunks"
  - "pgvector"
edges:
  - target: context/architecture.md
    condition: when understanding how the AI layer fits into the overall system
  - target: context/stack.md
    condition: when checking library versions or provider details
  - target: context/decisions.md
    condition: when understanding why MockEmbedder/MockLlm exist or why keys are optional
  - target: patterns/add-endpoint.md
    condition: when adding a new AI-backed endpoint
last_updated: 2026-06-23
---

# AI / Semantic Layer

## Overview

Two optional providers power semantic features. Both degrade gracefully when keys are absent:

```
note write → chunk_text → Embedder::embed → replace_note_chunks (note_chunks table)
GET /ask  → Embedder::embed(question) → ANN search note_chunks → Llm::answer(question, context)
```

## Provider Traits (`src/ai/mod.rs`)

- **`Embedder`** — `async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>`
  - Real: `VoyageEmbedder` (`src/ai/voyage.rs`) — calls Voyage AI `voyage-3` model; 1024-dim vectors
  - Mock: `MockEmbedder` — SHA-256 hash → deterministic 1024-dim unit vector; CI-safe, no key needed
- **`Llm`** — `async fn answer(&self, question: &str, context_blocks: &[String]) -> anyhow::Result<String>`
  - Real: `ClaudeLlm` (`src/ai/claude.rs`) — calls Anthropic Messages API with `claude-opus-4-8`
  - Mock: `MockLlm` — deterministic canned answer; returns `NO_ANSWER_MARKER` when no context

## Key Constants

- `EMBEDDING_DIMENSIONS = 1024` — must match Voyage `voyage-3` output AND `vector(1024)` in migration 0009; change all three together
- `VOYAGE_MODEL = "voyage-3"` — model name sent to Voyage API
- `MAX_CHUNK_CHARS = 1500` — max chars per chunk before splitting on paragraph boundaries
- `NO_ANSWER_MARKER = "I could not find an answer in the garden."` — sentinel for empty-context refusal; eval suite asserts on it

## Indexing Flow (`ai::index_note`)

Called after every successful create or update (best-effort):
1. `chunk_text(body)` — split on `\n\n` paragraph boundaries, pack up to `MAX_CHUNK_CHARS`; hard-split oversized paragraphs
2. `embedder.embed(&chunks)` — single batch call per note write
3. `replace_note_chunks(pool, note_id, expected_version, &rows)` — version-guarded upsert; a slower older concurrent update is skipped if a newer version already landed

**Vector format**: embeddings bound as Postgres text literal `[v1,v2,...]` cast to `::vector` in SQL — no custom SQLx encoder needed.

## Retrieval Flow (`GET /ask`)

1. `embedder.embed(&[question])` — single vector
2. ANN cosine-distance query against `note_chunks` filtered by `Visibility` (public callers only see published note chunks)
3. Top-K chunks assembled as context blocks
4. `llm.answer(question, &context_blocks)` — Claude synthesis with no-fabricate grounding prompt
5. Returns structured JSON with answer + cited note slugs

## Test/CI Wiring

Use `build_router_with_providers` (not `build_router`) to inject mocks:
```rust
let router = build_router_with_providers(config, pool, Arc::new(MockEmbedder), Some(Arc::new(MockLlm)));
```
`build_router` reads keys from `Config` and returns real providers when set. Never call this in tests.

## Gotchas

- `EMBEDDING_DIMENSIONS`, the Voyage model, and the `vector(1024)` column in migration 0009 must stay in sync. Changing the dimension requires a new migration to recreate the column.
- `index_note` re-derives chunks from `document.body_markdown` on every update (including metadata-only patches). This is intentional: a version bump from a concurrent body edit could leave the semantic index permanently stale if only body edits triggered reindexing.
- Non-finite floats in embeddings are coerced to `0.0` in `vector_to_pg_text` before the Postgres literal is built — Postgres rejects `NaN`/`Infinity` in vector literals.
- The `MockEmbedder` is deterministic but not bijective; two very different texts may hash to similar-looking vectors. Don't use it to assert similarity rankings between unrelated terms.
