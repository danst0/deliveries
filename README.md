# Delivery Tracker (GTK4)

A small Rust + GTK4 desktop app to check shipment status by scraping public tracking pages.

## Prerequisites
- Rust toolchain (https://rustup.rs)
- GTK 4 development libraries (on Debian/Ubuntu: `sudo apt install libgtk-4-dev`) 
- Network access to `nolp.dhl.de`

## Run
```bash
cargo run
```
Then add DHL tracking numbers and use the refresh icon to update all of them.

## Features
- Track multiple DHL numbers in one list
- Manual refresh per item or for all numbers via the refresh icon
- Automatic refresh every hour
- Archive entries to hide them and stop updates

## Notes
- This uses HTML scraping of `https://nolp.dhl.de/nextt-online-public/report?lang=en&id=<tracking>` and may break if DHL changes markup.
- Network requests run off the UI thread; failures are shown in the status line.
- No credentials are required; if DHL introduces rate limits or bot protection, requests may fail.
