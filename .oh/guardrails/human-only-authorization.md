---
id: human-only-authorization
severity: hard
statement: "Only a human at the CLI can mint authorization: the MCP server has no code path that approves a plan or writes a grant."
outcome: disk-growth-by-project
audit: human_only_authorization
---

## Detection
`crates/mcp/src/main.rs` never references `actions::approve`, `add_grant`, `write_grants` or `revoke_grant`. AST audit `human_only_authorization`.
