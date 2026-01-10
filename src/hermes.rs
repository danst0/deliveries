use anyhow::{anyhow, bail, Result};
use headless_chrome::Browser;
use std::time::Duration;
use crate::dhl::TrackingEvent;

pub fn fetch_tracking_events(tracking_id: &str) -> Result<Vec<TrackingEvent>> {
    let url = format!(
        "https://www.myhermes.de/empfangen/sendungsverfolgung/sendungsinformation/#{}",
        tracking_id
    );

    let browser = Browser::default()
        .map_err(|e| anyhow!("Failed to launch headless Chrome: {}", e))?;

    let tab = browser
        .new_tab()
        .map_err(|e| anyhow!("Failed to open new Chrome tab: {}", e))?;

    tab.navigate_to(&url)
        .map_err(|e| anyhow!("Failed to navigate to Hermes tracking page: {}", e))?;

    tab.wait_for_element_with_custom_timeout("body", Duration::from_secs(15))
        .map_err(|e| anyhow!("Timed out waiting for tracking page: {}", e))?;

    // Attempt to close cookie banner
    let _ = tab.evaluate(
        r#"
        const bannerButtons = document.querySelectorAll("button");
        for (const btn of bannerButtons) {
            if (btn.innerText.includes("Akzeptieren") || btn.innerText.includes("Alles akzeptieren")) {
                btn.click();
                break;
            }
        }
        "#,
        false
    );

    // Small delay for rendering
    std::thread::sleep(Duration::from_secs(2));

    let events_json = tab.evaluate(
        r#"
        (() => {
            const bodyText = document.body.innerText;
            const lines = bodyText.split('\n').map(l => l.trim()).filter(l => l.length > 0);
            const events = [];
            
            // Regex for date: e.g. Montag, 29.12.2025, 07:41 Uhr
            // Matches: Word, DD.MM.YYYY, HH:MM Uhr
            const dateRegex = /^\w+,\s+(\d{2}\.\d{2}\.\d{4}),\s+(\d{2}:\d{2})\s+Uhr$/;

            for (let i = 0; i < lines.length; i++) {
                const match = lines[i].match(dateRegex);
                if (match) {
                    let status = "Status unknown";
                    if (i + 1 < lines.length) {
                         status = lines[i+1];
                    }
                    
                    events.push({
                        date: match[1],
                        time: match[2],
                        description: status,
                        location: "" // Location is often mixed in description or not available separately
                    });
                }
            }
            return events;
        })()
        "#,
        true
    ).map_err(|e| anyhow!("Failed to evaluate extraction script: {}", e))?;

    let value = events_json.value.ok_or_else(|| anyhow!("No value returned from script"))?;

    #[derive(serde::Deserialize)]
    struct RawEvent {
        date: String,
        time: String,
        description: String,
        #[serde(default)]
        location: String,
    }

    let raw_events: Vec<RawEvent> = serde_json::from_value(value)
        .map_err(|e| anyhow!("Failed to parse script output: {}", e))?;

    if raw_events.is_empty() {
         bail!("No events found on Hermes page");
    }

    let events: Vec<TrackingEvent> = raw_events.into_iter().map(|raw| {
        TrackingEvent {
            timestamp: format!("{}, {}", raw.date, raw.time),
            location: if raw.location.is_empty() { "Germany".to_string() } else { raw.location },
            description: raw.description,
        }
    }).collect();

    Ok(events)
}
