# Quickstart: Zero to an AI-Readable Garden

This walkthrough takes you from nothing to a **populated, browsable digital
garden** that an AI agent can read, search, and edit — in one command.

By the end you will have:

- a seeded garden of interlinked notes running at `http://localhost:3000`,
- working **backlinks** ("Linked from" panels), search, and feeds, and
- an AI client connected to your live garden over the **MCP** server.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) with Compose v2
  (`docker compose version`).
- That's it for the demo. (To build the CLI from source instead, you also need
  Rust — see the [README](../README.md).)

## 1. Configure your keys

Copy the example environment file and set at least `INKWELL_API_KEY`:

```bash
cp .env.example .env
```

Edit `.env`:

- **`INKWELL_API_KEY`** — required. The shared write credential, sent as the
  `X-API-Key` header on every write. The app refuses to start until it is set.
  Use any sufficiently long random string, e.g. `openssl rand -hex 32`.
- **`INKWELL_MCP_KEY`** — optional, but required for the AI walkthrough in
  step 3. A *separate* credential the `inkwell mcp` server authenticates with,
  so AI access can be granted and revoked independently of the human authoring
  key. Generate it the same way.

```dotenv
INKWELL_API_KEY=replace-with-a-long-random-string
INKWELL_MCP_KEY=replace-with-a-different-long-random-string
```

> Never commit real keys. `.env` is gitignored.

## 2. The one command

```bash
docker compose up
```

On first start the app runs **migrate → seed → serve**:

1. `inkwell db migrate` brings the schema up to date.
2. `inkwell seed` plants a handful of bundled, interlinked notes **as
   published** — but only when the garden is empty, so restarts never duplicate
   the demo or clobber your own content.
3. `inkwell serve` starts the HTTP server.

Now open **<http://localhost:3000>**. You will see the seeded notes, including
*Welcome to Your Garden*. Click into any note (for example
<http://localhost:3000/wikilinks-and-backlinks>) and scroll to the bottom: the
**Linked from** panel lists the other notes that wikilink to it. Those panels
are computed from the live link graph, so they stay correct as the garden
grows. Search is at <http://localhost:3000/search?q=garden>.

To stop and wipe the demo database:

```bash
docker compose down -v
```

## 3. Connect an AI agent over MCP

Inkwell ships an **MCP server** so an AI client can read, search, create, and
edit notes in your *live* garden. The server:

- speaks the [Model Context Protocol](https://modelcontextprotocol.io) over
  **stdio** — start it with `inkwell mcp`;
- authenticates against the running HTTP server with **`INKWELL_MCP_KEY`**
  (set in step 1); and
- exposes the tools `search_notes`, `read_note`, `list_notes`, `create_note`,
  and `update_note`. Updates carry the note `version` as an `If-Match` check,
  so an agent never clobbers a newer edit.

Make sure the stack from step 2 is running (the MCP server is a thin client of
the HTTP API), then point your AI client at the `inkwell` binary's `mcp`
subcommand. Most clients use a JSON config like the following — the example
runs `inkwell mcp` inside the already-running compose container so it shares the
network and `INKWELL_MCP_KEY`:

```json
{
  "mcpServers": {
    "inkwell": {
      "command": "docker",
      "args": ["compose", "exec", "-T", "app", "inkwell", "mcp"]
    }
  }
}
```

If you have the `inkwell` binary on your host instead (e.g. `cargo build
--release`), point the client straight at it and pass the env it needs:

```json
{
  "mcpServers": {
    "inkwell": {
      "command": "/path/to/inkwell",
      "args": ["mcp"],
      "env": {
        "INKWELL_API_URL": "http://localhost:3000",
        "INKWELL_MCP_KEY": "the-same-value-as-in-your-.env"
      }
    }
  }
}
```

`inkwell mcp` resolves the server URL from `INKWELL_API_URL` (falling back to
`HOST`/`PORT`) and reads `INKWELL_MCP_KEY` from the environment; it logs to
stderr so its stdout stays a clean JSON-RPC stream. Ask your agent to "list the
notes in the garden" or "search the garden for backlinks" to confirm the
connection. A note an agent creates resolves its wikilinks and shows up in
**Linked from** panels just like one you wrote by hand.

## 4. Bring your own Obsidian vault

The seeded notes are a starting point. Import your existing vault with one
command (run from the host, against the running server):

```bash
inkwell import path/to/your/vault
```

Each Markdown note is pushed through the same write API. Notes with front
matter keep their `title`, `slug`, and `tags`; bare notes derive a title from
the filename and a slug from that title. Your `[[wikilinks]]` resolve into the
same graph, so imported notes immediately gain backlinks and become searchable.
Use `--dry-run` first to preview, and `--server <url>` to target a remote
deployment.

Imported notes arrive as **drafts** (invisible to the public site). Publish the
ones you want public:

```bash
inkwell author publish <slug>
```

That's the full loop: a populated garden in one command, an agent reading and
writing it over MCP, and your own notes folded into the same link graph.
