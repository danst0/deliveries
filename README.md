# DHL Delivery Tracker (GTK4)

A small Rust + GTK4 desktop app to check DHL shipment status in Germany by scraping the public tracking page.

## Prerequisites
- Rust toolchain (https://rustup.rs)
- GTK 4 development libraries (on Debian/Ubuntu: `sudo apt install libgtk-4-dev`) 
- Network access to `nolp.dhl.de`

## Run
```bash
cargo run
```
Then enter a DHL tracking number and press "Check".

## Notes
- This uses HTML scraping of `https://nolp.dhl.de/nextt-online-public/report?lang=en&id=<tracking>` and may break if DHL changes markup.
- Network requests run off the UI thread; failures are shown in the status line.
- No credentials are required; if DHL introduces rate limits or bot protection, requests may fail.
