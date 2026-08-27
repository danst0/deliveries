#[path = "../dhl.rs"]
mod dhl;

use dhl::fetch_tracking_events;
use std::env;
use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    let raw = match args.get(1) {
        Some(code) => code.clone(),
        None => fs::read_to_string("test_tracking_numbers.txt")
            .expect("pass a tracking number or create test_tracking_numbers.txt"),
    };
    let code = raw.trim();
    if code.is_empty() {
        eprintln!("Tracking number is empty");
        process::exit(1);
    }

    // Optional second argument: the recipient postcode.
    let zip = args.get(2).cloned();

    println!("Checking {}...", code);

    match fetch_tracking_events(code, zip.as_deref()) {
        Ok(details) => {
            println!(
                "censored={} plz_required={} delivered={} progress={:?}",
                details.censored, details.plz_required, details.delivered, details.progress
            );
            if let Some(status) = details.short_status.as_deref() {
                println!("status: {}", status);
            }
            if let Some((from, to)) = details.delivery_window.as_ref() {
                println!("delivery window: {} - {}", from, to);
            }
            if let Some(err) = details.zip_error.as_deref() {
                println!("postcode rejected: {}", err);
            }
            println!("Found {} events:", details.events.len());
            for (idx, ev) in details.events.iter().enumerate() {
                println!("{:>2}. {} | {} | {}", idx + 1, ev.timestamp, ev.location, ev.description);
            }
        }
        Err(err) => {
            eprintln!("Error: {}", err);
            process::exit(2);
        }
    }
}
