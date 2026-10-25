# MeteoCraft

A desktop app, written in Rust, that shows live data from the
[Ljubljana Koseze weather station](https://vreme.zajcek.org/koseze/).

The station's website has no data API, so MeteoCraft downloads the page, extracts the
measurements from the HTML and shows them in a native window. It updates itself every
5 minutes, the same interval as the website.

## Features

- **Now:** the current temperature, colored from icy blue to hot red, a plain-English
  summary such as "Cool and humid", wind chill, dew point, and a bar showing where the
  current temperature sits within today's range
- **Wind:** speed in km/h and m/s with its Beaufort description, and a compass showing
  where the wind is coming from
- **Tiles:** air pressure, humidity (with a fill bar), rain and snow depth, each with
  today's range or extra detail
- **Today's statistics:** minimum, maximum and average for each measurement
- **All-time records:** with the date and time each was set
- **Images:** the live camera and the station's temperature and pressure graphs
  (click an image to open it in your browser)
- Automatic refresh every 5 minutes, with a status badge (Live, Updating or Offline),
  a countdown and a manual **Refresh** button
- A dark theme, and a layout that reflows from several columns down to one as the
  window gets narrower

The website is in Slovenian; the app shows everything in English.

## Requirements

- Rust 1.95 or newer (required by egui 0.37)
- An internet connection

## Running

```sh
cargo run --release
```

The executable is written to `target/release/meteocraft` (`meteocraft.exe` on Windows).
Release builds on Windows open without a console window.

## Testing

```sh
cargo test
```

The parser tests run against a saved copy of the station page in
`tests/fixtures/koseze.html`, so they don't need network access. If the website's layout
changes, download a fresh copy of the page into that file and update the expected values
in the tests.

## Project layout

```text
apps/
  main.rs      App state, background downloads, auto-refresh and the dashboard layout
  station.rs   Downloading and parsing the station page
  theme.rs     Color palette, egui styling and the temperature color scale
  widgets.rs   Cards, responsive card rows, status badge, compass and temperature bar
tests/
  fixtures/
    koseze.html  Saved copy of the station page used by the tests
```

## How it works

- `station::fetch()` downloads the page with [`ureq`](https://crates.io/crates/ureq), and
  `station::parse()` reads the values with [`scraper`](https://crates.io/crates/scraper).
- Downloads run on a background thread so the window stays responsive.
- The camera and graph images are loaded directly from the station's server by
  `egui_extras` and reloaded whenever new data arrives.
- Labels are parsed in Slovenian, as they appear on the website, and translated for display
  by `english()` in `apps/main.rs`. A label that has no translation is shown in Slovenian,
  and a test fails if any label on the saved page is missing a translation.
- If the page layout changes and the parser can't find a section, the app shows an
  "unexpected page layout" message instead of wrong values.

## Disclaimer

The data comes from a private LaCrosse WS2300 station run by enthusiasts. It is not part
of the official ARSO network and is for information only. All data and images belong to
the station's operators at [vreme.zajcek.org](https://vreme.zajcek.org/koseze/).
