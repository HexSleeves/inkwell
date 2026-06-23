---
title: Bring Your Own Vault
slug: bring-your-own-vault
status: published
tags:
  - garden
  - obsidian
  - import
---

These seeded notes are just a starting point. You can pour your existing
Obsidian vault into the garden with one command:

```bash
inkwell import path/to/your/vault
```

Each Markdown note is pushed through the same write API the [[welcome]] notes
used. Notes that already carry front matter keep their `title`, `slug`, and
`tags`; bare Obsidian notes derive a title from the filename and a slug from
that title. Your `[[wikilinks]]` resolve into the same graph described in
[[wikilinks-and-backlinks]], so imported notes immediately gain backlinks and
become searchable — including by an agent, as in [[reading-with-ai]].

Imported notes arrive as drafts; publish the ones you want public with
`inkwell author publish <slug>`.
