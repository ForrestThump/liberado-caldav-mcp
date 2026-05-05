use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct AppState {
    pub caldav_base_url: String,
    pub username: String,
    pub password: String,
    pub http_client: reqwest::Client,
}

// CalDAV data types

#[derive(Debug, Serialize)]
pub struct CalendarInfo {
    pub url: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct EventInfo {
    pub uid: String,
    pub url: String,
    pub summary: String,
    pub description: String,
    pub start: String,
    pub end: String,
    pub location: String,
    pub all_day: bool,
}

#[derive(Debug, Serialize)]
pub struct TaskInfo {
    pub uid: String,
    pub url: String,
    pub summary: String,
    pub description: String,
    pub due: String,
    pub status: String,
    pub priority: u8,
}

// Tool argument types

#[derive(Debug, Deserialize)]
pub struct ListEventsArgs {
    pub calendar_url: String,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEventArgs {
    pub calendar_url: String,
    pub summary: String,
    pub start: String,
    pub end: String,
    pub description: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEventArgs {
    pub event_url: String,
    pub summary: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteEventArgs {
    pub event_url: String,
}

#[derive(Debug, Deserialize)]
pub struct ListTasksArgs {
    pub calendar_url: String,
    pub include_completed: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskArgs {
    pub calendar_url: String,
    pub summary: String,
    pub description: Option<String>,
    pub due: Option<String>,
    pub priority: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTaskArgs {
    pub task_url: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub due: Option<String>,
    pub status: Option<String>,
    pub priority: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteTaskArgs {
    pub task_url: String,
}

