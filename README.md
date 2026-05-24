# liberado-caldav-mcp

A [Model Context Protocol](https://modelcontextprotocol.io/) server providing CalDAV calendar and CardDAV contacts tools to AI assistants. Built in Rust using [TurboMCP](https://crates.io/crates/turbomcp). Connects to any standard CalDAV/CardDAV server (tested against [Baïkal](https://sabre.io/baikal/)).

## Tools

### Calendars & Events
| Tool | Description |
|---|---|
| `list_calendars` | List all available CalDAV calendars |
| `list_events` | List events in a calendar within an optional date range |
| `create_event` | Create a new calendar event |
| `update_event` | Update an existing event (only provided fields change) |
| `delete_event` | Delete a calendar event |

### Tasks (VTODO)
| Tool | Description |
|---|---|
| `list_tasks` | List tasks in a calendar, optionally including completed |
| `create_task` | Create a new task with optional due date and priority |
| `update_task` | Update an existing task (only provided fields change) |
| `delete_task` | Delete a task |

### Contacts (CardDAV)
| Tool | Description |
|---|---|
| `list_address_books` | List all CardDAV address books |
| `list_contacts` | List contacts in an address book |
| `create_contact` | Create a new contact (vCard) |
| `update_contact` | Update an existing contact (only provided fields change) |
| `delete_contact` | Delete a contact |

## Environment

| Variable | Default | Description |
|---|---|---|
| `CALDAV_URL` | `http://baikal/dav.php` | CalDAV/CardDAV server base URL |
| `CALDAV_USERNAME` | `admin` | Auth username |
| `CALDAV_PASSWORD` | _(empty)_ | Auth password |
| `BIND_ADDR` | `0.0.0.0:8000` | HTTP listen address |

## Running

### Docker

```bash
docker build -t liberado-caldav-mcp .
docker run -p 8000:8000 \
  -e CALDAV_URL=http://your-baikal/dav.php \
  -e CALDAV_USERNAME=admin \
  -e CALDAV_PASSWORD=secret \
  liberado-caldav-mcp
```

### Local development

```bash
CALDAV_URL=http://localhost/dav.php CALDAV_USERNAME=admin CALDAV_PASSWORD=secret cargo run
```

## MCP endpoint

```
http://<host>:8000/
```

Registered in OpenClaw and LibreChat as `caldav-mcp` (streamable-http).
