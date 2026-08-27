use anyhow::{anyhow, bail, Context, Result};
use headless_chrome::{Browser, LaunchOptionsBuilder};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, ACCEPT, ACCEPT_LANGUAGE, COOKIE, REFERER, SET_COOKIE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";
const DATA_BASE: &str = "https://www.dhl.de/int-verfolgen/data";
const REFERER_URL: &str = "https://www.dhl.de/de/privatkunden/pakete-empfangen/verfolgen.html";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackingEvent {
    pub timestamp: String,
    pub location: String,
    pub description: String,
}

/// Everything the DHL endpoint reports beyond the plain event list.
#[derive(Clone, Debug, Default)]
pub struct TrackingDetails {
    pub events: Vec<TrackingEvent>,
    /// DHL withholds part of the detail view until the recipient PLZ is supplied.
    pub censored: bool,
    pub plz_required: bool,
    pub versanddatum_required: bool,
    pub delivered: bool,
    pub progress: Option<(u32, u32)>,
    pub short_status: Option<String>,
    pub delivery_window: Option<(String, String)>,
    /// Set when a PLZ was supplied but DHL refused it; the anonymous view is kept.
    pub zip_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[serde(default)]
    sendungen: Vec<Shipment>,
}

#[derive(Debug, Deserialize)]
struct Shipment {
    #[serde(default)]
    sendungsdetails: Option<Sendungsdetails>,
    #[serde(default, rename = "plzBenoetigt")]
    plz_benoetigt: bool,
    #[serde(default, rename = "versandDatumBenoetigt")]
    versand_datum_benoetigt: bool,
    #[serde(default, rename = "sendungNichtGefunden")]
    sendung_nicht_gefunden: Option<SendungNichtGefunden>,
}

#[derive(Debug, Deserialize)]
struct SendungNichtGefunden {
    #[serde(default, rename = "keineDatenVerfuegbar")]
    keine_daten_verfuegbar: bool,
    #[serde(default, rename = "sendungsdatenZuAlt")]
    sendungsdaten_zu_alt: bool,
}

#[derive(Debug, Deserialize)]
struct Sendungsdetails {
    #[serde(default)]
    sendungsverlauf: Option<Sendungsverlauf>,
    #[serde(default)]
    zustellung: Option<Zustellung>,
    #[serde(default, rename = "hasCensoredInformation")]
    has_censored_information: bool,
    #[serde(default, rename = "istZugestellt")]
    ist_zugestellt: bool,
}

#[derive(Debug, Deserialize)]
struct Zustellung {
    #[serde(default, rename = "zustellzeitfensterVon")]
    zustellzeitfenster_von: Option<String>,
    #[serde(default, rename = "zustellzeitfensterBis")]
    zustellzeitfenster_bis: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Sendungsverlauf {
    #[serde(default)]
    events: Option<Vec<Event>>, // timeline entries
    #[serde(default)]
    status: Option<String>, // short status, e.g. "Vorbereitung für Weitertransport"
    #[serde(default)]
    fortschritt: Option<u32>,
    #[serde(default, rename = "maximalFortschritt")]
    maximal_fortschritt: Option<u32>,
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

/// Fetch DHL tracking events via the JSON endpoints backing the public tracker.
///
/// `zip` is the recipient postcode. DHL only hands out the uncensored detail view in
/// exchange for it, and only for shipments that actually withhold something.
pub fn fetch_tracking_events(tracking_id: &str, zip: Option<&str>) -> Result<TrackingDetails> {
    let trimmed = tracking_id.trim();
    if trimmed.is_empty() {
        bail!("Tracking number is empty");
    }
    let zip = zip.map(str::trim).filter(|z| !z.is_empty());

    let anonymous = match fetch_via_api(trimmed) {
        Ok(details) => details,
        Err(api_err) => {
            let fallback = fetch_via_headless(trimmed).map_err(|headless_err| {
                anyhow!("API failed: {}; headless Chrome failed: {}", api_err, headless_err)
            })?;

            if fallback.events.is_empty() {
                bail!("No events available after API and headless fallback attempts");
            }

            return Ok(fallback);
        }
    };

    // Only spend a PLZ request when DHL actually withholds something: every wrong
    // attempt counts towards a block that locks further lookups for hours.
    if !anonymous.censored && !anonymous.plz_required {
        return Ok(anonymous);
    }

    let Some(zip) = zip else { return Ok(anonymous) };

    match fetch_with_zip(trimmed, zip) {
        Ok(details) => Ok(details),
        // Keep the anonymous view rather than dropping the events we already have.
        Err(err) => Ok(TrackingDetails {
            zip_error: Some(err.to_string()),
            ..anonymous
        }),
    }
}

fn fetch_via_api(tracking_id: &str) -> Result<TrackingDetails> {
    let url = format!(
        "{}/search?piececode={}&noRedirect=true&language=de",
        DATA_BASE, tracking_id
    );

    let response = build_client()?
        .get(&url)
        .headers(common_headers())
        .send()
        .context("Network request failed")?
        .error_for_status()
        .context("Tracking server returned an error status")?;

    let parsed: ApiResponse = response.json().context("Failed to parse tracking JSON")?;

    details_from_response(parsed)
}

/// Ask for the uncensored detail view by proving knowledge of the recipient postcode.
fn fetch_with_zip(tracking_id: &str, zip: &str) -> Result<TrackingDetails> {
    let client = build_client()?;
    let (token, cookies) = fetch_session(&client)?;

    let response = client
        .post(format!("{}/shipment?language=de", DATA_BASE))
        .headers(common_headers())
        .header("verfolgen-CSRF-token", token)
        .header(COOKIE, cookies)
        .json(&json!({ "piececode": tracking_id, "zip": zip, "international": false }))
        .send()
        .context("PLZ request failed")?;

    let status = response.status();
    let body: Value = response.json().context("Failed to parse PLZ response")?;

    if !status.is_success() {
        if let Some(block) = body.get("blockTime") {
            bail!(
                "too many wrong attempts, DHL blocked PLZ lookups until {}",
                format_block_time(block)
            );
        }
        let message = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("DHL rejected the postcode");
        bail!("{}", message);
    }

    let parsed: ApiResponse =
        serde_json::from_value(body).context("Failed to parse PLZ payload")?;

    details_from_response(parsed)
}

/// The PLZ endpoint is CSRF protected; token and session cookies both come from the
/// SPA config that the tracking page loads before its first request.
fn fetch_session(client: &Client) -> Result<(String, String)> {
    let response = client
        .get(format!("{}/config?domain=de&language=de", DATA_BASE))
        .headers(common_headers())
        .send()
        .context("Failed to fetch DHL session config")?
        .error_for_status()
        .context("DHL session config returned an error status")?;

    let cookies = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");

    let config: Value = response.json().context("Failed to parse DHL session config")?;
    let token = config
        .get("verfolgenCsrfToken")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("DHL session config contained no CSRF token"))?
        .to_string();

    Ok((token, cookies))
}

fn details_from_response(parsed: ApiResponse) -> Result<TrackingDetails> {
    let shipment = parsed
        .sendungen
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No shipment data returned"))?;

    if let Some(missing) = shipment.sendung_nicht_gefunden.as_ref() {
        if missing.keine_daten_verfuegbar || missing.sendungsdaten_zu_alt {
            bail!("DHL no longer holds data for this tracking number");
        }
    }

    let verlauf = shipment
        .sendungsdetails
        .as_ref()
        .and_then(|d| d.sendungsverlauf.as_ref());
    let zustellung = shipment
        .sendungsdetails
        .as_ref()
        .and_then(|d| d.zustellung.as_ref());

    let events: Vec<TrackingEvent> = verlauf
        .and_then(|v| v.events.as_ref())
        .map(|events| {
            events
                .iter()
                .map(|ev| TrackingEvent {
                    timestamp: ev
                        .datum
                        .clone()
                        .unwrap_or_else(|| "Time not available".to_string()),
                    location: ev
                        .ort
                        .clone()
                        .filter(|ort| !ort.trim().is_empty())
                        .unwrap_or_else(|| "Location not available".to_string()),
                    description: ev
                        .status
                        .clone()
                        .unwrap_or_else(|| "No description available".to_string()),
                })
                .collect()
        })
        .unwrap_or_default();

    let details = TrackingDetails {
        events,
        censored: shipment
            .sendungsdetails
            .as_ref()
            .map_or(false, |d| d.has_censored_information),
        plz_required: shipment.plz_benoetigt,
        versanddatum_required: shipment.versand_datum_benoetigt,
        delivered: shipment
            .sendungsdetails
            .as_ref()
            .map_or(false, |d| d.ist_zugestellt),
        progress: verlauf.and_then(|v| Some((v.fortschritt?, v.maximal_fortschritt?))),
        short_status: verlauf.and_then(|v| v.status.clone()),
        delivery_window: zustellung.and_then(|z| {
            Some((
                z.zustellzeitfenster_von.clone()?,
                z.zustellzeitfenster_bis.clone()?,
            ))
        }),
        zip_error: None,
    };

    if details.events.is_empty()
        && !details.censored
        && !details.plz_required
        && !details.versanddatum_required
    {
        bail!("No events available for this tracking number");
    }

    Ok(details)
}

/// DHL reports the end of a lookup block as a unix timestamp.
fn format_block_time(value: &Value) -> String {
    let raw = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok()));

    let Some(stamp) = raw else {
        return value.to_string();
    };

    let seconds = if stamp > 100_000_000_000 {
        stamp / 1000
    } else {
        stamp
    };

    gtk4::glib::DateTime::from_unix_local(seconds)
        .ok()
        .and_then(|dt| dt.format("%H:%M").ok())
        .map(|formatted| formatted.to_string())
        .unwrap_or_else(|| stamp.to_string())
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .build()
        .context("Failed to build HTTP client")
}

fn common_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, "application/json".parse().unwrap());
    headers.insert(ACCEPT_LANGUAGE, "de-DE,de;q=0.9,en;q=0.8".parse().unwrap());
    headers.insert(REFERER, REFERER_URL.parse().unwrap());
    headers
}

/// `Browser::default()` panics when it cannot auto-detect Chrome, because it unwraps
/// the lookup. Going through `Browser::new` with an unset path resolves the executable
/// via `Process::new`, which reports the same failure as a plain error.
pub(crate) fn build_browser() -> Result<Browser> {
    let candidates = [
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/var/lib/flatpak/exports/bin/com.vivaldi.Vivaldi",
        "/var/lib/flatpak/exports/bin/org.chromium.Chromium",
        "/var/lib/flatpak/exports/bin/com.google.Chrome",
    ];

    for candidate in candidates {
        let path = std::path::Path::new(candidate);
        if !path.exists() {
            continue;
        }

        let mut builder = LaunchOptionsBuilder::default();
        builder.path(Some(path.to_path_buf()));
        builder.sandbox(false); // the sandbox is usually blocked inside Flatpak
        if let Ok(options) = builder.build() {
            if let Ok(browser) = Browser::new(options) {
                return Ok(browser);
            }
        }
    }

    let mut builder = LaunchOptionsBuilder::default();
    builder.sandbox(false);
    let options = builder
        .build()
        .map_err(|e| anyhow!("Failed to assemble Chrome launch options: {}", e))?;

    Browser::new(options).map_err(|e| {
        anyhow!(
            "no usable Chrome/Chromium binary found (is Chrome/Chromium installed?): {}",
            e
        )
    })
}

fn fetch_via_headless(tracking_id: &str) -> Result<TrackingDetails> {
    let url = format!("{}?piececode={}", REFERER_URL, tracking_id);

    let browser = build_browser()?;

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

    Ok(TrackingDetails {
        events: vec![TrackingEvent {
            timestamp: timestamp.unwrap_or_else(|| "Zeit nicht verfügbar".to_string()),
            location: location.unwrap_or_else(|| "Ort nicht verfügbar".to_string()),
            description: status.unwrap_or_else(|| "Status nicht verfügbar".to_string()),
        }],
        ..Default::default()
    })
}
