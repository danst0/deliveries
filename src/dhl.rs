use anyhow::{anyhow, bail, Context, Result};
use headless_chrome::Browser;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct TrackingEvent {
    pub timestamp: String,
    pub location: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    sendungen: Vec<Shipment>,
}

#[derive(Debug, Deserialize)]
struct Shipment {
    #[serde(default)]
    sendungsdetails: Option<Sendungsdetails>,
}

#[derive(Debug, Deserialize)]
struct Sendungsdetails {
    #[serde(default)]
    sendungsverlauf: Option<Sendungsverlauf>,
}

#[derive(Debug, Deserialize)]
struct Sendungsverlauf {
    #[serde(default)]
    events: Option<Vec<Event>>, // timeline entries
}

#[derive(Debug, Deserialize)]
struct Event {
    #[serde(default)]
    datum: Option<String>,
    #[serde(default)]
    ort: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Fetch DHL tracking events via the JSON endpoint backing the public tracker.
pub fn fetch_tracking_events(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let trimmed = tracking_id.trim();
    if trimmed.is_empty() {
        bail!("Tracking number is empty");
    }

    match fetch_via_api(trimmed) {
        Ok(events) => Ok(events),
        Err(api_err) => {
            let fallback = fetch_via_headless(trimmed)
                .map_err(|headless_err| anyhow!("API failed: {}; headless Chrome failed: {}", api_err, headless_err))?;

            if fallback.is_empty() {
                bail!("No events available after API and headless fallback attempts");
            }

            Ok(fallback)
        }
    }
}

fn fetch_via_api(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let url = format!(
        "https://www.dhl.de/int-verfolgen/data/search?piececode={}&noRedirect=true",
        tracking_id
    );

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36")
        .build()
        .context("Failed to build HTTP client")?;

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Accept-Language", "de-DE,de;q=0.9,en;q=0.8")
        .send()
        .context("Network request failed")?
        .error_for_status()
        .context("Tracking server returned an error status")?;

    let parsed: ApiResponse = response.json().context("Failed to parse tracking JSON")?;

    let shipment = parsed
        .sendungen
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No shipment data returned"))?;

    let events = shipment
        .sendungsdetails
        .and_then(|d| d.sendungsverlauf)
        .and_then(|v| v.events)
        .unwrap_or_default();

    if events.is_empty() {
        bail!("No events available for this tracking number");
    }

    Ok(events
        .into_iter()
        .map(|ev| TrackingEvent {
            timestamp: ev.datum.unwrap_or_else(|| "Time not available".to_string()),
            location: ev.ort.unwrap_or_else(|| "Location not available".to_string()),
            description: ev.status.unwrap_or_else(|| "No description available".to_string()),
        })
        .collect())
}

fn fetch_via_headless(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let url = format!(
        "https://www.dhl.de/de/privatkunden/pakete-empfangen/verfolgen.html?piececode={}",
        tracking_id
    );

    let browser = Browser::default()
        .map_err(|e| anyhow!("Failed to launch headless Chrome (is Chrome/Chromium available?): {}", e))?;

    let tab = browser
        .new_tab()
        .map_err(|e| anyhow!("Failed to open new Chrome tab: {}", e))?;
    tab.navigate_to(&url)
        .map_err(|e| anyhow!("Failed to navigate to DHL tracking page: {}", e))?;
    tab.wait_for_element_with_custom_timeout("body", Duration::from_secs(15))
        .map_err(|e| anyhow!("Timed out waiting for tracking page to load: {}", e))?;

    // Best-effort cookie/consent acknowledgement if present.
    for selector in [
        "#onetrust-accept-btn-handler",
        "button[aria-label='Einverstanden']",
        "button[mode='primary']",
        "button[title='Alle akzeptieren']",
    ] {
        if let Ok(el) = tab.wait_for_element_with_custom_timeout(selector, Duration::from_secs(3)) {
            // Ignore consent click errors; continue with the first clickable element.
            let _: Option<_> = el.click().ok();
            break;
        }
    }

    // Pull a lightweight snapshot from visible text instead of DOM-specific selectors that may change.
    let snapshot: Value = tab
                .evaluate(
            r#"
(() => {
  const text = (document.body && document.body.innerText ? document.body.innerText : "").replace(/\u00a0/g, " ");
  const statusMatch = text.match(/Aktueller Status:\s*([^\n]+)/i);
  const tsMatch = text.match(/(Mo|Di|Mi|Do|Fr|Sa|So),\s*\d{2}\.\d{2}\.\d{4},\s*\d{2}:\d{2}\s*Uhr/);
  const locMatch = text.match(/\b\d{5}\b[^\n]*/);
  return {
    status: statusMatch ? statusMatch[1].trim() : null,
    timestamp: tsMatch ? tsMatch[0].trim() : null,
    location_hint: locMatch ? locMatch[0].trim() : null,
  };
})()
"#,
            true,
        )
    .map_err(|e| anyhow!("Failed to evaluate tracking page: {}", e))?
    .value
    .ok_or_else(|| anyhow!("Headless Chrome returned no data"))?;

    let status = snapshot
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_string);
    let timestamp = snapshot
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    let location = snapshot
        .get("location_hint")
        .and_then(Value::as_str)
        .map(str::to_string);

    if status.is_none() && timestamp.is_none() {
        bail!("Headless extraction returned no tracking details");
    }

    Ok(vec![TrackingEvent {
        timestamp: timestamp.unwrap_or_else(|| "Zeit nicht verfügbar".to_string()),
        location: location.unwrap_or_else(|| "Ort nicht verfügbar".to_string()),
        description: status.unwrap_or_else(|| "Status nicht verfügbar".to_string()),
    }])
}
