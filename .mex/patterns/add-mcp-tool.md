---
name: add-mcp-tool
description: Add a new tool to the inkwell MCP server so AI agents can call it via the Model Context Protocol.
triggers:
  - "mcp tool"
  - "add tool"
  - "agent tool"
  - "rmcp"
  - "MCP"
edges:
  - target: context/architecture.md
    condition: when understanding how the MCP server connects to the HTTP API
  - target: context/conventions.md
    condition: when writing the tool implementation
  - target: patterns/add-endpoint.md
    condition: when the tool needs a new HTTP endpoint to back it
last_updated: 2026-06-23
---

# Add MCP Tool

## Context

The MCP server (`src/mcp/mod.rs`) is a thin client over `InkwellClient`. Every tool delegates to `self.client.*` — it never touches the DB. Tools are discovered via the `#[tool_router]` + `#[tool]` macros from `rmcp`. Argument types derive `JsonSchema` (for schema advertisement to the agent) and `Deserialize` (for call argument parsing).

The MCP server runs as `inkwell mcp` over stdio and authenticates with `INKWELL_API_KEY` set to a scoped token (the separate `INKWELL_MCP_KEY` was retired in slice 4, ADR 0009).

## Steps

1. **Define the argument type** in `src/mcp/mod.rs`:
   ```rust
   #[derive(Debug, Deserialize, JsonSchema)]
   pub struct MyToolArgs {
       /// Description shown to the agent in the tool schema.
       pub my_param: String,
       /// Optional param — use #[serde(default)].
       #[serde(default)]
       pub optional_param: Option<String>,
   }
   ```

2. **Add the tool method** inside `#[tool_router] impl InkwellMcpServer`:
   ```rust
   /// One-line description shown to the agent.
   #[tool(description = "Full description the agent sees. Be precise about inputs and outputs.")]
   pub async fn my_tool(
       &self,
       Parameters(args): Parameters<MyToolArgs>,
   ) -> Result<CallToolResult, ErrorData> {
       let result = self.client.my_operation(&args.my_param).await.map_err(to_mcp_error)?;
       ok_json(&serde_json::json!({ "key": result }))
   }
   ```

3. **Add the backing method to `InkwellClient`** (`src/client/mod.rs`) if the operation doesn't exist yet. Client methods call `GET /documents/...` or `POST /documents/...` via `reqwest`.

4. **Expose a new HTTP endpoint** if needed — follow `patterns/add-endpoint.md`.

5. **Test via `inkwell mcp`**: start `inkwell serve`, then `inkwell mcp` in another terminal. Use an MCP client or `echo '{"jsonrpc":"2.0","method":"tools/list","id":1}' | inkwell mcp` to confirm the tool appears.

## Gotchas

- All `#[tool]` methods must be inside the `#[tool_router] impl InkwellMcpServer` block — methods outside it are invisible to the router
- `ok_json` serializes to a single structured-JSON `Content` block; use it for all success responses; never return raw text unless the agent needs plain prose
- `to_mcp_error` maps `anyhow::Error` to `ErrorData::internal_error` — keep error messages human-readable since agents surface them to the user
- Optimistic concurrency: any tool that updates a note must send `expected_version` via `If-Match`. See `update_note` for the pattern — always read the current version with `read_note` first.
- The tool description and param doc-comments are the agent's only documentation — write them as if briefing a new colleague on the exact contract

## Verify

- [ ] Arg type derives `Debug + Deserialize + JsonSchema`
- [ ] Optional fields use `#[serde(default)]`
- [ ] Method is inside `#[tool_router] impl InkwellMcpServer`
- [ ] `ok_json` used for all success returns
- [ ] `to_mcp_error` used for all error paths
- [ ] `tools/list` via `inkwell mcp` shows the new tool
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes

## Update Scaffold
- [ ] Update `.mex/ROUTER.md` "Current Project State" if this completes an MCP feature
- [ ] Update `context/architecture.md` "Key Components" MCP section if the tool surface changes significantly
