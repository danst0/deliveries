#[path = "../dhl.rs"]
mod dhl;
#[path = "../hermes.rs"]
mod hermes;

use hermes::fetch_tracking_events;
use std::env;
use std::fs;
use std::process;

fn main() {
    // Try to read from command line first, then file
    let args: Vec<String> = env::args().collect();
    let code = if args.len() > 1 {
        args[1].clone()
    } else {
        // Try reading from test_hermes_number.txt
        match fs::read_to_string("test_hermes_number.txt") {
            Ok(s) => s.trim().to_string(),
            Err(_) => {
                // If not found, try test_tracking_numbers.txt but warn user it might be for DHL
                eprintln!("Usage: {} <hermes_tracking_number>", args[0]);
                eprintln!("   or create 'test_hermes_number.txt' with the number inside.");
                process::exit(1);
            }
        }
    };

    if code.is_empty() {
        eprintln!("Tracking number is empty");
        process::exit(1);
    }

    println!("Checking Hermes tracking for '{}'...", code);

    match fetch_tracking_events(&code) {
        Ok(events) => {
            println!("Found {} events:", events.len());
            for (idx, ev) in events.iter().enumerate() {
                println!("{:>2}. {} | {} | {}", idx + 1, ev.timestamp, ev.location, ev.description);
            }
        }
        Err(err) => {
            eprintln!("Error fetching Hermes events: {}", err);
            // Print the error chain for debugging
            for cause in err.chain().skip(1) {
                eprintln!("  Caused by: {}", cause);
            }
            process::exit(2);
        }
    }
}
