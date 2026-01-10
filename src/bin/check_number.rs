#[path = "../dhl.rs"]
mod dhl;

use dhl::fetch_tracking_events;
use std::fs;
use std::process;

fn main() {
    let raw = fs::read_to_string("test_tracking_numbers.txt")
        .expect("read test_tracking_numbers.txt next to project root");
    let code = raw.trim();
    if code.is_empty() {
        eprintln!("Tracking number file is empty");
        process::exit(1);
    }

    println!("Checking {}...", code);

    match fetch_tracking_events(code) {
        Ok(events) => {
            println!("Found {} events:", events.len());
            for (idx, ev) in events.iter().enumerate() {
                println!("{:>2}. {} | {} | {}", idx + 1, ev.timestamp, ev.location, ev.description);
            }
        }
        Err(err) => {
            eprintln!("Error: {}", err);
            process::exit(2);
        }
    }
}
