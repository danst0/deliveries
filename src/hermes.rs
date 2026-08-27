use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::time::Duration;
use crate::dhl::{build_browser, TrackingEvent};

pub fn fetch_tracking_events(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let trimmed = tracking_id.trim();
    if trimmed.is_empty() {
        bail!("Tracking number is empty");
    }

    fetch_via_headless(trimmed)
}

fn fetch_via_headless(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let url = format!(
        "https://www.myhermes.de/empfangen/sendungsverfolgung/sendungsinformation/#{}",
        tracking_id
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
