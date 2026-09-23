# chessgpt plugin for Claude

Bundles the chessgpt remote MCP server (`https://chessgpt.com/mcp`) with a `chess-coach` skill.

Install in Claude Code:

```
/plugin marketplace add chessgpt/chessgpt
/plugin install chessgpt@chessgpt
```

The first tool call opens a browser to sign in to chessgpt and approve access.

Self-hosting? Change the URL in `.mcp.json` to your server's `/mcp`.

On claude.ai, Claude Desktop and mobile you don't need the plugin: add `https://chessgpt.com/mcp` under Settings → Connectors → Add custom connector.
