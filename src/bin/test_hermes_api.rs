use reqwest::blocking::Client;
use std::env;
use std::error::Error;
use std::fs;

fn main() -> Result<(), Box<dyn Error>> {
    let tracking_id = env::args()
        .nth(1)
        .or_else(|| fs::read_to_string("test_hermes_number.txt").ok())
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .ok_or("pass a Hermes tracking number or create test_hermes_number.txt")?;
    let url = format!(
        "https://www.myhermes.de/services/tracking/shipments?search={}",
        tracking_id
    );

    println!("Fetching {}", url);

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36")
        .build()?;

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()?;

    println!("Status: {}", response.status());
    let text = response.text()?;
    println!("Body len: {}", text.len());
    println!("Body preview: {:.1000}...", text);

    Ok(())
}
