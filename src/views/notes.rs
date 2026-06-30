use crate::domain::document::DocumentSummary;

use super::layout::{HeadMeta, SiteMeta, date_line, escape_html, render_page, render_tag_chips};

/// Render the `/notes` page: a complete, single-page index of every published
/// note. Each note is a compact row (title, date, tags) carrying `data-title`
/// and `data-date` attributes so the only client JS — a nonce'd inline script —
/// can filter (by title substring) and sort (Newest / A–Z) entirely in the
/// browser, with no server round-trip. An empty set degrades to a plain "no
/// notes" message with no toolbar, list, or script.
///
/// `total` is the published-note count; when it exceeds the rendered rows the
/// list was capped, so a truncation note is shown. The page is pinned to public
/// (published-only) visibility by its caller, exactly like the Dashboard feed.
pub fn render_notes_index_page(
    docs: &[DocumentSummary],
    total: i64,
    csp_nonce: &str,
    site: &SiteMeta<'_>,
) -> String {
    let main = if docs.is_empty() {
        r#"<div class="notes-page-header">
          <h1>Notes <span class="accent-dot">·</span> <span class="accent-title">Every Note</span></h1>
          <p class="notes-subtitle">A complete index of the garden.</p>
        </div>
        <p class="empty">No notes yet.</p>"#
            .to_string()
    } else {
        let rows = docs
            .iter()
            .map(|doc| {
                // `data-title` is lowercased for case-insensitive filtering and
                // A–Z sorting; `data-date` is the sortable serialized timestamp
                // (lexicographic order == chronological order). Both are HTML-
                // escaped because they land inside attribute values.
                let data_title = escape_html(&doc.title.to_lowercase());
                let data_date = escape_html(
                    &crate::domain::document::timestamp::serialize_to_string(&doc.created_at),
                );
                format!(
                    r#"<li class="note-row" data-title="{data_title}" data-date="{data_date}">
            <a class="note-row-title" href="/{slug}">{title}</a>
            <span class="note-row-meta">{date}</span>
            {tags}
          </li>"#,
                    slug = urlencoding::encode(&doc.slug),
                    title = escape_html(&doc.title),
                    date = date_line("Published", doc.created_at),
                    tags = render_tag_chips(&doc.tags),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let truncation = if total > docs.len() as i64 {
            format!(
                r#"<p class="notes-truncation">Showing the {} most recent of {} notes.</p>"#,
                docs.len(),
                total
            )
        } else {
            String::new()
        };

        let nonce = escape_html(csp_nonce);

        format!(
            r#"<div class="notes-page-header">
          <h1>Notes <span class="accent-dot">·</span> <span class="accent-title">Every Note</span></h1>
          <p class="notes-subtitle">A complete index of the garden.</p>
        </div>
        <div class="notes-toolbar">
          <input id="notes-filter" type="search" placeholder="Filter notes…" aria-label="Filter notes" autocomplete="off" />
          <div class="notes-sort" role="group" aria-label="Sort notes">
            <button id="sort-newest" type="button" aria-pressed="true">Newest</button>
            <button id="sort-az" type="button" aria-pressed="false">A–Z</button>
          </div>
        </div>
        {truncation}
        <ul class="notes-list" id="notes-list">
{rows}
        </ul>
        <p class="empty notes-no-matches" id="notes-no-matches" hidden>No notes match your filter.</p>
        <script nonce="{nonce}">
(function () {{
  var input = document.getElementById('notes-filter');
  var list = document.getElementById('notes-list');
  var empty = document.getElementById('notes-no-matches');
  var newest = document.getElementById('sort-newest');
  var az = document.getElementById('sort-az');
  if (!list) return;
  var rows = Array.prototype.slice.call(list.querySelectorAll('li.note-row'));

  function applyFilter() {{
    var q = input ? input.value.trim().toLowerCase() : '';
    var shown = 0;
    rows.forEach(function (li) {{
      var match = !q || (li.dataset.title || '').indexOf(q) !== -1;
      li.style.display = match ? '' : 'none';
      if (match) shown++;
    }});
    if (empty) empty.hidden = shown !== 0;
  }}

  function sortBy(key, dir) {{
    var sorted = rows.slice().sort(function (a, b) {{
      var av = a.dataset[key] || '';
      var bv = b.dataset[key] || '';
      if (av < bv) return -1 * dir;
      if (av > bv) return 1 * dir;
      return 0;
    }});
    sorted.forEach(function (li) {{ list.appendChild(li); }});
  }}

  function setSort(activeButton) {{
    [newest, az].forEach(function (btn) {{
      if (btn) btn.setAttribute('aria-pressed', btn === activeButton ? 'true' : 'false');
    }});
  }}

  if (input) input.addEventListener('input', applyFilter);
  if (newest) newest.addEventListener('click', function () {{ sortBy('date', -1); setSort(newest); }});
  if (az) az.addEventListener('click', function () {{ sortBy('title', 1); setSort(az); }});
}})();
</script>"#
        )
    };

    render_page(
        site,
        HeadMeta {
            title: &format!("Notes \u{2014} {}", site.name),
            description: Some("A complete index of every published note."),
            canonical_url: format!("{}/notes", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce: Some(csp_nonce),
            nav_current: Some("notes"),
            wide_layout: false,
        },
        &main,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::{DocumentStatus, GrowthStage};
    use time::OffsetDateTime;
    use time::macros::datetime;

    fn summary(
        slug: &str,
        title: &str,
        created_at: OffsetDateTime,
        tags: &[&str],
    ) -> DocumentSummary {
        DocumentSummary {
            id: uuid::Uuid::nil(),
            slug: slug.to_string(),
            title: title.to_string(),
            body_excerpt_source: String::new(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            growth: GrowthStage::Seedling,
            status: DocumentStatus::Published,
            created_at,
            updated_at: created_at,
        }
    }

    #[test]
    fn empty_notes_render_the_no_notes_message_without_toolbar_or_script() {
        let site = SiteMeta::defaults();
        let html = render_notes_index_page(&[], 0, "nonce123", &site);

        assert!(html.contains(r#"<p class="empty">No notes yet.</p>"#));
        assert!(!html.contains("notes-toolbar"));
        assert!(!html.contains("notes-list"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn non_empty_notes_render_compact_rows_with_filter_and_sort_controls() {
        let site = SiteMeta::defaults();
        let docs = vec![
            summary(
                "first",
                "First Note",
                datetime!(2026-06-25 12:00 UTC),
                &["rust", "axum"],
            ),
            summary(
                "second",
                "Second Note",
                datetime!(2026-06-23 12:00 UTC),
                &["sqlx"],
            ),
        ];
        let html = render_notes_index_page(&docs, 2, "nonce123", &site);

        // Toolbar: filter input + two sort buttons.
        assert!(html.contains(r#"<input id="notes-filter" type="search""#));
        assert!(html.contains(
            r#"<button id="sort-newest" type="button" aria-pressed="true">Newest</button>"#
        ));
        assert!(
            html.contains(
                r#"<button id="sort-az" type="button" aria-pressed="false">A–Z</button>"#
            )
        );

        // Compact rows with sort/filter data attributes and links.
        assert!(html.contains(r#"<ul class="notes-list" id="notes-list">"#));
        assert!(html.contains(r#"data-title="first note""#));
        assert!(html.contains(r#"data-date="2026-06-25T12:00:00.000Z""#));
        assert!(html.contains(r#"<a class="note-row-title" href="/first">First Note</a>"#));

        // No truncation note when total == rendered count.
        assert!(!html.contains("notes-truncation"));

        // nav + canonical.
        assert!(html.contains(r#"href="/notes""#));
    }

    #[test]
    fn truncation_note_appears_when_total_exceeds_rendered_rows() {
        let site = SiteMeta::defaults();
        let docs = vec![summary("a", "A", datetime!(2026-06-25 12:00 UTC), &[])];
        let html = render_notes_index_page(&docs, 1000, "nonce123", &site);
        assert!(html.contains("Showing the 1 most recent of 1000 notes."));
    }

    #[test]
    fn titles_are_html_escaped_in_rows_and_data_attributes() {
        let site = SiteMeta::defaults();
        let docs = vec![summary(
            "x",
            "A <b> & \"Q\"",
            datetime!(2026-06-25 12:00 UTC),
            &[],
        )];
        let html = render_notes_index_page(&docs, 1, "nonce123", &site);
        assert!(html.contains("A &lt;b&gt; &amp; &quot;Q&quot;"));
        assert!(!html.contains("<b>"));
    }

    #[test]
    fn the_script_carries_the_html_escaped_nonce() {
        let site = SiteMeta::defaults();
        let docs = vec![summary("a", "A", datetime!(2026-06-25 12:00 UTC), &[])];
        let html = render_notes_index_page(&docs, 1, "abc123", &site);
        assert!(html.contains(r#"<script nonce="abc123">"#));

        let hostile = render_notes_index_page(&docs, 1, r#""><x"#, &site);
        assert!(!hostile.contains(r#"<script nonce=""><x">"#));
        assert!(hostile.contains("&quot;&gt;&lt;x"));
    }
}
