---
title: Wikilinks and Backlinks
slug: wikilinks-and-backlinks
status: published
tags:
  - garden
  - notes
---

A **wikilink** is written as `[[target-note]]`. When inkwell renders a note it
resolves each wikilink against the published garden:

- If the target exists, the link becomes a normal link you can click.
- If it does not exist yet, the link renders as a *stub* — and lights up
  automatically the moment a note with that slug is published.

A **backlink** is the reverse direction. Because [[welcome]] links to this
note, this note shows *Welcome to Your Garden* in its **Linked from** panel.
Backlinks are computed from the link graph, so they stay correct as you edit.

This two-way linking is what makes [[what-is-a-digital-garden]] feel like a
garden rather than a pile of pages. An agent can traverse the same graph — see
[[reading-with-ai]].
