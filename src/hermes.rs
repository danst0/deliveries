use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, ORIGIN, REFERER};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

use crate::dhl::{build_browser, TrackingEvent};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";
const SEARCH_URL: &str = "https://api.my-deliveries.de/tnt/v2/shipments/search/";
const SITE: &str = "https://www.myhermes.de";

#[derive(Debug, Deserialize)]
struct Shipment {
    #[serde(default, rename = "parcelProgress")]
    parcel_progress: Option<Vec<Progress>>,
}

#[derive(Debug, Deserialize)]
struct Progress {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default, rename = "historyText")]
    history_text: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

pub fn fetch_tracking_events(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let trimmed = tracking_id.trim();
    if trimmed.is_empty() {
        bail!("Tracking number is empty");
    }

    match fetch_via_api(trimmed) {
        Ok(events) => Ok(events),
        // Headless Chrome cannot run inside the Flatpak sandbox, so this only rescues
        // native builds; the API error is what matters when it is unavailable.
        Err(api_err) => fetch_via_headless(trimmed).map_err(|headless_err| {
            anyhow!(
                "API failed: {}; headless Chrome failed: {}",
                api_err,
                headless_err
            )
        }),
    }
}

/// Hermes' tracking page reads this JSON endpoint; it needs no browser.
fn fetch_via_api(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let client = Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .build()
        .context("Failed to build HTTP client")?;

    let response = client
        .get(format!("{}{}", SEARCH_URL, tracking_id))
        .header(ACCEPT, "application/json")
        .header("X-Language", "de")
        .header(ORIGIN, SITE)
        .header(REFERER, format!("{}/", SITE))
        .send()
        .context("Network request failed")?;

    if response.status() == StatusCode::NOT_FOUND {
        bail!("Hermes holds no data for this tracking number");
    }

    let response = response
        .error_for_status()
        .context("Tracking server returned an error status")?;

    let shipments: Vec<Shipment> = response.json().context("Failed to parse tracking JSON")?;

    let shipment = shipments
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No shipment data returned"))?;

    let mut progress = shipment.parcel_progress.unwrap_or_default();
    if progress.is_empty() {
        bail!("No events available for this tracking number");
    }

    // The site sorts the timeline oldest-first; ISO timestamps sort lexicographically.
    progress.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    Ok(progress
        .into_iter()
        .map(|entry| TrackingEvent {
            timestamp: entry
                .timestamp
                .unwrap_or_else(|| "Time not available".to_string()),
            location: "Location not available".to_string(),
            description: entry
                .history_text
                .or(entry.status)
                .unwrap_or_else(|| "No description available".to_string()),
        })
        .collect())
}

fn fetch_via_headless(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let url = format!(
        "{}/empfangen/sendungsverfolgung/sendungsinformation/#{}",
        SITE, tracking_id
    );

    let browser = build_browser()?;

    let tab = browser
        .new_tab()
        .map_err(|e| anyhow!("Failed to open new Chrome tab: {}", e))?;
    tab.navigate_to(&url)
        .map_err(|e| anyhow!("Failed to navigate to Hermes tracking page: {}", e))?;
    tab.wait_for_element_with_custom_timeout("body", Duration::from_secs(15))
        .map_err(|e| anyhow!("Timed out waiting for tracking page to load: {}", e))?;

    // Best-effort cookie/consent acknowledgement if present.
    for selector in [
        "button[aria-label='Einverstanden']",
        "button[mode='primary']",
        "button[title='Alle akzeptieren']",
        "button",
    ] {
        if let Ok(el) = tab.wait_for_element_with_custom_timeout(selector, Duration::from_secs(3)) {
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
  const statusMatch = text.match(/Status\s*[:\-]?\s*([^\n]+)/i);
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
