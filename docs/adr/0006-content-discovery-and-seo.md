# ADR 0006: Content Discovery and SEO

Status: accepted

Feed, sitemap, and search continue to expose published content only. Search preserves the current `ILIKE`-based parity path.
When published discovery URLs exceed one sitemap page, `/sitemap.xml` becomes a sitemap index that points to bounded sitemap part routes.
