use std::collections::HashMap;

use chrono::Utc;
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader;
use reqwest::Method;
use thiserror::Error;
use uuid::Uuid;

use crate::models::{
    AddressBookInfo, AppState, CalendarInfo, ContactInfo, CreateContactArgs, CreateEventArgs,
    CreateTaskArgs, DeleteContactArgs, DeleteEventArgs, DeleteTaskArgs, EventInfo,
    ListContactsArgs, ListEventsArgs, ListTasksArgs, TaskInfo, UpdateContactArgs, UpdateEventArgs,
    UpdateTaskArgs,
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
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "response" {
                    in_response = true;
                    current_href.clear();
                    current_props.clear();
                }
                current_tag = name;
                current_text.clear();
            }
            Ok(XmlEvent::Empty(ref e)) if in_response => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "calendar" {
                    current_props.insert("__is_calendar__".to_string(), "1".to_string());
                } else if name == "addressbook" {
                    current_props.insert("__is_addressbook__".to_string(), "1".to_string());
                }
            }
            Ok(XmlEvent::Text(ref e)) if in_response => {
                if let Ok(t) = e.unescape() {
                    current_text.push_str(&t);
                }
            }
            Ok(XmlEvent::CData(ref e)) if in_response => {
                current_text.push_str(&String::from_utf8_lossy(e.as_ref()));
            }
            Ok(XmlEvent::End(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
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
        "UID",
        "DTSTART",
        "DTEND",
        "SUMMARY",
        "DESCRIPTION",
        "LOCATION",
        "DTSTAMP",
        "LAST-MODIFIED",
        "SEQUENCE",
        "STATUS",
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
        "UID",
        "SUMMARY",
        "DESCRIPTION",
        "DUE",
        "STATUS",
        "PRIORITY",
        "DTSTAMP",
        "LAST-MODIFIED",
        "SEQUENCE",
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

async fn propfind(
    state: &AppState,
    url: &str,
    depth: &str,
    body: &str,
) -> Result<String, CaldavError> {
    dav_request(state, "PROPFIND", url, &[("Depth", depth)], Some(body)).await
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

fn address_book_home_url(state: &AppState) -> String {
    format!(
        "{}/addressbooks/{}/",
        state.caldav_base_url.trim_end_matches('/'),
        state.username
    )
}

async fn http_put_vcard(state: &AppState, url: &str, vcard: &str) -> Result<(), CaldavError> {
    dav_request(
        state,
        "PUT",
        url,
        &[("Content-Type", "text/vcard; charset=utf-8")],
        Some(vcard),
    )
    .await
    .map(|_| ())
}

fn build_vcard(props: &HashMap<String, String>) -> String {
    let mut lines = vec!["BEGIN:VCARD".to_string(), "VERSION:3.0".to_string()];
    for key in &["UID", "FN", "EMAIL", "TEL", "ORG", "NOTE", "REV"] {
        if let Some(val) = props.get(*key) {
            if !val.is_empty() {
                lines.push(format!("{key}:{val}"));
            }
        }
    }
    lines.push("END:VCARD".to_string());
    lines.join("\r\n")
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

pub async fn update_event(state: &AppState, args: &UpdateEventArgs) -> Result<(), CaldavError> {
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
            let status = p
                .get("STATUS")
                .cloned()
                .unwrap_or_else(|| "NEEDS-ACTION".to_string());
            if !include_completed && status == "COMPLETED" {
                continue;
            }
            let priority: u8 = p.get("PRIORITY").and_then(|s| s.parse().ok()).unwrap_or(0);
            tasks.push(TaskInfo {
                uid: p.get("UID").cloned().unwrap_or_default(),
                url: href,
                summary: p.get("SUMMARY").cloned().unwrap_or_default(),
                description: p.get("DESCRIPTION").cloned().unwrap_or_default(),
                due: p.get("DUE").map(|s| ical_dt_to_iso(s)).unwrap_or_default(),
                status,
                priority,
            });
        }
    }
    Ok(tasks)
}

pub async fn create_task(state: &AppState, args: &CreateTaskArgs) -> Result<TaskInfo, CaldavError> {
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

pub async fn update_task(state: &AppState, args: &UpdateTaskArgs) -> Result<(), CaldavError> {
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

// ---------------------------------------------------------------------------
// CardDAV public API
// ---------------------------------------------------------------------------

pub async fn list_address_books(state: &AppState) -> Result<Vec<AddressBookInfo>, CaldavError> {
    let url = address_book_home_url(state);
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:prop><d:displayname/><d:resourcetype/><c:addressbook-description/></d:prop>
</d:propfind>"#;
    let xml = propfind(state, &url, "1", body).await?;
    let responses = parse_multistatus(&xml);
    Ok(responses
        .into_iter()
        .filter(|(_, props)| props.contains_key("__is_addressbook__"))
        .map(|(href, props)| AddressBookInfo {
            url: href,
            name: props.get("displayname").cloned().unwrap_or_default(),
            description: props
                .get("addressbook-description")
                .cloned()
                .unwrap_or_default(),
        })
        .collect())
}

pub async fn list_contacts(
    state: &AppState,
    args: &ListContactsArgs,
) -> Result<Vec<ContactInfo>, CaldavError> {
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<c:addressbook-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:prop><d:getetag/><c:address-data/></d:prop>
</c:addressbook-query>"#;
    let xml = report(state, &args.address_book_url, body).await?;
    let responses = parse_multistatus(&xml);
    let mut contacts = Vec::new();
    for (href, props) in responses {
        if let Some(vcard_data) = props.get("address-data") {
            if !vcard_data.contains("BEGIN:VCARD") {
                continue;
            }
            let p = parse_ical_component(vcard_data, "VCARD");
            contacts.push(ContactInfo {
                uid: p.get("UID").cloned().unwrap_or_default(),
                url: href,
                full_name: p.get("FN").cloned().unwrap_or_default(),
                email: p.get("EMAIL").cloned().unwrap_or_default(),
                phone: p.get("TEL").cloned().unwrap_or_default(),
                organization: p.get("ORG").cloned().unwrap_or_default(),
                notes: p.get("NOTE").cloned().unwrap_or_default(),
            });
        }
    }
    Ok(contacts)
}

pub async fn create_contact(
    state: &AppState,
    args: &CreateContactArgs,
) -> Result<ContactInfo, CaldavError> {
    let uid = Uuid::new_v4().to_string();
    let mut props = HashMap::new();
    props.insert("UID".to_string(), uid.clone());
    props.insert("FN".to_string(), args.full_name.clone());
    props.insert("EMAIL".to_string(), args.email.clone().unwrap_or_default());
    props.insert("TEL".to_string(), args.phone.clone().unwrap_or_default());
    props.insert(
        "ORG".to_string(),
        args.organization.clone().unwrap_or_default(),
    );
    props.insert("NOTE".to_string(), args.notes.clone().unwrap_or_default());
    props.insert(
        "REV".to_string(),
        Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
    );
    let vcard = build_vcard(&props);
    let url = format!(
        "{}{}.vcf",
        args.address_book_url.trim_end_matches('/').to_string() + "/",
        uid
    );
    http_put_vcard(state, &url, &vcard).await?;
    Ok(ContactInfo {
        uid,
        url,
        full_name: args.full_name.clone(),
        email: args.email.clone().unwrap_or_default(),
        phone: args.phone.clone().unwrap_or_default(),
        organization: args.organization.clone().unwrap_or_default(),
        notes: args.notes.clone().unwrap_or_default(),
    })
}

pub async fn update_contact(state: &AppState, args: &UpdateContactArgs) -> Result<(), CaldavError> {
    let existing = http_get(state, &args.contact_url).await?;
    let mut props = parse_ical_component(&existing, "VCARD");
    if let Some(v) = &args.full_name {
        props.insert("FN".to_string(), v.clone());
    }
    if let Some(v) = &args.email {
        props.insert("EMAIL".to_string(), v.clone());
    }
    if let Some(v) = &args.phone {
        props.insert("TEL".to_string(), v.clone());
    }
    if let Some(v) = &args.organization {
        props.insert("ORG".to_string(), v.clone());
    }
    if let Some(v) = &args.notes {
        props.insert("NOTE".to_string(), v.clone());
    }
    props.insert(
        "REV".to_string(),
        Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
    );
    let vcard = build_vcard(&props);
    http_put_vcard(state, &args.contact_url, &vcard).await
}

pub async fn delete_contact(state: &AppState, args: &DeleteContactArgs) -> Result<(), CaldavError> {
    http_delete(state, &args.contact_url).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(base_url: &str, username: &str) -> AppState {
        AppState {
            caldav_base_url: base_url.to_string(),
            username: username.to_string(),
            password: "secret".to_string(),
            http_client: reqwest::Client::new(),
        }
    }

    // --- build_vcard tests ---

    #[test]
    fn test_build_vcard_all_fields() {
        let mut props = HashMap::new();
        props.insert("UID".to_string(), "test-uid-123".to_string());
        props.insert("FN".to_string(), "Jane Doe".to_string());
        props.insert("EMAIL".to_string(), "jane@example.com".to_string());
        props.insert("TEL".to_string(), "+1-555-0100".to_string());
        props.insert("ORG".to_string(), "Acme Corp".to_string());
        props.insert("NOTE".to_string(), "Some notes".to_string());
        props.insert("REV".to_string(), "20260101T000000Z".to_string());

        let vcard = build_vcard(&props);
        assert!(vcard.contains("BEGIN:VCARD"), "missing BEGIN:VCARD");
        assert!(vcard.contains("VERSION:3.0"), "missing VERSION:3.0");
        assert!(vcard.contains("FN:Jane Doe"), "missing FN");
        assert!(vcard.contains("EMAIL:jane@example.com"), "missing EMAIL");
        assert!(vcard.contains("TEL:+1-555-0100"), "missing TEL");
        assert!(vcard.contains("ORG:Acme Corp"), "missing ORG");
        assert!(vcard.contains("NOTE:Some notes"), "missing NOTE");
        assert!(vcard.contains("END:VCARD"), "missing END:VCARD");
    }

    #[test]
    fn test_build_vcard_empty_optional_fields_omitted() {
        let mut props = HashMap::new();
        props.insert("UID".to_string(), "uid-only".to_string());
        props.insert("FN".to_string(), "John Smith".to_string());
        // EMAIL, TEL, ORG, NOTE intentionally absent
        let vcard = build_vcard(&props);
        assert!(!vcard.contains("EMAIL:"), "empty EMAIL should be omitted");
        assert!(!vcard.contains("TEL:"), "empty TEL should be omitted");
        assert!(!vcard.contains("ORG:"), "empty ORG should be omitted");
        assert!(!vcard.contains("NOTE:"), "empty NOTE should be omitted");
        assert!(vcard.contains("FN:John Smith"));
    }

    #[test]
    fn test_build_vcard_empty_string_fields_omitted() {
        let mut props = HashMap::new();
        props.insert("UID".to_string(), "uid-empty".to_string());
        props.insert("FN".to_string(), "Alice".to_string());
        props.insert("EMAIL".to_string(), "".to_string());
        props.insert("TEL".to_string(), "".to_string());
        let vcard = build_vcard(&props);
        assert!(
            !vcard.contains("EMAIL:"),
            "empty string EMAIL should be omitted"
        );
        assert!(
            !vcard.contains("TEL:"),
            "empty string TEL should be omitted"
        );
    }

    #[test]
    fn test_build_vcard_crlf_line_endings() {
        let mut props = HashMap::new();
        props.insert("FN".to_string(), "Test".to_string());
        let vcard = build_vcard(&props);
        assert!(vcard.contains("\r\n"), "lines should be joined with CRLF");
        // Verify begin and end are separated by CRLF
        assert!(vcard.starts_with("BEGIN:VCARD\r\n"));
        assert!(vcard.ends_with("\r\nEND:VCARD"));
    }

    // --- address_book_home_url tests ---

    #[test]
    fn test_address_book_home_url_basic() {
        let state = make_state("http://baikal/dav.php", "admin");
        let url = address_book_home_url(&state);
        assert_eq!(url, "http://baikal/dav.php/addressbooks/admin/");
    }

    #[test]
    fn test_address_book_home_url_trailing_slash_trimmed() {
        let state = make_state("http://baikal/dav.php/", "admin");
        let url = address_book_home_url(&state);
        assert_eq!(url, "http://baikal/dav.php/addressbooks/admin/");
    }

    #[test]
    fn test_address_book_home_url_custom_user() {
        let state = make_state("https://dav.example.com", "shiloh");
        let url = address_book_home_url(&state);
        assert_eq!(url, "https://dav.example.com/addressbooks/shiloh/");
    }

    // --- parse_ical_component with VCARD tests ---

    #[test]
    fn test_parse_vcard_basic_fields() {
        let vcard = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:uid-abc\r\nFN:Bob Builder\r\nEMAIL:bob@example.com\r\nTEL:555-1234\r\nORG:BuildCo\r\nNOTE:A note\r\nEND:VCARD\r\n";
        let props = parse_ical_component(vcard, "VCARD");
        assert_eq!(props.get("FN").map(|s| s.as_str()), Some("Bob Builder"));
        assert_eq!(
            props.get("EMAIL").map(|s| s.as_str()),
            Some("bob@example.com")
        );
        assert_eq!(props.get("TEL").map(|s| s.as_str()), Some("555-1234"));
        assert_eq!(props.get("ORG").map(|s| s.as_str()), Some("BuildCo"));
        assert_eq!(props.get("NOTE").map(|s| s.as_str()), Some("A note"));
        assert_eq!(props.get("UID").map(|s| s.as_str()), Some("uid-abc"));
    }

    #[test]
    fn test_parse_vcard_email_with_params() {
        // EMAIL;TYPE=INTERNET:foo@bar.com — base key should be EMAIL
        let vcard = "BEGIN:VCARD\r\nVERSION:3.0\r\nEMAIL;TYPE=INTERNET:foo@bar.com\r\nFN:Test\r\nEND:VCARD\r\n";
        let props = parse_ical_component(vcard, "VCARD");
        assert_eq!(
            props.get("EMAIL").map(|s| s.as_str()),
            Some("foo@bar.com"),
            "EMAIL with params should strip the param part from the key"
        );
    }

    #[test]
    fn test_parse_vcard_no_begin_returns_empty() {
        let not_vcard = "FN:Someone\r\nEMAIL:x@y.com\r\n";
        let props = parse_ical_component(not_vcard, "VCARD");
        assert!(
            props.is_empty(),
            "should return empty map when BEGIN:VCARD not found"
        );
    }

    // --- parse_multistatus addressbook detection tests ---

    #[test]
    fn test_parse_multistatus_addressbook_flag() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:response>
    <d:href>/addressbooks/admin/default/</d:href>
    <d:propstat>
      <d:prop>
        <d:displayname>Default</d:displayname>
        <d:resourcetype><d:collection/><c:addressbook/></d:resourcetype>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;
        let responses = parse_multistatus(xml);
        assert_eq!(responses.len(), 1);
        let (_, props) = &responses[0];
        assert!(
            props.contains_key("__is_addressbook__"),
            "should detect addressbook resource type"
        );
        assert!(
            !props.contains_key("__is_calendar__"),
            "should not flag as calendar"
        );
    }

    #[test]
    fn test_parse_multistatus_calendar_flag_still_works() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:response>
    <d:href>/calendars/admin/default/</d:href>
    <d:propstat>
      <d:prop>
        <d:displayname>My Cal</d:displayname>
        <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;
        let responses = parse_multistatus(xml);
        assert_eq!(responses.len(), 1);
        let (_, props) = &responses[0];
        assert!(
            props.contains_key("__is_calendar__"),
            "should still detect calendar resource type"
        );
        assert!(
            !props.contains_key("__is_addressbook__"),
            "should not flag as addressbook"
        );
    }

    #[test]
    fn test_parse_multistatus_mixed_calendar_and_addressbook() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:" xmlns:ca="urn:ietf:params:xml:ns:caldav" xmlns:cd="urn:ietf:params:xml:ns:carddav">
  <d:response>
    <d:href>/calendars/admin/cal/</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype><d:collection/><ca:calendar/></d:resourcetype>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/addressbooks/admin/book/</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype><d:collection/><cd:addressbook/></d:resourcetype>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;
        let responses = parse_multistatus(xml);
        assert_eq!(responses.len(), 2);
        let cal = responses
            .iter()
            .find(|(href, _)| href.contains("cal"))
            .unwrap();
        let book = responses
            .iter()
            .find(|(href, _)| href.contains("book"))
            .unwrap();
        assert!(cal.1.contains_key("__is_calendar__"));
        assert!(!cal.1.contains_key("__is_addressbook__"));
        assert!(book.1.contains_key("__is_addressbook__"));
        assert!(!book.1.contains_key("__is_calendar__"));
    }
}
