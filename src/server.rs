use std::sync::Arc;

use turbomcp::prelude::*;

use crate::caldav;
use crate::models::{
    AppState, CreateContactArgs, CreateEventArgs, CreateTaskArgs, DeleteContactArgs,
    DeleteEventArgs, DeleteTaskArgs, ListContactsArgs, ListEventsArgs, ListTasksArgs,
    UpdateContactArgs, UpdateEventArgs, UpdateTaskArgs,
};

#[derive(Clone)]
pub struct CaldavServer {
    state: Arc<AppState>,
}

#[turbomcp::server(name = "caldav-mcp", version = "0.1.0")]
impl CaldavServer {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AppState {
                caldav_base_url: std::env::var("CALDAV_URL")
                    .unwrap_or_else(|_| "http://baikal/dav.php".to_string()),
                username: std::env::var("CALDAV_USERNAME").unwrap_or_else(|_| "admin".to_string()),
                password: std::env::var("CALDAV_PASSWORD").unwrap_or_default(),
                http_client: reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .expect("failed to build HTTP client"),
            }),
        }
    }

    #[tool("List all available CalDAV calendars")]
    async fn list_calendars(&self) -> McpResult<String> {
        caldav::list_calendars(&self.state)
            .await
            .map(|cals| serde_json::to_string_pretty(&cals).unwrap())
            .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("List events in a calendar within a date range")]
    async fn list_events(
        &self,
        calendar_url: String,
        start: Option<String>,
        end: Option<String>,
    ) -> McpResult<String> {
        caldav::list_events(
            &self.state,
            &ListEventsArgs {
                calendar_url,
                start,
                end,
            },
        )
        .await
        .map(|v| serde_json::to_string_pretty(&v).unwrap())
        .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("Create a new calendar event")]
    async fn create_event(
        &self,
        calendar_url: String,
        summary: String,
        start: String,
        end: String,
        description: Option<String>,
        location: Option<String>,
    ) -> McpResult<String> {
        caldav::create_event(
            &self.state,
            &CreateEventArgs {
                calendar_url,
                summary,
                start,
                end,
                description,
                location,
            },
        )
        .await
        .map(|v| serde_json::to_string_pretty(&v).unwrap())
        .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("Update an existing calendar event; only provided fields are changed")]
    async fn update_event(
        &self,
        event_url: String,
        summary: Option<String>,
        start: Option<String>,
        end: Option<String>,
        description: Option<String>,
        location: Option<String>,
    ) -> McpResult<String> {
        caldav::update_event(
            &self.state,
            &UpdateEventArgs {
                event_url,
                summary,
                start,
                end,
                description,
                location,
            },
        )
        .await
        .map(|_| "updated".to_string())
        .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("Delete a calendar event")]
    async fn delete_event(&self, event_url: String) -> McpResult<String> {
        caldav::delete_event(&self.state, &DeleteEventArgs { event_url })
            .await
            .map(|_| "deleted".to_string())
            .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("List tasks (VTODO) in a calendar")]
    async fn list_tasks(
        &self,
        calendar_url: String,
        include_completed: Option<bool>,
    ) -> McpResult<String> {
        caldav::list_tasks(
            &self.state,
            &ListTasksArgs {
                calendar_url,
                include_completed,
            },
        )
        .await
        .map(|v| serde_json::to_string_pretty(&v).unwrap())
        .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("Create a new task (VTODO)")]
    async fn create_task(
        &self,
        calendar_url: String,
        summary: String,
        description: Option<String>,
        due: Option<String>,
        priority: Option<u8>,
    ) -> McpResult<String> {
        caldav::create_task(
            &self.state,
            &CreateTaskArgs {
                calendar_url,
                summary,
                description,
                due,
                priority,
            },
        )
        .await
        .map(|v| serde_json::to_string_pretty(&v).unwrap())
        .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("Update an existing task; only provided fields are changed")]
    async fn update_task(
        &self,
        task_url: String,
        summary: Option<String>,
        description: Option<String>,
        due: Option<String>,
        status: Option<String>,
        priority: Option<u8>,
    ) -> McpResult<String> {
        caldav::update_task(
            &self.state,
            &UpdateTaskArgs {
                task_url,
                summary,
                description,
                due,
                status,
                priority,
            },
        )
        .await
        .map(|_| "updated".to_string())
        .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("Delete a task")]
    async fn delete_task(&self, task_url: String) -> McpResult<String> {
        caldav::delete_task(&self.state, &DeleteTaskArgs { task_url })
            .await
            .map(|_| "deleted".to_string())
            .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("List all CardDAV address books")]
    async fn list_address_books(&self) -> McpResult<String> {
        caldav::list_address_books(&self.state)
            .await
            .map(|v| serde_json::to_string_pretty(&v).unwrap())
            .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("List contacts in an address book")]
    async fn list_contacts(&self, address_book_url: String) -> McpResult<String> {
        caldav::list_contacts(&self.state, &ListContactsArgs { address_book_url })
            .await
            .map(|v| serde_json::to_string_pretty(&v).unwrap())
            .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("Create a new contact (VCARD)")]
    async fn create_contact(
        &self,
        address_book_url: String,
        full_name: String,
        email: Option<String>,
        phone: Option<String>,
        organization: Option<String>,
        notes: Option<String>,
    ) -> McpResult<String> {
        caldav::create_contact(
            &self.state,
            &CreateContactArgs {
                address_book_url,
                full_name,
                email,
                phone,
                organization,
                notes,
            },
        )
        .await
        .map(|v| serde_json::to_string_pretty(&v).unwrap())
        .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("Update an existing contact; only provided fields are changed")]
    async fn update_contact(
        &self,
        contact_url: String,
        full_name: Option<String>,
        email: Option<String>,
        phone: Option<String>,
        organization: Option<String>,
        notes: Option<String>,
    ) -> McpResult<String> {
        caldav::update_contact(
            &self.state,
            &UpdateContactArgs {
                contact_url,
                full_name,
                email,
                phone,
                organization,
                notes,
            },
        )
        .await
        .map(|_| "updated".to_string())
        .map_err(|e| McpError::internal(e.to_string()))
    }

    #[tool("Delete a contact")]
    async fn delete_contact(&self, contact_url: String) -> McpResult<String> {
        caldav::delete_contact(&self.state, &DeleteContactArgs { contact_url })
            .await
            .map(|_| "deleted".to_string())
            .map_err(|e| McpError::internal(e.to_string()))
    }
}
