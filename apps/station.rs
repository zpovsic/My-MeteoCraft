//! Fetching and parsing of the Ljubljana Koseze weather station page.

use std::fmt;

use scraper::{ElementRef, Html, Selector};

pub const STATION_URL: &str = "https://vreme.zajcek.org/koseze/";

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wind {
    pub speed: String,
    pub speed_kmh: Option<f32>,
    pub direction: String,
    pub direction_deg: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyStat {
    pub label: String,
    pub min: Option<String>,
    pub max: Option<String>,
    pub avg: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Extreme {
    pub value: String,
    pub when: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub label: String,
    pub min: Option<Extreme>,
    pub max: Option<Extreme>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StationData {
    pub last_measurement: String,
    pub atmosphere: Vec<Reading>,
    pub wind: Wind,
    pub precipitation: Vec<Reading>,
    pub daily: Vec<DailyStat>,
    pub records: Vec<Record>,
}

impl StationData {
    pub fn reading(&self, label_prefix: &str) -> Option<&Reading> {
        self.atmosphere
            .iter()
            .find(|r| r.label.starts_with(label_prefix))
    }
}

#[derive(Debug)]
pub enum FetchError {
    Http(String),
    Layout(&'static str),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Http(msg) => write!(f, "could not download station data: {msg}"),
            FetchError::Layout(what) => write!(f, "unexpected page layout: {what} not found"),
        }
    }
}

impl std::error::Error for FetchError {}

pub fn image_url(path: &str) -> String {
    format!("{STATION_URL}{path}")
}

pub fn fetch() -> Result<StationData, FetchError> {
    let mut response = ureq::get(STATION_URL)
        .header(
            "User-Agent",
            concat!("meteocraft/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| FetchError::Http(e.to_string()))?;
    let html = response
        .body_mut()
        .read_to_string()
        .map_err(|e| FetchError::Http(e.to_string()))?;
    parse(&html)
}

pub fn parse(html: &str) -> Result<StationData, FetchError> {
    let doc = Html::parse_document(html);

    let last_measurement = doc
        .select(&sel("#top-data"))
        .next()
        .map(|el| {
            text_of(el)
                .trim_start_matches("Zadnja meritev:")
                .trim()
                .to_owned()
        })
        .ok_or(FetchError::Layout("last measurement"))?;

    let atmosphere = label_value_rows(&doc, "#ozracje tr");
    if atmosphere.is_empty() {
        return Err(FetchError::Layout("atmosphere section"));
    }

    let speed = first_text(&doc, "#veter-spd").ok_or(FetchError::Layout("wind speed"))?;
    let direction = first_text(&doc, "#veter-smer").ok_or(FetchError::Layout("wind direction"))?;
    let wind = Wind {
        speed_kmh: speed
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok()),
        direction_deg: direction
            .split_whitespace()
            .rev()
            .find_map(|s| s.parse().ok()),
        speed,
        direction,
    };

    let precipitation = label_value_rows(&doc, "#padavine tr");

    Ok(StationData {
        last_measurement,
        atmosphere,
        wind,
        precipitation,
        daily: daily_stats(&doc),
        records: records(&doc),
    })
}

fn label_value_rows(doc: &Html, row_selector: &str) -> Vec<Reading> {
    let td = sel("td");
    doc.select(&sel(row_selector))
        .filter_map(|row| {
            let mut cells = row.select(&td).map(text_of);
            let label = cells.next()?.trim_end_matches(':').trim().to_owned();
            let value = cells.next()?;
            Some(Reading {
                label,
                value: if has_digit(&value) { value } else { "—".to_owned() },
            })
        })
        .collect()
}

/// Rows use `colspan` on the label cell to skip columns, so values are placed
/// by their column index rather than their position in the row.
fn daily_stats(doc: &Html) -> Vec<DailyStat> {
    let td = sel("td");
    doc.select(&sel("#statistika tr.datarow"))
        .map(|row| {
            let mut label = String::new();
            let mut values: [Option<String>; 3] = Default::default();
            let mut column = 0;
            for cell in row.select(&td) {
                let span: usize = cell
                    .value()
                    .attr("colspan")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                if column == 0 {
                    label = text_of(cell).trim_end_matches(':').trim().to_owned();
                } else if let Some(slot) = values.get_mut(column - 1) {
                    *slot = Some(text_of(cell)).filter(|v| !v.is_empty());
                }
                column += span;
            }
            let [min, max, avg] = values;
            DailyStat { label, min, max, avg }
        })
        .collect()
}

fn records(doc: &Html) -> Vec<Record> {
    let td = sel("td");
    let bold = sel("b");
    let font = sel("font");
    let extreme = |cell: ElementRef| {
        let value = cell
            .select(&bold)
            .map(text_of)
            .find(|v| !v.is_empty())?;
        let when = cell
            .select(&font)
            .next()
            .map(|f| text_of(f).trim_matches(|c| c == '(' || c == ')').to_owned())
            .unwrap_or_default();
        Some(Extreme { value, when })
    };

    doc.select(&sel("#ekstremi tr.datarow"))
        .filter_map(|row| {
            let mut cells = row.select(&td);
            let label = text_of(cells.next()?).trim_end_matches(':').trim().to_owned();
            let min = cells.next().and_then(extreme);
            let max = cells.next().and_then(extreme);
            Some(Record { label, min, max })
        })
        .collect()
}

fn sel(selector: &str) -> Selector {
    Selector::parse(selector).expect("static selector is valid")
}

fn first_text(doc: &Html, selector: &str) -> Option<String> {
    doc.select(&sel(selector)).next().map(text_of)
}

/// Element text with all whitespace (including `&nbsp;`) collapsed to single spaces.
fn text_of(el: ElementRef) -> String {
    el.text()
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_digit(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/koseze.html");

    fn data() -> StationData {
        parse(FIXTURE).expect("fixture parses")
    }

    #[test]
    fn parses_current_conditions() {
        let d = data();
        assert_eq!(d.last_measurement, "25.10.2026 ob 15:50:05");
        assert_eq!(d.reading("temperatura").unwrap().value, "8.6 °C");
        assert_eq!(d.reading("zračni tlak").unwrap().value, "1013.2 hPa");
        assert_eq!(d.reading("vlažnost").unwrap().value, "86%");
        assert_eq!(d.reading("občutek mraza").unwrap().value, "8.6 °C");
        assert_eq!(d.atmosphere.len(), 5);
    }

    #[test]
    fn parses_wind() {
        let w = data().wind;
        assert_eq!(w.speed, "0.0 km/h (0.00 m/s)");
        assert_eq!(w.speed_kmh, Some(0.0));
        assert_eq!(w.direction, "WNW oz. 292.5 °");
        assert_eq!(w.direction_deg, Some(292.5));
    }

    #[test]
    fn parses_precipitation_with_missing_snow() {
        let p = data().precipitation;
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].value, "0.00 mm");
        assert_eq!(p[2].label, "višina snežne odeje");
        assert_eq!(p[2].value, "—");
    }

    #[test]
    fn daily_stats_respect_colspan() {
        let daily = data().daily;
        let temp = &daily[0];
        assert_eq!(temp.label, "temperatura");
        assert_eq!(temp.min.as_deref(), Some("3.5 °C"));
        assert_eq!(temp.max.as_deref(), Some("8.8 °C"));
        assert_eq!(temp.avg.as_deref(), Some("5.4 °C"));

        let wind = daily.iter().find(|s| s.label == "hitrost vetra").unwrap();
        assert_eq!(wind.min, None);
        assert_eq!(wind.max.as_deref(), Some("4.3 km/h"));
        assert_eq!(wind.avg.as_deref(), Some("0.3 km/h"));

        let chill = daily.iter().find(|s| s.label == "občutek mraza").unwrap();
        assert_eq!(chill.max, None);
    }

    #[test]
    fn parses_records() {
        let records = data().records;
        let temp = &records[0];
        assert_eq!(temp.label, "temperatura");
        let min = temp.min.as_ref().unwrap();
        assert_eq!(min.value, "-15.90 °C");
        assert_eq!(min.when, "25.01.2006, 06:22:06");
        assert_eq!(temp.max.as_ref().unwrap().value, "38.60 °C");

        let rain = records.iter().find(|r| r.label == "padavine v 24h").unwrap();
        assert!(rain.min.is_none());
        assert_eq!(rain.max.as_ref().unwrap().value, "133.05 mm");
    }

    #[test]
    fn rejects_unrelated_page() {
        assert!(parse("<html><body>hello</body></html>").is_err());
    }
}
