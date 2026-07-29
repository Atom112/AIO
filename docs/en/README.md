# AIO Documentation

This directory documents the features implemented on the `dev` branch. The documentation is organized into user guides, development guides, and references. It does not cover product capabilities that have not been released yet.

## User Guide

1. [Getting Started](usage/getting-started.md): Installation, first-time setup, and your first conversation.
2. [Providers and Models](usage/providers-and-models.md): Remote providers, model catalog, and local inference.
3. [Chat and Agent](usage/chat-and-agent.md): Assistants, projects, tool modes, attachments, branching, and exporting.
4. [MCP and Skills](usage/mcp-and-skills.md): Extension installation, project binding, and tool approval.
5. [App Settings and Shortcuts](usage/app-settings-and-shortcuts.md): Updates, auto-launch, themes, project memory, and quick actions.

## Development Guide

- [Development Environment](development/getting-started.md): Dependencies, running, building, and platform requirements.
- [System Architecture](development/architecture.md): Frontend and backend layering, state, storage, and major data flows.
- [Extension Development](development/extensions.md): Provider, Engine, MCP, and file parser extension points.

## Reference and Support

- [Configuration Reference](reference/configuration.md): Configuration files, database, attachment formats, and security boundaries.
- [Troubleshooting](troubleshooting.md): Connection, engine, MCP, permissions, and build issues.
- [Changelog](../../CHANGELOG.md)

> [!IMPORTANT]
> Documentation follows the code and release workflow. Backend plugins not exposed in the UI are explicitly marked and should not be treated as delivered user features.
