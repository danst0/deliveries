use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use reqwest::blocking::Client;
use scraper::{ElementRef, Html, Selector};

#[derive(Clone, Debug)]
pub struct TrackingEvent {
    pub timestamp: String,
    pub location: String,
    pub description: String,
}

/// Fetch DHL tracking events by scraping the public tracking page.
pub fn fetch_tracking_events(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let trimmed = tracking_id.trim();
    if trimmed.is_empty() {
        bail!("Tracking number is empty");
    }

    let url = format!(
        "https://nolp.dhl.de/nextt-online-public/report?lang=en&id={}",
        trimmed
    );

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (compatible; deliveries-tracker/0.1; +https://example.invalid)")
        .build()
        .context("Failed to build HTTP client")?;

    let response = client
        .get(&url)
        .send()
        .context("Network request failed")?
        .error_for_status()
        .context("Tracking server returned an error status")?;

    let body = response.text().context("Failed to read response body")?;
    let events = parse_html(&body);

    if events.is_empty() {
        bail!("Could not parse tracking details; DHL markup may have changed");
    }

    Ok(events)
}

fn parse_html(html: &str) -> Vec<TrackingEvent> {
    let doc = Html::parse_document(html);

    if let Some(events) = parse_cards(&doc) {
        return events;
    }

    if let Some(events) = parse_table(&doc) {
        return events;
    }

    Vec::new()
}

fn parse_cards(doc: &Html) -> Option<Vec<TrackingEvent>> {
    static EVENT_CARD: Lazy<Selector> = Lazy::new(|| Selector::parse("div.event, li.event, div.status, li.status").unwrap());
    static DATE_SEL: Lazy<Selector> = Lazy::new(|| Selector::parse(".status-date, time, .date").unwrap());
    static LOC_SEL: Lazy<Selector> = Lazy::new(|| Selector::parse(".status-location, .place, .location").unwrap());
    static DESC_SEL: Lazy<Selector> = Lazy::new(|| Selector::parse(".status-text, .status-description, p, .text").unwrap());

    let mut events = Vec::new();

    for node in doc.select(&EVENT_CARD) {
        let timestamp = first_text(&node, &DATE_SEL);
        let location = first_text(&node, &LOC_SEL);
        let description = first_text(&node, &DESC_SEL);

        if timestamp.is_empty() && description.is_empty() {
            continue;
        }

        events.push(TrackingEvent {
            timestamp: if timestamp.is_empty() { "Time not available".to_string() } else { timestamp },
            location: if location.is_empty() { "Location not available".to_string() } else { location },
            description: if description.is_empty() {
                "No description available".to_string()
            } else {
                description
            },
        });
    }

    if events.is_empty() {
        None
    } else {
        Some(events)
    }
}

fn parse_table(doc: &Html) -> Option<Vec<TrackingEvent>> {
    static ROW_SEL: Lazy<Selector> = Lazy::new(|| Selector::parse("table tr").unwrap());
    static CELL_SEL: Lazy<Selector> = Lazy::new(|| Selector::parse("td").unwrap());

    let mut events = Vec::new();

    for row in doc.select(&ROW_SEL) {
        let mut cells = row.select(&CELL_SEL);
        let first = cells.next().map(extract_text).unwrap_or_default();
        let second = cells.next().map(extract_text).unwrap_or_default();
        let third = cells.next().map(extract_text).unwrap_or_default();

        if first.is_empty() && second.is_empty() && third.is_empty() {
            continue;
        }

        // Many DHL tables are date/time | location | description
        // If only two columns exist, treat them as timestamp and description.
        let (timestamp, location, description) = if third.is_empty() {
            (first, String::new(), second)
        } else {
            (first, second, third)
        };

        events.push(TrackingEvent {
            timestamp: if timestamp.is_empty() { "Time not available".to_string() } else { timestamp },
            location: if location.is_empty() { "Location not available".to_string() } else { location },
            description: if description.is_empty() {
                "No description available".to_string()
            } else {
                description
            },
        });
    }

    if events.is_empty() {
        None
    } else {
        Some(events)
    }
}

fn first_text(node: &ElementRef<'_>, selector: &Selector) -> String {
    node
        .select(selector)
        .next()
        .map(extract_text)
        .unwrap_or_default()
}

fn extract_text(node: ElementRef<'_>) -> String {
    node.text().collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}
