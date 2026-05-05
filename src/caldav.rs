use std::collections::HashMap;

use chrono::Utc;
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader;
use reqwest::Method;
use thiserror::Error;
use uuid::Uuid;

use crate::models::{
    AppState, CalendarInfo, CreateEventArgs, CreateTaskArgs, DeleteEventArgs, DeleteTaskArgs,
    EventInfo, ListEventsArgs, ListTasksArgs, TaskInfo, UpdateEventArgs, UpdateTaskArgs,
};

#[derive(Debug, Error)]
pub enum CaldavError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("CalDAV error {status}: {body}")]
    Status { status: u16, body: String },
    #[allow(dead_code)]
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Invalid argument: {0}")]
    InvalidArg(String),
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

/// Parse a CalDAV multistatus XML response into per-response property maps.
/// Returns vec of (href, props) where props maps local element name → text content.
fn parse_multistatus(xml: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut reader = Reader::from_str(xml);
    let mut responses: Vec<(String, HashMap<String, String>)> = Vec::new();
    let mut current_href = String::new();
    let mut current_props: HashMap<String, String> = HashMap::new();
    let mut current_tag = String::new();
    let mut current_text = String::new();
    let mut in_response = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(ref e)) => {
                let name =
                    String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "response" {
                    in_response = true;
                    current_href.clear();
                    current_props.clear();
                }
                current_tag = name;
                current_text.clear();
            }
            Ok(XmlEvent::Empty(ref e)) if in_response => {
                let name =
                    String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "calendar" {
                    current_props.insert("__is_calendar__".to_string(), "1".to_string());
                }
            }
            Ok(XmlEvent::Text(ref e)) if in_response => {
                if let Ok(t) = e.unescape() {
                    current_text.push_str(&t);
                }
            }
            Ok(XmlEvent::CData(ref e)) if in_response => {
                current_text
                    .push_str(&String::from_utf8_lossy(e.as_ref()));
            }
            Ok(XmlEvent::End(ref e)) => {
                let name =
                    String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if in_response {
                    let text = current_text.trim().to_string();
                    if !text.is_empty() {
                        if name == "href" && current_href.is_empty() {
                            current_href = text.clone();
                        }
                        current_props.insert(name.clone(), text);
                    }
                    if name == "response" {
                        responses.push((current_href.clone(), current_props.clone()));
                        in_response = false;
                    }
                }
                current_text.clear();
                current_tag.clear();
            }
            Ok(XmlEvent::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    let _ = current_tag; // suppress unused warning
    responses
}

// ---------------------------------------------------------------------------
// iCal helpers
// ---------------------------------------------------------------------------

/// Convert ISO 8601 datetime/date string to iCal format.
/// Returns (ical_string, is_all_day).
fn iso_to_ical_dt(iso: &str) -> Result<(String, bool), CaldavError> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
        let utc = dt.with_timezone(&Utc);
        return Ok((utc.format("%Y%m%dT%H%M%SZ").to_string(), false));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d") {
        return Ok((date.format("%Y%m%d").to_string(), true));
    }
    Err(CaldavError::InvalidArg(format!("invalid datetime: {iso}")))
}

/// Convert iCal datetime string to ISO 8601.
fn ical_dt_to_iso(ical_dt: &str) -> String {
    let s = ical_dt.trim_end_matches('Z');
    if s.contains('T') && s.len() >= 15 {
        format!(
            "{}-{}-{}T{}:{}:{}Z",
            &s[0..4],
            &s[4..6],
            &s[6..8],
            &s[9..11],
            &s[11..13],
            &s[13..15]
        )
    } else if s.len() == 8 {
        format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8])
    } else {
        ical_dt.to_string()
    }
}

/// Parse a VEVENT or VTODO component from an iCal string.
/// Returns a flat map of base property name → value.
fn parse_ical_component(ical: &str, component: &str) -> HashMap<String, String> {
    let unfolded = ical
        .replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\n ", "")
        .replace("\n\t", "");

    let begin = format!("BEGIN:{component}");
    let end = format!("END:{component}");

    let start_idx = match unfolded.find(&begin) {
        Some(i) => i + begin.len(),
        None => return HashMap::new(),
    };
    let end_idx = match unfolded[start_idx..].find(&end) {
        Some(i) => start_idx + i,
        None => return HashMap::new(),
    };

    let mut props = HashMap::new();
    for line in unfolded[start_idx..end_idx].lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key_part = &line[..colon];
            let value = &line[colon + 1..];
            let base_key = key_part.split(';').next().unwrap_or(key_part);
            props.insert(base_key.to_uppercase(), value.to_string());
        }
    }
    props
}

fn now_ical() -> String {
    Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

fn build_event_ical(props: &HashMap<String, String>) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//caldav-mcp-rs//EN".to_string(),
        "BEGIN:VEVENT".to_string(),
    ];
    for key in &[
        "UID", "DTSTART", "DTEND", "SUMMARY", "DESCRIPTION", "LOCATION",
        "DTSTAMP", "LAST-MODIFIED", "SEQUENCE", "STATUS",
    ] {
        if let Some(val) = props.get(*key) {
            if !val.is_empty() {
                lines.push(format!("{key}:{val}"));
            }
        }
    }
    lines.push("END:VEVENT".to_string());
    lines.push("END:VCALENDAR".to_string());
    lines.join("\r\n")
}

fn build_task_ical(props: &HashMap<String, String>) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//caldav-mcp-rs//EN".to_string(),
        "BEGIN:VTODO".to_string(),
    ];
    for key in &[
        "UID", "SUMMARY", "DESCRIPTION", "DUE", "STATUS", "PRIORITY",
        "DTSTAMP", "LAST-MODIFIED", "SEQUENCE",
    ] {
        if let Some(val) = props.get(*key) {
            if !val.is_empty() {
                lines.push(format!("{key}:{val}"));
            }
        }
    }
    lines.push("END:VTODO".to_string());
    lines.push("END:VCALENDAR".to_string());
    lines.join("\r\n")
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async fn dav_request(
    state: &AppState,
    method: &str,
    url: &str,
    extra_headers: &[(&str, &str)],
    body: Option<&str>,
) -> Result<String, CaldavError> {
    let method = Method::from_bytes(method.as_bytes())
        .map_err(|e| CaldavError::InvalidArg(e.to_string()))?;

    let mut req = state
        .http_client
        .request(method, url)
        .basic_auth(&state.username, Some(&state.password));

    for (k, v) in extra_headers {
        req = req.header(*k, *v);
    }

    if let Some(b) = body {
        req = req
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(b.to_string());
    }

    let resp = req.send().await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;

    if status >= 400 {
        return Err(CaldavError::Status { status, body: text });
    }
    Ok(text)
}

async fn propfind(state: &AppState, url: &str, depth: &str, body: &str) -> Result<String, CaldavError> {
    dav_request(
        state,
        "PROPFIND",
        url,
        &[("Depth", depth)],
        Some(body),
    )
    .await
}

async fn report(state: &AppState, url: &str, body: &str) -> Result<String, CaldavError> {
    dav_request(state, "REPORT", url, &[("Depth", "1")], Some(body)).await
}

async fn http_get(state: &AppState, url: &str) -> Result<String, CaldavError> {
    dav_request(state, "GET", url, &[], None).await
}

async fn http_put(state: &AppState, url: &str, ical: &str) -> Result<(), CaldavError> {
    dav_request(
        state,
        "PUT",
        url,
        &[("Content-Type", "text/calendar; charset=utf-8")],
        Some(ical),
    )
    .await
    .map(|_| ())
}

async fn http_delete(state: &AppState, url: &str) -> Result<(), CaldavError> {
    dav_request(state, "DELETE", url, &[], None)
        .await
        .map(|_| ())
}

fn calendar_home_url(state: &AppState) -> String {
    format!(
        "{}/calendars/{}/",
        state.caldav_base_url.trim_end_matches('/'),
        state.username
    )
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub async fn list_calendars(state: &AppState) -> Result<Vec<CalendarInfo>, CaldavError> {
    let url = calendar_home_url(state);
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:displayname/>
    <d:resourcetype/>
    <c:calendar-description/>
  </d:prop>
</d:propfind>"#;

    let xml = propfind(state, &url, "1", body).await?;
    let responses = parse_multistatus(&xml);

    let calendars = responses
        .into_iter()
        .filter(|(_, props)| props.contains_key("__is_calendar__"))
        .map(|(href, props)| CalendarInfo {
            url: href,
            name: props.get("displayname").cloned().unwrap_or_default(),
            description: props
                .get("calendar-description")
                .cloned()
                .unwrap_or_default(),
        })
        .collect();

    Ok(calendars)
}

pub async fn list_events(
    state: &AppState,
    args: &ListEventsArgs,
) -> Result<Vec<EventInfo>, CaldavError> {
    let now = Utc::now();
    let start = args
        .start
        .as_deref()
        .map(|s| iso_to_ical_dt(s).map(|(dt, _)| dt))
        .transpose()?
        .unwrap_or_else(|| now.format("%Y%m%dT%H%M%SZ").to_string());
    let end = args
        .end
        .as_deref()
        .map(|s| iso_to_ical_dt(s).map(|(dt, _)| dt))
        .transpose()?
        .unwrap_or_else(|| {
            (now + chrono::Duration::days(30))
                .format("%Y%m%dT%H%M%SZ")
                .to_string()
        });

    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT">
        <c:time-range start="{start}" end="{end}"/>
      </c:comp-filter>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#
    );

    let xml = report(state, &args.calendar_url, &body).await?;
    let responses = parse_multistatus(&xml);

    let mut events = Vec::new();
    for (href, props) in responses {
        if let Some(ical_data) = props.get("calendar-data") {
            if !ical_data.contains("BEGIN:VEVENT") {
                continue;
            }
            let p = parse_ical_component(ical_data, "VEVENT");
            let dtstart = p.get("DTSTART").cloned().unwrap_or_default();
            let all_day = !dtstart.contains('T');
            events.push(EventInfo {
                uid: p.get("UID").cloned().unwrap_or_default(),
                url: href,
                summary: p.get("SUMMARY").cloned().unwrap_or_default(),
                description: p.get("DESCRIPTION").cloned().unwrap_or_default(),
                start: ical_dt_to_iso(&dtstart),
                end: ical_dt_to_iso(p.get("DTEND").map(|s| s.as_str()).unwrap_or("")),
                location: p.get("LOCATION").cloned().unwrap_or_default(),
                all_day,
            });
        }
    }
    Ok(events)
}

pub async fn create_event(
    state: &AppState,
    args: &CreateEventArgs,
) -> Result<EventInfo, CaldavError> {
    let uid = Uuid::new_v4().to_string();
    let (dtstart, all_day) = iso_to_ical_dt(&args.start)?;
    let (dtend, _) = iso_to_ical_dt(&args.end)?;
    let now = now_ical();

    let mut props = HashMap::new();
    props.insert("UID".to_string(), uid.clone());
    props.insert("DTSTART".to_string(), dtstart);
    props.insert("DTEND".to_string(), dtend);
    props.insert("SUMMARY".to_string(), args.summary.clone());
    props.insert(
        "DESCRIPTION".to_string(),
        args.description.clone().unwrap_or_default(),
    );
    props.insert(
        "LOCATION".to_string(),
        args.location.clone().unwrap_or_default(),
    );
    props.insert("DTSTAMP".to_string(), now.clone());
    props.insert("LAST-MODIFIED".to_string(), now);
    props.insert("SEQUENCE".to_string(), "0".to_string());

    let ical = build_event_ical(&props);
    let url = format!(
        "{}{}.ics",
        args.calendar_url.trim_end_matches('/').to_string() + "/",
        uid
    );
    http_put(state, &url, &ical).await?;

    Ok(EventInfo {
        uid,
        url,
        summary: args.summary.clone(),
        description: args.description.clone().unwrap_or_default(),
        start: args.start.clone(),
        end: args.end.clone(),
        location: args.location.clone().unwrap_or_default(),
        all_day,
    })
}

pub async fn update_event(
    state: &AppState,
    args: &UpdateEventArgs,
) -> Result<(), CaldavError> {
    let existing_ical = http_get(state, &args.event_url).await?;
    let mut props = parse_ical_component(&existing_ical, "VEVENT");

    if let Some(s) = &args.summary {
        props.insert("SUMMARY".to_string(), s.clone());
    }
    if let Some(s) = &args.start {
        let (dt, _) = iso_to_ical_dt(s)?;
        props.insert("DTSTART".to_string(), dt);
    }
    if let Some(s) = &args.end {
        let (dt, _) = iso_to_ical_dt(s)?;
        props.insert("DTEND".to_string(), dt);
    }
    if let Some(s) = &args.description {
        props.insert("DESCRIPTION".to_string(), s.clone());
    }
    if let Some(s) = &args.location {
        props.insert("LOCATION".to_string(), s.clone());
    }

    let now = now_ical();
    props.insert("LAST-MODIFIED".to_string(), now.clone());
    props.insert("DTSTAMP".to_string(), now);
    let seq: u32 = props
        .get("SEQUENCE")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    props.insert("SEQUENCE".to_string(), (seq + 1).to_string());

    let ical = build_event_ical(&props);
    http_put(state, &args.event_url, &ical).await
}

pub async fn delete_event(state: &AppState, args: &DeleteEventArgs) -> Result<(), CaldavError> {
    http_delete(state, &args.event_url).await
}

pub async fn list_tasks(
    state: &AppState,
    args: &ListTasksArgs,
) -> Result<Vec<TaskInfo>, CaldavError> {
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VTODO"/>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#;

    let xml = report(state, &args.calendar_url, body).await?;
    let responses = parse_multistatus(&xml);
    let include_completed = args.include_completed.unwrap_or(false);

    let mut tasks = Vec::new();
    for (href, props) in responses {
        if let Some(ical_data) = props.get("calendar-data") {
            if !ical_data.contains("BEGIN:VTODO") {
                continue;
            }
            let p = parse_ical_component(ical_data, "VTODO");
            let status = p.get("STATUS").cloned().unwrap_or_else(|| "NEEDS-ACTION".to_string());
            if !include_completed && status == "COMPLETED" {
                continue;
            }
            let priority: u8 = p
                .get("PRIORITY")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            tasks.push(TaskInfo {
                uid: p.get("UID").cloned().unwrap_or_default(),
                url: href,
                summary: p.get("SUMMARY").cloned().unwrap_or_default(),
                description: p.get("DESCRIPTION").cloned().unwrap_or_default(),
                due: p
                    .get("DUE")
                    .map(|s| ical_dt_to_iso(s))
                    .unwrap_or_default(),
                status,
                priority,
            });
        }
    }
    Ok(tasks)
}

pub async fn create_task(
    state: &AppState,
    args: &CreateTaskArgs,
) -> Result<TaskInfo, CaldavError> {
    let uid = Uuid::new_v4().to_string();
    let now = now_ical();

    let mut props = HashMap::new();
    props.insert("UID".to_string(), uid.clone());
    props.insert("SUMMARY".to_string(), args.summary.clone());
    props.insert(
        "DESCRIPTION".to_string(),
        args.description.clone().unwrap_or_default(),
    );
    if let Some(due) = &args.due {
        let (dt, _) = iso_to_ical_dt(due)?;
        props.insert("DUE".to_string(), dt);
    }
    props.insert("STATUS".to_string(), "NEEDS-ACTION".to_string());
    props.insert(
        "PRIORITY".to_string(),
        args.priority.unwrap_or(0).to_string(),
    );
    props.insert("DTSTAMP".to_string(), now.clone());
    props.insert("LAST-MODIFIED".to_string(), now);
    props.insert("SEQUENCE".to_string(), "0".to_string());

    let ical = build_task_ical(&props);
    let url = format!(
        "{}{}.ics",
        args.calendar_url.trim_end_matches('/').to_string() + "/",
        uid
    );
    http_put(state, &url, &ical).await?;

    Ok(TaskInfo {
        uid,
        url,
        summary: args.summary.clone(),
        description: args.description.clone().unwrap_or_default(),
        due: args.due.clone().unwrap_or_default(),
        status: "NEEDS-ACTION".to_string(),
        priority: args.priority.unwrap_or(0),
    })
}

pub async fn update_task(
    state: &AppState,
    args: &UpdateTaskArgs,
) -> Result<(), CaldavError> {
    let existing_ical = http_get(state, &args.task_url).await?;
    let mut props = parse_ical_component(&existing_ical, "VTODO");

    if let Some(s) = &args.summary {
        props.insert("SUMMARY".to_string(), s.clone());
    }
    if let Some(s) = &args.description {
        props.insert("DESCRIPTION".to_string(), s.clone());
    }
    if let Some(s) = &args.due {
        let (dt, _) = iso_to_ical_dt(s)?;
        props.insert("DUE".to_string(), dt);
    }
    if let Some(s) = &args.status {
        let status = s.to_uppercase();
        let valid = ["NEEDS-ACTION", "IN-PROCESS", "COMPLETED", "CANCELLED"];
        if !valid.contains(&status.as_str()) {
            return Err(CaldavError::InvalidArg(format!("invalid status: {s}")));
        }
        props.insert("STATUS".to_string(), status);
    }
    if let Some(p) = args.priority {
        props.insert("PRIORITY".to_string(), p.to_string());
    }

    let now = now_ical();
    props.insert("LAST-MODIFIED".to_string(), now.clone());
    props.insert("DTSTAMP".to_string(), now);
    let seq: u32 = props
        .get("SEQUENCE")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    props.insert("SEQUENCE".to_string(), (seq + 1).to_string());

    let ical = build_task_ical(&props);
    http_put(state, &args.task_url, &ical).await
}

pub async fn delete_task(state: &AppState, args: &DeleteTaskArgs) -> Result<(), CaldavError> {
    http_delete(state, &args.task_url).await
}
