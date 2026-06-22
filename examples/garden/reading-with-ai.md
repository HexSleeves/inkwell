---
title: Reading Your Garden with AI
slug: reading-with-ai
status: published
tags:
  - garden
  - mcp
  - ai
---

# Reading Your Garden with AI

Inkwell ships an **MCP server** (`inkwell mcp`) so an AI agent can read, search,
create, and edit notes in your *live* garden over the Model Context Protocol.

The agent can:

- **search** notes by free text (title and body),
- **read** a single note by slug, returning its body and a `version`,
- **list** every note it can see,
- **create** new notes (wikilinks resolve on write), and
- **update** a note, passing the `version` back as an optimistic-concurrency
  check so it never clobbers a newer edit.

Because the agent writes through the same API as a human, the [[wikilinks-and-backlinks]]
graph stays consistent: a note an agent creates shows up in **Linked from**
panels just like one you wrote by hand.

See the project quickstart for a copy-pasteable client config. New to all this?
Start at [[welcome]] or read [[what-is-a-digital-garden]].
