//! Semantic layer for the garden (card T10, P3): embeddings + RAG.
//!
//! Two provider traits sit in front of the network so the whole feature stays
//! CI-green without API keys:
//!   - [`Embedder`] turns text into fixed-dimension vectors. The real impl is
//!     [`voyage::VoyageEmbedder`] (Voyage AI); tests and the no-key path use
//!     [`MockEmbedder`], a deterministic hash → vector.
//!   - [`Llm`] synthesizes an answer from retrieved context. The real impl is
//!     [`claude::ClaudeLlm`] (Anthropic Messages API via raw HTTP); tests use
//!     [`MockLlm`], which answers deterministically.
//!
//! Real provider calls are gated behind configured keys: with no key, embedding
//! generation is a logged no-op and `/ask` returns "AI features not configured"
//! rather than 500ing. The deterministic mocks let the eval suite assert
//! retrieval/citation behavior with zero network access.

use std::sync::Arc;

use async_trait::async_trait;

pub mod claude;
pub mod voyage;

/// Embedding dimension. Matches the Voyage model we target (voyage-3 = 1024)
/// and the `vector(1024)` column in migration 0009 — keep all three in sync.
pub const EMBEDDING_DIMENSIONS: usize = 1024;

/// Voyage model used for the real embedder.
pub const VOYAGE_MODEL: &str = "voyage-3";

/// Maximum characters per chunk before splitting. Notes are split on blank-line
/// paragraph boundaries and packed up to this size so each chunk is a coherent
/// unit small enough to embed meaningfully. A small const keeps embedding work
/// bounded per note, mirroring every other bounded surface in the garden.
pub const MAX_CHUNK_CHARS: usize = 1_500;

/// Turns text into fixed-dimension embedding vectors. One call embeds a batch so
/// chunking a note is a single provider round-trip.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed each input string, returning one `EMBEDDING_DIMENSIONS`-length
    /// vector per input, in order. An empty input slice yields an empty result.
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

/// Synthesizes a natural-language answer from a question and retrieved context.
#[async_trait]
pub trait Llm: Send + Sync {
    /// Answer `question` grounded in `context_blocks` (already-retrieved note
    /// excerpts). Implementations must never fabricate citations; when the
    /// context does not support an answer they should say so plainly.
    async fn answer(&self, question: &str, context_blocks: &[String]) -> anyhow::Result<String>;
}

/// Deterministic, network-free embedder. Hashes each input into a fixed-length
/// vector: identical text always yields the same vector, similar text shares
/// structure, and unrelated text is far apart — enough for the eval suite to
/// assert that the right notes are retrieved without any provider.
#[derive(Clone, Debug, Default)]
pub struct MockEmbedder;

#[async_trait]
impl Embedder for MockEmbedder {
    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| mock_embedding(t)).collect())
    }
}

/// Hash `text` into a deterministic, L2-normalized `EMBEDDING_DIMENSIONS`-vector.
///
/// Each lowercased whitespace token contributes to a small set of dimensions
/// (derived from its hash), so notes that share words land near each other under
/// cosine distance while unrelated notes stay far apart. Deterministic across
/// runs and processes — the eval suite depends on it.
pub fn mock_embedding(text: &str) -> Vec<f32> {
    use sha2::{Digest, Sha256};

    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    for token in text.split_whitespace() {
        let token = token.trim_matches(|c: char| !c.is_alphanumeric());
        if token.is_empty() {
            continue;
        }
        let token = token.to_lowercase();
        let digest = Sha256::digest(token.as_bytes());
        // Spread each token across a few dimensions so co-occurring words build
        // an overlapping signature.
        for window in digest.chunks(4) {
            let idx = (u32::from_le_bytes([
                window[0],
                *window.get(1).unwrap_or(&0),
                *window.get(2).unwrap_or(&0),
                *window.get(3).unwrap_or(&0),
            ]) as usize)
                % EMBEDDING_DIMENSIONS;
            // Sign from the high bit keeps the distribution centered.
            let sign = if digest[0] & 1 == 0 { 1.0 } else { -1.0 };
            vector[idx] += sign;
        }
    }
    l2_normalize(&mut vector);
    vector
}

/// Normalize a vector to unit length in place (no-op for the zero vector, which
/// an empty note produces). Cosine distance over normalized vectors is what the
/// retrieval query computes, so normalizing here keeps mock and real embeddings
/// comparable.
fn l2_normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for v in vector.iter_mut() {
            *v /= norm;
        }
    }
}

/// Deterministic, network-free LLM for tests and the eval suite.
///
/// Behavior is intentionally simple and inspectable: if `context_blocks` is
/// empty it refuses cleanly (the "no-answer" path); otherwise it echoes a short
/// answer that cites the supplied context, so the known-answer eval can assert a
/// real answer came back grounded in the retrieved notes.
#[derive(Clone, Debug, Default)]
pub struct MockLlm;

/// Sentinel phrase the mock (and the real Claude prompt) use to signal "the
/// garden does not contain an answer". The eval suite asserts on it.
pub const NO_ANSWER_MARKER: &str = "I could not find an answer in the garden.";

#[async_trait]
impl Llm for MockLlm {
    async fn answer(&self, question: &str, context_blocks: &[String]) -> anyhow::Result<String> {
        if context_blocks.is_empty() {
            return Ok(NO_ANSWER_MARKER.to_string());
        }
        // Deterministic, grounded-looking answer: restate the question and quote
        // a snippet of the first context block so tests can assert the answer is
        // derived from the retrieved notes, not hallucinated.
        let snippet: String = context_blocks[0].chars().take(120).collect();
        Ok(format!(
            "Based on the garden: {} (re: \"{}\")",
            snippet.trim(),
            question.trim()
        ))
    }
}

/// Build the embedder for a config: the real Voyage embedder when a key is set,
/// otherwise the deterministic mock. Returned behind an `Arc` so it can be
/// shared across requests in [`AppState`](crate::http::AppState).
pub fn build_embedder(config: &crate::config::Config) -> Arc<dyn Embedder> {
    match config.voyage_api_key.as_deref() {
        Some(key) => match voyage::VoyageEmbedder::new(key.to_string()) {
            Ok(embedder) => Arc::new(embedder),
            Err(error) => {
                // Never print the key; file:line + type only via Debug.
                tracing::warn!(%error, "VoyageEmbedder init failed; using mock embedder");
                Arc::new(MockEmbedder)
            }
        },
        None => {
            tracing::info!("VOYAGE_API_KEY not set; using deterministic mock embedder");
            Arc::new(MockEmbedder)
        }
    }
}

/// Build the LLM for a config: the real Claude client when a key is set,
/// otherwise `None` (so `/ask` reports "AI features not configured" rather than
/// silently substituting a mock in production).
pub fn build_llm(config: &crate::config::Config) -> Option<Arc<dyn Llm>> {
    match config.anthropic_api_key.as_deref() {
        Some(key) => match claude::ClaudeLlm::new(key.to_string(), config.llm_model.clone()) {
            Ok(llm) => Some(Arc::new(llm)),
            Err(error) => {
                tracing::warn!(%error, "ClaudeLlm init failed; /ask will report AI not configured");
                None
            }
        },
        None => None,
    }
}

/// Chunk `body`, embed each chunk via `embedder`, and replace the note's stored
/// chunks — the best-effort indexing step the write path runs after a
/// create/update (mirrors `persist_source_edges`).
///
/// A body that chunks to nothing clears the note's embeddings. A provider error
/// propagates so the caller can log-and-continue without failing the write; the
/// `MockEmbedder` never errors, so the no-key path always indexes.
///
/// `expected_version` is the `documents.version` the caller just wrote. It is
/// threaded into the replace so a slower OLDER concurrent update can't overwrite
/// a newer revision's embeddings (the replace is skipped when the row's current
/// version no longer matches).
pub async fn index_note(
    pool: &sqlx::PgPool,
    embedder: &dyn Embedder,
    note_id: uuid::Uuid,
    expected_version: i64,
    body: &str,
) -> anyhow::Result<()> {
    use crate::db::chunks::{NewChunk, replace_note_chunks};

    let chunks = chunk_text(body);
    if chunks.is_empty() {
        replace_note_chunks(pool, note_id, expected_version, &[]).await?;
        return Ok(());
    }
    let embeddings = embedder.embed(&chunks).await?;
    // `zip` would silently truncate on a length mismatch, persisting a partial
    // chunk set. Fail fast so indexing stays all-or-nothing.
    if embeddings.len() != chunks.len() {
        anyhow::bail!(
            "embedder returned {} embeddings for {} chunks",
            embeddings.len(),
            chunks.len()
        );
    }
    let rows: Vec<NewChunk> = chunks
        .into_iter()
        .zip(embeddings)
        .enumerate()
        .map(|(i, (content, embedding))| NewChunk {
            chunk_index: i as i32,
            content,
            embedding,
        })
        .collect();
    replace_note_chunks(pool, note_id, expected_version, &rows).await?;
    Ok(())
}

/// Split `text` into chunks at paragraph (blank-line) boundaries, packing
/// paragraphs together up to [`MAX_CHUNK_CHARS`]. A single paragraph longer than
/// the cap is hard-split on character boundaries so no chunk exceeds the cap.
/// Whitespace-only input yields no chunks.
pub fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    let flush = |current: &mut String, chunks: &mut Vec<String>| {
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_string());
        }
        current.clear();
    };

    for paragraph in text.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        // A paragraph that alone exceeds the cap is hard-split; flush whatever is
        // pending first so order is preserved.
        if paragraph.chars().count() > MAX_CHUNK_CHARS {
            flush(&mut current, &mut chunks);
            for piece in hard_split(paragraph, MAX_CHUNK_CHARS) {
                chunks.push(piece);
            }
            continue;
        }
        // Packing this paragraph would overflow the current chunk: flush first.
        if current.chars().count() + paragraph.chars().count() + 2 > MAX_CHUNK_CHARS {
            flush(&mut current, &mut chunks);
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(paragraph);
    }
    flush(&mut current, &mut chunks);
    chunks
}

/// Hard-split a single oversized string into `<= max_chars` pieces on character
/// boundaries (so multi-byte UTF-8 is never split mid-codepoint).
fn hard_split(text: &str, max_chars: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    for ch in text.chars() {
        if piece.chars().count() >= max_chars {
            pieces.push(std::mem::take(&mut piece));
        }
        piece.push(ch);
    }
    if !piece.trim().is_empty() {
        pieces.push(piece);
    }
    pieces
}

/// Format an embedding as the pgvector text input literal, e.g. `[0.1,0.2,...]`.
///
/// Embeddings are bound as text and cast to `::vector` in SQL, so the feature
/// needs no version-coupled sqlx encoder for the `vector` type — the cast does
/// the work at the database. Non-finite values would make Postgres reject the
/// literal, so they are coerced to `0`.
pub fn vector_to_pg_text(vector: &[f32]) -> String {
    let mut out = String::with_capacity(vector.len() * 8 + 2);
    out.push('[');
    for (i, v) in vector.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let v = if v.is_finite() { *v } else { 0.0 };
        // `{}` on f32 emits a round-trippable shortest form Postgres accepts.
        use std::fmt::Write;
        let _ = write!(out, "{v}");
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_embedding_is_deterministic_and_dimensioned() {
        let a = mock_embedding("the quick brown fox");
        let b = mock_embedding("the quick brown fox");
        assert_eq!(a.len(), EMBEDDING_DIMENSIONS);
        assert_eq!(a, b, "same text must embed identically");
    }

    #[test]
    fn mock_embedding_is_normalized_for_nonempty_text() {
        let v = mock_embedding("gardens grow over time");
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "expected unit length, got {norm}"
        );
    }

    #[test]
    fn mock_embedding_empty_text_is_zero_vector() {
        let v = mock_embedding("   ");
        assert_eq!(v.len(), EMBEDDING_DIMENSIONS);
        assert!(v.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn related_text_is_closer_than_unrelated() {
        // Cosine similarity = dot product for unit vectors.
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let base = mock_embedding("postgres full text search indexing");
        let related = mock_embedding("postgres indexing for search performance");
        let unrelated = mock_embedding("baking sourdough bread at home");
        assert!(
            dot(&base, &related) > dot(&base, &unrelated),
            "related text should be more similar than unrelated"
        );
    }

    #[test]
    fn chunk_text_splits_on_paragraphs_and_drops_blanks() {
        let chunks = chunk_text("First para.\n\nSecond para.\n\n\n   \n\nThird.");
        assert_eq!(chunks, vec!["First para.\n\nSecond para.\n\nThird."]);
    }

    #[test]
    fn chunk_text_packs_until_cap_then_splits() {
        let para = "a".repeat(MAX_CHUNK_CHARS - 10);
        let text = format!("{para}\n\n{para}");
        let chunks = chunk_text(&text);
        assert_eq!(chunks.len(), 2, "two near-cap paragraphs do not fit in one");
        for c in &chunks {
            assert!(c.chars().count() <= MAX_CHUNK_CHARS);
        }
    }

    #[test]
    fn chunk_text_hard_splits_oversized_paragraph() {
        let para = "b".repeat(MAX_CHUNK_CHARS * 2 + 50);
        let chunks = chunk_text(&para);
        assert!(chunks.len() >= 3);
        for c in &chunks {
            assert!(c.chars().count() <= MAX_CHUNK_CHARS);
        }
    }

    #[test]
    fn chunk_text_empty_input_yields_no_chunks() {
        assert!(chunk_text("   \n\n  ").is_empty());
    }

    #[test]
    fn vector_to_pg_text_formats_bracketed_csv() {
        assert_eq!(vector_to_pg_text(&[1.0, 2.5, -3.0]), "[1,2.5,-3]");
        assert_eq!(vector_to_pg_text(&[]), "[]");
        // Non-finite coerced to 0 so Postgres accepts the literal.
        assert_eq!(vector_to_pg_text(&[f32::NAN, f32::INFINITY]), "[0,0]");
    }

    #[tokio::test]
    async fn mock_llm_refuses_without_context() {
        let answer = MockLlm.answer("anything?", &[]).await.unwrap();
        assert_eq!(answer, NO_ANSWER_MARKER);
    }

    #[tokio::test]
    async fn mock_llm_grounds_answer_in_context() {
        let answer = MockLlm
            .answer("what is x?", &["X is a thing in the garden.".to_string()])
            .await
            .unwrap();
        assert!(answer.contains("X is a thing"));
    }
}
