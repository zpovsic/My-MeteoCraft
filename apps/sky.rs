//! The current weather (sky and precipitation) as observed by ARSO, the Slovenian
//! Environment Agency, at Ljubljana Bežigrad, about 3 km east of Koseze. The Koseze
//! station only measures, it doesn't observe the sky.

pub mod icon;

use std::time::Duration;

pub const SOURCE: &str = "ARSO · Ljubljana Bežigrad";
/// ARSO publishes a new observation every 10 minutes.
pub const PICKUP_PERIOD: Duration = Duration::from_secs(600);
const OBSERVATION_URL: &str = "https://meteo.arso.gov.si/uploads/probase/www/observ/surface/text/sl/observationAms_LJUBL-ANA_BEZIGRAD_latest.xml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    Clear,
    FewClouds,
    PartlyCloudy,
    MostlyCloudy,
    Overcast,
    Fog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precipitation {
    Drizzle,
    Rain,
    Sleet,
    Snow,
    Hail,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sky {
    pub cover: Cover,
    pub precipitation: Option<Precipitation>,
    pub thunder: bool,
    /// Showers fall from broken cloud, so the icon lets the sun or moon show.
    pub showers: bool,
    pub night: bool,
    /// Plain English, e.g. "Partly cloudy" or "Light rain showers".
    pub description: String,
    /// Local time of the observation, e.g. "7:10".
    pub observed: String,
}

pub fn fetch() -> Result<Sky, String> {
    let mut response = ureq::get(OBSERVATION_URL)
        .header(
            "User-Agent",
            concat!("meteocraft/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| e.to_string())?;
    let xml = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse(&xml)
}

/// `nn_icon-wwsyn_icon` joins the cloud cover and the weather phenomenon with `_`,
/// e.g. `clear`, `FG`, `overcast_modRA` or `prevCloudy_lightSHSN`.
pub fn parse(xml: &str) -> Result<Sky, String> {
    let code = element(xml, "nn_icon-wwsyn_icon").unwrap_or_default();
    let mut cover = None;
    let mut phenomenon = None;
    for part in code.split('_').filter(|part| !part.is_empty()) {
        match cover_of(part) {
            Some(found) => cover = Some(found),
            None => phenomenon = Some(part),
        }
    }
    if cover.is_none() && phenomenon.is_none() {
        return Err("ARSO reported no sky observation".into());
    }

    let (intensity, base) = phenomenon.map_or((None, ""), split_intensity);
    if base == "FG" {
        cover = Some((Cover::Fog, "Fog"));
    }
    let (cover, cover_text) = cover.unwrap_or((Cover::Overcast, "Overcast"));
    let precipitation = if base.contains("RASN") {
        Some(Precipitation::Sleet)
    } else if base.contains("SN") {
        Some(Precipitation::Snow)
    } else if base.contains("GR") {
        Some(Precipitation::Hail)
    } else if base.contains("RA") {
        Some(Precipitation::Rain)
    } else if base.contains("DZ") {
        Some(Precipitation::Drizzle)
    } else {
        None
    };
    let thunder = base.starts_with("TS");
    let showers = base.starts_with("SH");

    let description = if thunder {
        "Thunderstorm".to_owned()
    } else if let Some(precipitation) = precipitation {
        let freezing = base.starts_with("FZ");
        let noun = match precipitation {
            Precipitation::Drizzle if freezing => "freezing drizzle",
            Precipitation::Drizzle => "drizzle",
            Precipitation::Rain if freezing => "freezing rain",
            Precipitation::Rain if showers => "rain showers",
            Precipitation::Rain => "rain",
            Precipitation::Sleet => "rain and snow",
            Precipitation::Snow if showers => "snow showers",
            Precipitation::Snow => "snow",
            Precipitation::Hail => "hail",
        };
        match intensity {
            Some("light") => format!("Light {noun}"),
            Some("heavy") => format!("Heavy {noun}"),
            _ => capitalize(noun),
        }
    } else {
        cover_text.to_owned()
    };

    let valid = element(xml, "valid").and_then(time_of_day);
    let night = match (
        valid,
        element(xml, "sunrise").and_then(time_of_day),
        element(xml, "sunset").and_then(time_of_day),
    ) {
        (Some((now, _)), Some((sunrise, _)), Some((sunset, _))) => now < sunrise || now >= sunset,
        _ => false,
    };

    Ok(Sky {
        cover,
        precipitation,
        thunder,
        showers,
        night,
        description,
        observed: valid.map(|(_, text)| text.to_owned()).unwrap_or_default(),
    })
}

fn cover_of(code: &str) -> Option<(Cover, &'static str)> {
    Some(match code {
        "clear" => (Cover::Clear, "Clear"),
        "mostClear" => (Cover::FewClouds, "Mostly clear"),
        "slightCloudy" => (Cover::FewClouds, "Slightly cloudy"),
        "partCloudy" => (Cover::PartlyCloudy, "Partly cloudy"),
        "modCloudy" => (Cover::PartlyCloudy, "Moderately cloudy"),
        "prevCloudy" => (Cover::MostlyCloudy, "Mostly cloudy"),
        "overcast" => (Cover::Overcast, "Overcast"),
        "FG" => (Cover::Fog, "Fog"),
        _ => return None,
    })
}

/// `lightRA` → (`light`, `RA`).
fn split_intensity(code: &str) -> (Option<&str>, &str) {
    ["light", "mod", "heavy"]
        .into_iter()
        .find_map(|intensity| Some((Some(intensity), code.strip_prefix(intensity)?)))
        .unwrap_or((None, code))
}

/// `26.10.2026 7:10 CET` → minutes since midnight and `7:10`.
fn time_of_day(stamp: &str) -> Option<(u32, &str)> {
    let time = stamp.split_whitespace().nth(1)?;
    let (hours, minutes) = time.split_once(':')?;
    Some((
        hours.parse::<u32>().ok()? * 60 + minutes.parse::<u32>().ok()?,
        time,
    ))
}

/// The text of the first `<name>…</name>`; empty elements are written `<name/>`.
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{name}>"))? + name.len() + 2;
    let len = xml[start..].find(&format!("</{name}>"))?;
    Some(xml[start..start + len].trim())
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/arso-bezigrad.xml");

    fn observation(code: &str, valid: &str) -> String {
        format!(
            "<data><sunrise>26.10.2026 6:34 CET</sunrise><sunset>26.10.2026 16:57 CET</sunset>\
             <valid>26.10.2026 {valid} CET</valid><valid_day>Ponedeljek CET</valid_day>\
             <nn_icon-wwsyn_icon>{code}</nn_icon-wwsyn_icon><wwsyn_icon/></data>"
        )
    }

    #[test]
    fn parses_saved_observation() {
        let sky = parse(FIXTURE).unwrap();
        assert_eq!(sky.cover, Cover::Clear);
        assert_eq!(sky.precipitation, None);
        assert_eq!(sky.description, "Clear");
        assert_eq!(sky.observed, "7:20");
        assert!(!sky.night);
    }

    #[test]
    fn combines_cover_and_phenomenon() {
        let sky = parse(&observation("prevCloudy_lightSHSN", "12:00")).unwrap();
        assert_eq!(sky.cover, Cover::MostlyCloudy);
        assert_eq!(sky.precipitation, Some(Precipitation::Snow));
        assert!(sky.showers);
        assert_eq!(sky.description, "Light snow showers");

        let sky = parse(&observation("overcast_modRASN", "12:00")).unwrap();
        assert_eq!(sky.precipitation, Some(Precipitation::Sleet));
        assert_eq!(sky.description, "Rain and snow");

        let sky = parse(&observation("heavyTSRA", "12:00")).unwrap();
        assert_eq!(sky.cover, Cover::Overcast);
        assert!(sky.thunder);
        assert_eq!(sky.description, "Thunderstorm");
    }

    #[test]
    fn recognizes_fog_and_night() {
        let sky = parse(&observation("FG", "5:40")).unwrap();
        assert_eq!(sky.cover, Cover::Fog);
        assert_eq!(sky.description, "Fog");
        assert!(sky.night);
        assert!(parse(&observation("clear", "16:57")).unwrap().night);
    }

    #[test]
    fn rejects_missing_observation() {
        assert!(parse(&observation("", "12:00")).is_err());
        assert!(parse("<html>hello</html>").is_err());
    }
}
