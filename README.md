# MeteoCraft

A desktop app, written in Rust, that shows live data from the
[Ljubljana Koseze weather station](https://vreme.zajcek.org/koseze/).

The station's website has no data API, so MeteoCraft downloads the page, extracts the
measurements from the HTML and shows them in a native window. It updates itself every
5 minutes, the same interval as the website.

## Features

- **Current weather:** a symbol (sun or moon, clouds, rain, snow, fog, thunderstorm) and
  a description such as "Partly cloudy" or "Light rain" in the middle of the header. The
  Koseze station doesn't observe the sky, so this comes from the official ARSO
  observation at Ljubljana Bežigrad, about 3 km east. Click it to open ARSO's
  [forecast chart for Ljubljana](https://vreme.arso.gov.si/napoved/Ljubljana/graf) in a
  window of its own (Microsoft Edge in app mode, or a browser tab if Edge is missing);
  the MeteoCraft window hides to the taskbar widget
- **Now:** the current temperature, colored from icy blue to hot red, a plain-English
  summary such as "Cool and humid", wind chill, dew point, and a bar showing where the
  current temperature sits within today's range
- **Wind:** speed in km/h and m/s with its Beaufort description, and a compass showing
  where the wind is coming from
- **Tiles:** air pressure, humidity (with a fill bar), rain and snow depth, each with
  today's range or extra detail
- **Today's statistics:** minimum, maximum and average for each measurement
- **All-time records:** with the date and time each was set
- **Map and graphs:** a dark map of Koseze, from Google Maps when you
  [set an API key](#google-maps) and from OpenStreetMap otherwise, and the station's
  temperature and pressure graphs. Click the map to open Google Maps at Koseze in a window
  of its own (the MeteoCraft window hides to the taskbar widget)
- **Last 24 hours:** click a graph to open a window with the app's own charts of
  temperature (with wind chill and heat index where they differ), air pressure and
  humidity. Each shows its low and high, and hovering shows the values at that time
  on every chart. The station publishes this history only as graph images, so the app
  reads the values out of them. The window reopens where you last left it, at the same
  size
- **Alerts (Windows):** a notification when frost, wind, rain or falling pressure begins.
  Choose which ones under **Alerts** in the header, and send a test notification from the
  same menu. Click a notification to open the window. See [Alerts](#alerts) for the rules
- Automatic refresh every 5 minutes, with a status badge (Live, Updating or Offline),
  a countdown and a manual **Refresh** button
- A dark theme, and a layout that reflows from several columns down to one as the
  window gets narrower
- **Taskbar widget (Windows):** the current weather symbol and, beside it, the current
  temperature in its color, embedded in the taskbar just left of the notification area,
  with a tooltip summarizing the latest reading. The tooltip and the menu follow the
  taskbar's light or dark theme. Click the widget to open the window; right-click it for
  **Open MeteoCraft**, **Refresh now**, **Start with Windows** and **Quit**. Closing the
  window hides it to the widget and the app keeps refreshing; use **Quit** to exit.

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

The widget sits on a horizontal taskbar. On a vertical taskbar, or if the taskbar can't
host it, MeteoCraft shows a notification-area icon with the temperature instead. Windows
11 puts new notification-area icons in the hidden overflow (the **^** arrow); to keep
that icon visible, drag it out of the overflow, or turn on **MeteoCraft** under
Settings → Personalization → Taskbar → Other system tray icons.

### Google Maps

The map card shows a Google map when the `METEOCRAFT_GOOGLE_MAPS_KEY` environment
variable holds a Google Maps Platform API key with the
[Maps Static API](https://developers.google.com/maps/documentation/maps-static) enabled.
Google requires a billing account for the key, but the app makes one request each time it
starts, well within the free monthly usage. On Windows, set the variable for your account
and restart MeteoCraft:

```sh
setx METEOCRAFT_GOOGLE_MAPS_KEY "your-api-key"
```

Without a key the card shows the OpenStreetMap map. If Google rejects the key or can't be
reached, the card falls back to OpenStreetMap and says why under the map.

### Starting with Windows

Check **Start with Windows** in the icon's menu to start MeteoCraft when you sign in. It
adds a `MeteoCraft` entry under
`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` that runs the current
executable with `--tray`, so it starts with only the taskbar widget. Until the first
download succeeds (the network may still be connecting), it retries every 30 seconds.
Uncheck the item to remove the entry.

The entry points at the executable's current location, so if you move or rename the
program, check the item again from the new copy. You can run it the same way by hand:

```sh
meteocraft --tray
```

### Alerts

MeteoCraft checks each new measurement and notifies you when:

| Alert | Notifies when | Can notify again once |
|---|---|---|
| Frost | the temperature drops to 0 °C or below | it is back to 1 °C or above |
| Wind | the wind reaches 29 km/h, a fresh breeze | it is below 20 km/h |
| Rain | any rain falls in the last hour | an hour passes without rain |
| Falling pressure | the pressure falls 3 hPa or more within 3 hours | the fall is under 1.5 hPa |

The station reports only the average wind, not gusts. The first measurement after the
app starts is never an alert, so conditions that already hold don't notify you. Alerts
come only while MeteoCraft is running, including when only the taskbar widget is showing.

### Settings

Which alerts are on, the position of the windows and the charts window's size are saved
in `%APPDATA%\MeteoCraft\data\app.ron`.

## Testing

```sh
cargo test
```

The parser tests run against saved copies of the station page in
`tests/fixtures/koseze.html` and of the ARSO observation in
`tests/fixtures/arso-bezigrad.xml`, so they don't need network access. If the website's layout
changes, download a fresh copy of the page into that file and update the expected values
in the tests. Likewise, the tests that read values out of the graphs use saved copies of
the three graph images.

## Project layout

```text
apps/
  alerts.rs    Decides when the weather is worth a notification
  app_window.rs  Opens a web page in a window of its own
  charts.rs    The "Last 24 hours" charts
  history.rs   The last 24 hours of measurements, read from the station's graphs
  history/
    digitize.rs  Reads the axes and lines of an RRDtool graph image
  main.rs      App state, background downloads, auto-refresh and the dashboard layout
  map.rs       The map of Koseze, from Google Maps or OpenStreetMap, in a dark style
  notify.rs    Shows Windows notifications under MeteoCraft's name and icon
  sky.rs       The current weather from ARSO's Ljubljana Bežigrad observation
  sky/
    icon.rs    Draws the weather symbols, for both the header and the taskbar
  station.rs   Downloading and parsing the station page
  theme.rs     Color palette, egui styling and the temperature color scale
  tray.rs      The taskbar presence: widget, fallback notification-area icon and menu
  tray/
    autostart.rs  The "Start with Windows" entry in the registry's Run key
    dark_mode.rs  Reads the taskbar theme and applies it to the menus
    tooltip.rs    The hover tooltip, drawn by the app to match the theme
    widget.rs     The weather symbol and temperature embedded in the taskbar
  widgets.rs   Cards, responsive card rows, status badge, compass and temperature bar
tests/
  fixtures/
    arso-bezigrad.xml  Saved ARSO observation used by the tests
    graf*.png          Saved copies of the station's three graphs
    koseze.html        Saved copy of the station page used by the tests
```

## How it works

- `station::fetch()` downloads the page with [`ureq`](https://crates.io/crates/ureq), and
  `station::parse()` reads the values with [`scraper`](https://crates.io/crates/scraper).
- Downloads run on a background thread so the window stays responsive.
- The graph images are loaded directly from the station's server by `egui_extras` and
  reloaded whenever new data arrives.
- For the charts, `history::fetch()` downloads three graphs and reads them back into
  numbers. The graphs are drawn by RRDtool: it finds the white plot area, reads the value
  and hour labels by matching each 6×7 pixel character against templates of the graph
  font, and maps every column of each line color to a time and a value. A column where
  the line steps holds both levels, so the new level is the end away from the previous
  one. The values are as precise as the graphs' pixels: about 2 minutes across, and one
  pixel of each graph's scale up and down (about 1.3 hPa for the pressure). While the
  charts are open they are reread with every refresh.
- With a key, the map is a single Maps Static API image in a dark style set through the
  API's styling parameters. Google's terms don't allow storing it, so it is downloaded on
  every start.
- Otherwise the map is built once from [OpenStreetMap](https://www.openstreetmap.org/copyright)
  tiles and kept in the system's temp folder for 30 days, as the
  [tile usage policy](https://operations.osmfoundation.org/policies/tiles/) asks.
- `sky::fetch()` reads the cloud cover and weather phenomenon (e.g. `overcast_lightRA`)
  from ARSO's [observation feed](https://meteo.arso.gov.si/uploads/meteo/help/sl/xml_service.html),
  at most every 10 minutes as ARSO publishes it, and uses the sunrise and sunset in the
  same feed to choose a sun or a moon. The symbols are drawn by the app from simple
  shapes, so they look the same in the header and in the taskbar.
- The taskbar widget is a layered child window of the taskbar, drawn by the app on its own
  thread. It follows the notification area as it grows or shrinks, and comes back when
  Explorer restarts.
- Notifications are Windows toasts. So they show MeteoCraft's name and icon, the app
  registers itself under `HKEY_CURRENT_USER\Software\Classes\AppUserModelId\MeteoCraft`
  and saves the icon in `%LOCALAPPDATA%\MeteoCraft`.
- Labels are parsed in Slovenian, as they appear on the website, and translated for display
  by `english()` in `apps/main.rs`. A label that has no translation is shown in Slovenian,
  and a test fails if any label on the saved page is missing a translation.
- If the page layout changes and the parser can't find a section, the app shows an
  "unexpected page layout" message instead of wrong values.

## Disclaimer

The data comes from a private LaCrosse WS2300 station run by enthusiasts. It is not part
of the official ARSO network and is for information only. All data and images belong to
the station's operators at [vreme.zajcek.org](https://vreme.zajcek.org/koseze/).
The current weather comes from the Slovenian Environment Agency
([ARSO](https://meteo.arso.gov.si/)). The Google map is © Google and used under the
[Google Maps Platform terms](https://cloud.google.com/maps-platform/terms). OpenStreetMap
map data is © OpenStreetMap contributors, available under the
[Open Database License](https://www.openstreetmap.org/copyright).

## License

The MeteoCraft source code is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option. This covers the code only, not the weather data or images shown by the app.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.
