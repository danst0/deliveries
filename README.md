# Deliveries

A small Rust + GTK4 desktop app that tracks DHL parcels and keeps their status in one list.

Available on Flathub:

```bash
flatpak install flathub me.dumke.deliveries
```

## Features

- Track multiple DHL numbers in one list, each with an optional custom name
- Shows the short status, progress step and the expected delivery window
- Manual refresh per item or for the whole list, plus an automatic refresh every hour
- Optional recipient postcode per shipment, for the cases where DHL withholds details
- Archive entries to hide them and stop updating them
- <kbd>Ctrl</kbd>+<kbd>Q</kbd> quits, <kbd>Ctrl</kbd>+<kbd>W</kbd> closes the current window

## Build and run

Prerequisites: a Rust toolchain (https://rustup.rs) and the GTK 4 development
libraries (Debian/Ubuntu: `sudo apt install libgtk-4-dev`, Fedora:
`sudo dnf install gtk4-devel`).

```bash
cargo run
```

Tracking numbers are stored in `deliveries.json` under the user data directory —
`~/.local/share/deliveries-tracker/` for a normal build, and
`~/.var/app/me.dumke.deliveries/data/deliveries-tracker/` under Flatpak.

## The recipient postcode

DHL serves a reduced view of a shipment when it considers part of the detail
censored, and releases the rest in exchange for the recipient postcode. Open a
shipment, enter the postcode and press <kbd>Enter</kbd>.

This only changes anything when DHL actually withholds something; for most
parcels the anonymous response is already complete. The app therefore checks the
`hasCensoredInformation` and `plzBenoetigt` flags first and skips the request
otherwise — repeated wrong postcodes make DHL block further lookups for hours.

## How the data is fetched

There is no public, documented API behind this. The app talks to the same
undocumented JSON endpoints that the dhl.de tracking page uses:

- `GET /int-verfolgen/data/search` for the anonymous view
- `POST /int-verfolgen/data/shipment` for the postcode-verified view, which needs
  a CSRF token and session cookies from `/int-verfolgen/data/config`

Expect this to break whenever DHL changes those endpoints. A headless-Chrome
fallback exists for the case where the JSON endpoint fails, but it needs a
Chrome or Chromium binary and cannot work inside the Flatpak sandbox.

Requests run off the UI thread; failures show up in the status line.

## Hermes

`src/hermes.rs` implements Hermes tracking against
`https://api.my-deliveries.de/tnt/v2/shipments/search/{id}`, but it is not wired
into the app: its success path has not yet been verified against a live
shipment. It previously relied on headless Chrome, which never worked in the
Flatpak build. Try it with the helper binary below and open an issue if it works
for you.

## Helper binaries

```bash
cargo run --bin check_number -- <tracking-number> [postcode]
cargo run --bin check_hermes -- <hermes-tracking-number>
```

`check_number` prints the parsed flags, status, delivery window and the full
event list, which is the quickest way to see what DHL returns for a number.

## License

GNU Affero General Public License v3.0 or later (AGPL-3.0-or-later).
See [LICENSE](LICENSE).
