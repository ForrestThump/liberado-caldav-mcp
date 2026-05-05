use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

use crate::caldav;
use crate::models::{
    AppState, CreateEventArgs, CreateTaskArgs, DeleteEventArgs, DeleteTaskArgs, JsonRpcRequest,
    JsonRpcResponse, ListEventsArgs, ListTasksArgs, UpdateEventArgs, UpdateTaskArgs,
};

pub fn create_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/mcp", post(handle_mcp))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({"ok": true}))
}

fn check_api_key(expected: &Option<String>, headers: &HeaderMap) -> bool {
    match expected {
        None => true,
        Some(key) => {
            let provided = headers
                .get("x-api-key")
                .or_else(|| headers.get("authorization"))
                .and_then(|v| v.to_str().ok())
                .map(|s| s.strip_prefix("Bearer ").unwrap_or(s));
            provided.map(|p| p.trim() == key.trim()).unwrap_or(false)
        }
    }
}

fn tool_ok(id: Option<Value>, text: String) -> Json<JsonRpcResponse> {
    Json(JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": text}], "isError": false}),
    ))
}

fn tool_err(id: Option<Value>, msg: String) -> Json<JsonRpcResponse> {
    Json(JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": format!("Error: {msg}")}], "isError": true}),
    ))
}

fn parse_args<T: serde::de::DeserializeOwned>(
    params: Option<Value>,
    id: Option<Value>,
) -> Result<T, Json<JsonRpcResponse>> {
    let args = params
        .and_then(|p| p.get("arguments").cloned())
        .unwrap_or(Value::Object(Default::default()));
    serde_json::from_value(args).map_err(|e| {
        tool_err(id, format!("invalid arguments: {e}"))
    })
}

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "list_calendars",
                "description": "List all available CalDAV calendars",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "list_events",
                "description": "List events in a calendar within a date range",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "calendar_url": {"type": "string", "description": "Calendar URL from list_calendars"},
                        "start": {"type": "string", "description": "Range start (ISO 8601). Default: now"},
                        "end": {"type": "string", "description": "Range end (ISO 8601). Default: +30 days"}
                    },
                    "required": ["calendar_url"]
                }
            },
            {
                "name": "create_event",
                "description": "Create a new calendar event",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "calendar_url": {"type": "string", "description": "Calendar URL from list_calendars"},
                        "summary": {"type": "string", "description": "Event title"},
                        "start": {"type": "string", "description": "Start datetime (ISO 8601)"},
                        "end": {"type": "string", "description": "End datetime (ISO 8601)"},
                        "description": {"type": "string", "description": "Event description"},
                        "location": {"type": "string", "description": "Event location"}
                    },
                    "required": ["calendar_url", "summary", "start", "end"]
                }
            },
            {
                "name": "update_event",
                "description": "Update an existing calendar event. Only provided fields are changed.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "event_url": {"type": "string", "description": "Event URL from list_events"},
                        "summary": {"type": "string"},
                        "start": {"type": "string", "description": "ISO 8601"},
                        "end": {"type": "string", "description": "ISO 8601"},
                        "description": {"type": "string"},
                        "location": {"type": "string"}
                    },
                    "required": ["event_url"]
                }
            },
            {
                "name": "delete_event",
                "description": "Delete a calendar event",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "event_url": {"type": "string", "description": "Event URL from list_events"}
                    },
                    "required": ["event_url"]
                }
            },
            {
                "name": "list_tasks",
                "description": "List tasks (VTODO) in a calendar",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "calendar_url": {"type": "string", "description": "Calendar URL from list_calendars"},
                        "include_completed": {"type": "boolean", "description": "Include completed tasks. Default: false"}
                    },
                    "required": ["calendar_url"]
                }
            },
            {
                "name": "create_task",
                "description": "Create a new task",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "calendar_url": {"type": "string", "description": "Calendar URL from list_calendars"},
                        "summary": {"type": "string", "description": "Task title"},
                        "description": {"type": "string"},
                        "due": {"type": "string", "description": "Due date/datetime (ISO 8601)"},
                        "priority": {"type": "integer", "description": "Priority 1-9 (1=highest), 0=none", "minimum": 0, "maximum": 9}
                    },
                    "required": ["calendar_url", "summary"]
                }
            },
            {
                "name": "update_task",
                "description": "Update an existing task. Only provided fields are changed.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task_url": {"type": "string", "description": "Task URL from list_tasks"},
                        "summary": {"type": "string"},
                        "description": {"type": "string"},
                        "due": {"type": "string", "description": "ISO 8601"},
                        "status": {"type": "string", "enum": ["NEEDS-ACTION", "IN-PROCESS", "COMPLETED", "CANCELLED"]},
                        "priority": {"type": "integer", "minimum": 0, "maximum": 9}
                    },
                    "required": ["task_url"]
                }
            },
            {
                "name": "delete_task",
                "description": "Delete a task",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task_url": {"type": "string", "description": "Task URL from list_tasks"}
                    },
                    "required": ["task_url"]
                }
            }
        ]
    })
}

async fn handle_mcp(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    if !check_api_key(&state.api_key, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(JsonRpcResponse::err(req.id, 401, "unauthorized".into())),
        )
            .into_response();
    }

    let id = req.id.clone();

    match req.method.as_str() {
        "initialize" => Json(JsonRpcResponse::ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "caldav-mcp", "version": "0.1.0"}
            }),
        ))
        .into_response(),

        "notifications/initialized" | "ping" => {
            Json(JsonRpcResponse::ok(id, json!({}))).into_response()
        }

        "tools/list" => Json(JsonRpcResponse::ok(id, tools_list())).into_response(),

        "tools/call" => {
            let tool_name = req
                .params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            dispatch_tool(&state, id, &tool_name, req.params)
                .await
                .into_response()
        }

        _ => Json(JsonRpcResponse::err(id, -32601, "method not found".into())).into_response(),
    }
}

async fn dispatch_tool(
    state: &Arc<AppState>,
    id: Option<Value>,
    name: &str,
    params: Option<Value>,
) -> Json<JsonRpcResponse> {
    match name {
        "list_calendars" => match caldav::list_calendars(state).await {
            Ok(cals) => tool_ok(id, serde_json::to_string_pretty(&cals).unwrap()),
            Err(e) => tool_err(id, e.to_string()),
        },

        "list_events" => {
            let args = match parse_args::<ListEventsArgs>(params, id.clone()) {
                Ok(a) => a,
                Err(e) => return e,
            };
            match caldav::list_events(state, &args).await {
                Ok(events) => tool_ok(id, serde_json::to_string_pretty(&events).unwrap()),
                Err(e) => tool_err(id, e.to_string()),
            }
        }

        "create_event" => {
            let args = match parse_args::<CreateEventArgs>(params, id.clone()) {
                Ok(a) => a,
                Err(e) => return e,
            };
            match caldav::create_event(state, &args).await {
                Ok(event) => tool_ok(id, serde_json::to_string_pretty(&event).unwrap()),
                Err(e) => tool_err(id, e.to_string()),
            }
        }

        "update_event" => {
            let args = match parse_args::<UpdateEventArgs>(params, id.clone()) {
                Ok(a) => a,
                Err(e) => return e,
            };
            match caldav::update_event(state, &args).await {
                Ok(()) => tool_ok(id, "updated".to_string()),
                Err(e) => tool_err(id, e.to_string()),
            }
        }

        "delete_event" => {
            let args = match parse_args::<DeleteEventArgs>(params, id.clone()) {
                Ok(a) => a,
                Err(e) => return e,
            };
            match caldav::delete_event(state, &args).await {
                Ok(()) => tool_ok(id, "deleted".to_string()),
                Err(e) => tool_err(id, e.to_string()),
            }
        }

        "list_tasks" => {
            let args = match parse_args::<ListTasksArgs>(params, id.clone()) {
                Ok(a) => a,
                Err(e) => return e,
            };
            match caldav::list_tasks(state, &args).await {
                Ok(tasks) => tool_ok(id, serde_json::to_string_pretty(&tasks).unwrap()),
                Err(e) => tool_err(id, e.to_string()),
            }
        }

        "create_task" => {
            let args = match parse_args::<CreateTaskArgs>(params, id.clone()) {
                Ok(a) => a,
                Err(e) => return e,
            };
            match caldav::create_task(state, &args).await {
                Ok(task) => tool_ok(id, serde_json::to_string_pretty(&task).unwrap()),
                Err(e) => tool_err(id, e.to_string()),
            }
        }

        "update_task" => {
            let args = match parse_args::<UpdateTaskArgs>(params, id.clone()) {
                Ok(a) => a,
                Err(e) => return e,
            };
            match caldav::update_task(state, &args).await {
                Ok(()) => tool_ok(id, "updated".to_string()),
                Err(e) => tool_err(id, e.to_string()),
            }
        }

        "delete_task" => {
            let args = match parse_args::<DeleteTaskArgs>(params, id.clone()) {
                Ok(a) => a,
                Err(e) => return e,
            };
            match caldav::delete_task(state, &args).await {
                Ok(()) => tool_ok(id, "deleted".to_string()),
                Err(e) => tool_err(id, e.to_string()),
            }
        }

        _ => tool_err(id, format!("unknown tool: {name}")),
    }
}
