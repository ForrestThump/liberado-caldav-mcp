# Architecture — liberado-caldav-mcp

## Overview

A stateless HTTP MCP server that translates MCP tool calls into CalDAV/CardDAV protocol requests (WebDAV + iCalendar/vCard). Uses TurboMCP for the MCP layer and speaks raw CalDAV over HTTP Basic auth. No local state — all data lives on the CalDAV server.

## Source layout

```
src/
  main.rs     — entry point: reads BIND_ADDR, starts TurboMCP HTTP server
  lib.rs      — re-exports server and submodules (for integration tests)
  server.rs   — MCP tool definitions via #[turbomcp::server] macro
  caldav.rs   — CalDAV/CardDAV HTTP client (PROPFIND, REPORT, PUT, DELETE)
  models.rs   — AppState, argument structs for all tool calls
```

## Request flow

```
MCP client
    │
    │  HTTP POST /   (MCP JSON-RPC)
    ▼
TurboMCP runtime
    │
    ▼
CaldavServer::<tool>()   (server.rs)
    │
    ▼
caldav::<operation>()    (caldav.rs)
    │
    │  HTTP (WebDAV: PROPFIND / REPORT / PUT / DELETE)
    │  Auth: Basic (CALDAV_USERNAME / CALDAV_PASSWORD)
    ▼
CalDAV/CardDAV server (Baïkal)
```

## Protocol details

CalDAV and CardDAV are built on WebDAV. Key request types:

| Operation | HTTP method | Body format |
|---|---|---|
| Discover calendars/address books | `PROPFIND` | XML |
| Query events/tasks/contacts | `REPORT` | XML (calendar-query / addressbook-query) |
| Create or update | `PUT` | iCalendar (`.ics`) or vCard (`.vcf`) |
| Delete | `DELETE` | — |

Responses are XML; the `caldav.rs` module parses them with `quick-xml`.

Events and tasks use iCalendar format (RFC 5545). Contacts use vCard format (RFC 6350). UUIDs are generated with `uuid::Uuid::new_v4()` for new resources.

## Key types

| Type | Module | Role |
|---|---|---|
| `CaldavServer` | `server` | TurboMCP server struct; holds `Arc<AppState>` |
| `AppState` | `models` | Shared config: base URL, credentials, `reqwest::Client` |
| `*Args` structs | `models` | Typed argument structs for each tool call |

## Design decisions

**Stateless per-request** — `AppState` holds only config and a connection-pooled `reqwest::Client`. No caching, no local storage.

**Raw WebDAV, no CalDAV library** — The caldav.rs module hand-rolls the XML/WebDAV requests. This keeps the dependency tree small and makes the protocol behavior explicit.

**All updates are read-modify-write** — For `update_event`, `update_task`, and `update_contact`, the server fetches the existing resource, patches only the provided fields, then PUTs it back. This preserves fields the tool doesn't expose (recurrence rules, custom properties, etc.).

## Dependencies

| Crate | Purpose |
|---|---|
| `turbomcp` | MCP server framework (HTTP transport, tool macro) |
| `tokio` | Async runtime |
| `reqwest` | HTTP client for CalDAV/CardDAV requests |
| `quick-xml` | XML parsing for WebDAV responses |
| `uuid` | UUID generation for new resource URLs |
| `chrono` | Date/time formatting for iCalendar |
| `serde` / `serde_json` | JSON serialization for MCP responses |
| `thiserror` | Typed error enum |
| `tracing` / `tracing-subscriber` | Structured logging |

## Deployment

- **Image**: multi-stage Dockerfile — `rust:1.89-slim-bookworm` builder → `debian:bookworm-slim` runtime.
- **Port**: 8000 (configurable via `BIND_ADDR`).
- **No volumes, no secrets stored** — credentials come from environment variables only.
