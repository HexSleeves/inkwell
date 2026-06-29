use crate::domain::document::{ArchiveMonth, DocumentSummary};

use super::layout::{HeadMeta, SiteMeta, escape_html, render_document_list, render_page};

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub fn month_name(month: i32) -> &'static str {
    let idx = (month.clamp(1, 12) - 1) as usize;
    MONTH_NAMES[idx]
}

pub fn render_archive_index_page(months: &[ArchiveMonth], site: &SiteMeta<'_>) -> String {
    let body = if months.is_empty() {
        r#"<p class="empty">No documents published yet.</p>"#.to_string()
    } else {
        render_month_groups(months)
    };
    render_page(
        site,
        HeadMeta {
            title: &format!("Archive \u{2014} {}", site.name),
            description: Some("Browse all published documents by date."),
            canonical_url: format!("{}/archive", site.base_url),
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
            nav_current: None,
            wide_layout: false,
        },
        &format!("<h1>Archive</h1>\n        {}", body),
    )
}

fn render_month_groups(months: &[ArchiveMonth]) -> String {
    let mut sections = Vec::new();
    let mut current_year: Option<i32> = None;
    let mut year_items: Vec<String> = Vec::new();

    for month in months {
        if current_year != Some(month.year) {
            if let Some(year) = current_year {
                sections.push(render_year_section(year, &year_items));
            }
            current_year = Some(month.year);
            year_items = Vec::new();
        }
        year_items.push(format!(
            r#"          <li>
            <a href="/archive/{}/{:02}">{} {} <span class="count">({})</span></a>
          </li>"#,
            month.year,
            month.month,
            month_name(month.month),
            month.year,
            month.count
        ));
    }
    if let Some(year) = current_year {
        sections.push(render_year_section(year, &year_items));
    }
    sections.join("\n        ")
}

fn render_year_section(year: i32, items: &[String]) -> String {
    format!(
        r#"<section class="archive-year">
          <h2>{year}</h2>
          <ul class="archive-months">
{}
          </ul>
        </section>"#,
        items.join("\n")
    )
}

pub fn render_archive_month_page(
    year: i32,
    month: i32,
    documents: &[DocumentSummary],
    page: i64,
    total_pages: i64,
    site: &SiteMeta<'_>,
) -> String {
    let mname = month_name(month);
    let list = if documents.is_empty() {
        r#"<p class="empty">No published documents this month.</p>"#.to_string()
    } else {
        render_document_list(documents)
    };

    let prev = if page > 1 {
        let href = if page - 1 <= 1 {
            format!("/archive/{}/{:02}", year, month)
        } else {
            format!("/archive/{}/{:02}/page/{}", year, month, page - 1)
        };
        format!(r#"<a rel="prev" href="{}">&larr; Newer</a>"#, href)
    } else {
        r#"<span class="spacer">&larr; Newer</span>"#.to_string()
    };
    let next = if page < total_pages {
        format!(
            r#"<a rel="next" href="/archive/{}/{:02}/page/{}">Older &rarr;</a>"#,
            year,
            month,
            page + 1
        )
    } else {
        r#"<span class="spacer">Older &rarr;</span>"#.to_string()
    };
    let pager = if total_pages > 1 {
        format!(r#"<nav class="pager">{}{}</nav>"#, prev, next)
    } else {
        String::new()
    };

    let heading = format!("{} {}", mname, year);
    let title = if page > 1 {
        format!("{} \u{2014} {} \u{2014} Page {}", heading, site.name, page)
    } else {
        format!("{} \u{2014} {}", heading, site.name)
    };
    let canonical = if page > 1 {
        format!(
            "{}/archive/{}/{:02}/page/{}",
            site.base_url, year, month, page
        )
    } else {
        format!("{}/archive/{}/{:02}", site.base_url, year, month)
    };

    let back = r#"<p class="archive-back"><a href="/archive">&larr; All months</a></p>"#;
    render_page(
        site,
        HeadMeta {
            title: &title,
            description: Some(&format!("Published documents from {} {}.", mname, year)),
            canonical_url: canonical,
            og_type: "website",
            json_ld: None,
            csp_nonce: None,
            nav_current: None,
            wide_layout: false,
        },
        &format!(
            "<h1>Archive: {}</h1>\n        {}{}{}\n        {}",
            escape_html(&heading),
            back,
            list,
            pager,
            back
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::layout::SiteMeta;

    fn sample_months() -> Vec<ArchiveMonth> {
        vec![
            ArchiveMonth {
                year: 2026,
                month: 6,
                count: 5,
            },
            ArchiveMonth {
                year: 2026,
                month: 5,
                count: 3,
            },
            ArchiveMonth {
                year: 2025,
                month: 12,
                count: 8,
            },
        ]
    }

    #[test]
    fn month_name_maps_all_12() {
        assert_eq!(month_name(1), "January");
        assert_eq!(month_name(6), "June");
        assert_eq!(month_name(12), "December");
    }

    #[test]
    fn month_name_clamps_out_of_range() {
        assert_eq!(month_name(0), "January");
        assert_eq!(month_name(13), "December");
    }

    #[test]
    fn archive_index_includes_year_headings_and_month_links() {
        let site = SiteMeta::defaults();
        let html = render_archive_index_page(&sample_months(), &site);
        assert!(html.contains("<h2>2026</h2>"));
        assert!(html.contains("<h2>2025</h2>"));
        assert!(html.contains(r#"href="/archive/2026/06""#));
        assert!(html.contains(r#"href="/archive/2025/12""#));
        assert!(html.contains("June 2026"));
        assert!(html.contains("December 2025"));
        assert!(html.contains("(5)"));
        assert!(html.contains("(8)"));
    }

    #[test]
    fn archive_index_empty_renders_empty_message() {
        let site = SiteMeta::defaults();
        let html = render_archive_index_page(&[], &site);
        assert!(html.contains(r#"class="empty""#));
    }

    #[test]
    fn archive_index_has_canonical_url() {
        let site = SiteMeta {
            name: "Inkwell",
            description: None,
            author: None,
            base_url: "https://example.com".to_string(),
            custom_css_url: None,
        };
        let html = render_archive_index_page(&[], &site);
        assert!(html.contains(r#"rel="canonical" href="https://example.com/archive""#));
    }

    #[test]
    fn archive_month_page_has_canonical_and_pager() {
        let site = SiteMeta {
            name: "Inkwell",
            description: None,
            author: None,
            base_url: "https://example.com".to_string(),
            custom_css_url: None,
        };
        let html = render_archive_month_page(2026, 6, &[], 2, 3, &site);
        assert!(
            html.contains(r#"rel="canonical" href="https://example.com/archive/2026/06/page/2""#)
        );
        assert!(html.contains(r#"rel="prev""#));
        assert!(html.contains(r#"rel="next""#));
        assert!(
            html.contains(r#"href="/archive/2026/06""#),
            "prev page 1 goes to base month URL"
        );
    }

    #[test]
    fn archive_month_page_has_back_link() {
        let site = SiteMeta::defaults();
        let html = render_archive_month_page(2026, 1, &[], 1, 1, &site);
        assert!(html.contains(r#"href="/archive""#));
    }

    #[test]
    fn archive_month_page_title_includes_month_year() {
        let site = SiteMeta::defaults();
        let html = render_archive_month_page(2026, 3, &[], 1, 1, &site);
        assert!(html.contains("March 2026"));
    }
}
