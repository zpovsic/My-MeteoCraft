//! Weather alerts: frost, wind, rain and fast-falling pressure, checked against every
//! new measurement. Each alert fires when its condition begins and not again until the
//! condition has clearly ended, so a reading hovering around a threshold stays quiet.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::station::StationData;
use crate::{beaufort, direction_name, number, value_of};

const FROST: f32 = 0.0;
const FROST_CLEARED: f32 = 1.0;
/// The start of a fresh breeze (Beaufort 5).
const WIND: f32 = 29.0;
const WIND_CLEARED: f32 = 20.0;
/// A fall of 3 hPa in 3 hours is what forecasters call falling quickly.
const PRESSURE_FALL: f32 = 3.0;
const PRESSURE_FALL_CLEARED: f32 = 1.5;
const PRESSURE_PERIOD: Duration = Duration::from_secs(3 * 3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Frost,
    Wind,
    Rain,
    Pressure,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Frost, Kind::Wind, Kind::Rain, Kind::Pressure];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Frost => "Frost",
            Kind::Wind => "Wind",
            Kind::Rain => "Rain",
            Kind::Pressure => "Falling pressure",
        }
    }

    pub fn rule(self) -> String {
        match self {
            Kind::Frost => format!("The temperature drops to {FROST:.0} °C or below"),
            Kind::Wind => format!("The wind reaches {WIND:.0} km/h, a fresh breeze"),
            Kind::Rain => "Rain starts to fall".to_owned(),
            Kind::Pressure => format!(
                "The pressure falls {PRESSURE_FALL:.0} hPa or more within {} hours",
                PRESSURE_PERIOD.as_secs() / 3600
            ),
        }
    }

    /// The name in the saved settings.
    fn key(self) -> &'static str {
        match self {
            Kind::Frost => "frost",
            Kind::Wind => "wind",
            Kind::Rain => "rain",
            Kind::Pressure => "pressure",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub kind: Kind,
    pub title: String,
    pub body: String,
}

/// The measurements the alerts look at.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Conditions {
    pub temperature: Option<f32>,
    pub wind_kmh: Option<f32>,
    /// Where the wind blows from, e.g. "northwest".
    pub wind_from: String,
    pub rain_last_hour: Option<f32>,
    pub pressure: Option<f32>,
}

impl Conditions {
    pub fn of(data: &StationData) -> Self {
        let point = data.wind.direction.split_whitespace().next().unwrap_or("");
        Self {
            temperature: number(value_of(data.reading("temperatura"))),
            wind_kmh: data.wind.speed_kmh,
            wind_from: direction_name(point).to_owned(),
            rain_last_hour: number(value_of(data.precipitation("padavine v zadnji uri"))),
            pressure: number(value_of(data.reading("zračni tlak"))),
        }
    }
}

pub struct Watch {
    enabled: [bool; Kind::ALL.len()],
    /// Whether each condition held at the last measurement; `None` before the first,
    /// which only sets the baseline: conditions that already hold at startup are not news.
    active: Option<[bool; Kind::ALL.len()]>,
    pressure: VecDeque<(Instant, f32)>,
}

impl Watch {
    /// `settings` is what `settings()` returned last time; without it every alert is on.
    pub fn new(settings: Option<&str>) -> Self {
        let enabled = match settings {
            Some(settings) => {
                Kind::ALL.map(|kind| settings.split(',').any(|key| key.trim() == kind.key()))
            }
            None => [true; Kind::ALL.len()],
        };
        Self {
            enabled,
            active: None,
            pressure: VecDeque::new(),
        }
    }

    pub fn settings(&self) -> String {
        Kind::ALL
            .into_iter()
            .filter(|&kind| self.is_enabled(kind))
            .map(Kind::key)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn is_enabled(&self, kind: Kind) -> bool {
        self.enabled[kind as usize]
    }

    pub fn set_enabled(&mut self, kind: Kind, enabled: bool) {
        self.enabled[kind as usize] = enabled;
    }

    /// The alerts that the measurement `now` starts.
    pub fn check(&mut self, now: Instant, conditions: &Conditions) -> Vec<Alert> {
        if let Some(pressure) = conditions.pressure {
            self.pressure.push_back((now, pressure));
        }
        while self
            .pressure
            .front()
            .is_some_and(|&(at, _)| now.duration_since(at) > PRESSURE_PERIOD)
        {
            self.pressure.pop_front();
        }
        let fall = conditions.pressure.map(|pressure| {
            self.pressure
                .iter()
                .map(|&(_, p)| p)
                .fold(pressure, f32::max)
                - pressure
        });

        let before = self.active;
        let active = Kind::ALL.map(|kind| {
            let was = before.is_some_and(|active| active[kind as usize]);
            let rising = |value: Option<f32>, on: f32, off: f32| {
                value.map_or(was, |v| if was { v >= off } else { v >= on })
            };
            match kind {
                Kind::Frost => conditions
                    .temperature
                    .map_or(was, |t| if was { t < FROST_CLEARED } else { t <= FROST }),
                Kind::Wind => rising(conditions.wind_kmh, WIND, WIND_CLEARED),
                Kind::Rain => conditions.rain_last_hour.map_or(was, |mm| mm > 0.0),
                Kind::Pressure => rising(fall, PRESSURE_FALL, PRESSURE_FALL_CLEARED),
            }
        });
        self.active = Some(active);

        let Some(before) = before else {
            return Vec::new();
        };
        Kind::ALL
            .into_iter()
            .filter(|&kind| {
                active[kind as usize] && !before[kind as usize] && self.is_enabled(kind)
            })
            .filter_map(|kind| alert(kind, conditions, fall))
            .collect()
    }
}

fn alert(kind: Kind, c: &Conditions, fall: Option<f32>) -> Option<Alert> {
    let (title, body) = match kind {
        Kind::Frost => (
            "❄ Frost at Koseze".to_owned(),
            format!("The temperature has dropped to {:.1} °C.", c.temperature?),
        ),
        Kind::Wind => {
            let kmh = c.wind_kmh?;
            let from = if c.wind_from.is_empty() {
                String::new()
            } else {
                format!(" from the {}", c.wind_from)
            };
            (
                "🌬 Windy at Koseze".to_owned(),
                format!("{kmh:.0} km/h{from}, a {}.", beaufort(kmh)),
            )
        }
        Kind::Rain => {
            let title = if c.temperature.is_some_and(|t| t <= FROST_CLEARED) {
                "🌨 Rain or snow at Koseze"
            } else {
                "🌧 Rain at Koseze"
            };
            (
                title.to_owned(),
                format!("{:.1} mm in the last hour.", c.rain_last_hour?),
            )
        }
        Kind::Pressure => (
            "📉 Pressure falling fast".to_owned(),
            format!(
                "Down {:.1} hPa within {} hours, to {:.1} hPa. Unsettled weather may be on the way.",
                fall?,
                PRESSURE_PERIOD.as_secs() / 3600,
                c.pressure?
            ),
        ),
    };
    Some(Alert { kind, title, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn temperature(t: f32) -> Conditions {
        Conditions {
            temperature: Some(t),
            ..Conditions::default()
        }
    }

    fn kinds(alerts: &[Alert]) -> Vec<Kind> {
        alerts.iter().map(|a| a.kind).collect()
    }

    #[test]
    fn the_first_measurement_only_sets_the_baseline() {
        let mut watch = Watch::new(None);
        let now = Instant::now();
        assert!(watch.check(now, &temperature(-3.0)).is_empty());
        assert!(watch.check(now + 5 * MINUTE, &temperature(-3.5)).is_empty());
    }

    #[test]
    fn frost_fires_once_until_it_has_clearly_warmed_up() {
        let mut watch = Watch::new(None);
        let start = Instant::now();
        let temperatures = [2.0, 0.4, -0.2, 0.3, -0.1, 0.8, 1.2, -0.5];
        let fired_at: Vec<usize> = temperatures
            .iter()
            .enumerate()
            .filter(|&(i, &t)| {
                let alerts = watch.check(start + i as u32 * 5 * MINUTE, &temperature(t));
                kinds(&alerts) == [Kind::Frost]
            })
            .map(|(i, _)| i)
            .collect();
        assert_eq!(fired_at, [2, 7]);
    }

    #[test]
    fn missing_readings_keep_the_state() {
        let mut watch = Watch::new(None);
        let start = Instant::now();
        watch.check(start, &temperature(2.0));
        assert_eq!(
            kinds(&watch.check(start + MINUTE, &temperature(-1.0))),
            [Kind::Frost]
        );
        assert!(
            watch
                .check(start + 2 * MINUTE, &Conditions::default())
                .is_empty()
        );
        assert!(
            watch
                .check(start + 3 * MINUTE, &temperature(-1.0))
                .is_empty()
        );
    }

    #[test]
    fn rain_fires_when_it_starts() {
        let mut watch = Watch::new(None);
        let start = Instant::now();
        let rain = |mm| Conditions {
            rain_last_hour: Some(mm),
            temperature: Some(6.0),
            ..Conditions::default()
        };
        watch.check(start, &rain(0.0));
        let alerts = watch.check(start + 5 * MINUTE, &rain(0.3));
        assert_eq!(kinds(&alerts), [Kind::Rain]);
        assert_eq!(alerts[0].title, "🌧 Rain at Koseze");
        assert_eq!(alerts[0].body, "0.3 mm in the last hour.");
        assert!(watch.check(start + 10 * MINUTE, &rain(1.2)).is_empty());
        watch.check(start + 70 * MINUTE, &rain(0.0));
        assert_eq!(
            kinds(&watch.check(start + 75 * MINUTE, &rain(0.3))),
            [Kind::Rain]
        );
    }

    #[test]
    fn wind_names_the_direction_and_strength() {
        let mut watch = Watch::new(None);
        let start = Instant::now();
        let wind = |kmh| Conditions {
            wind_kmh: Some(kmh),
            wind_from: "northwest".to_owned(),
            ..Conditions::default()
        };
        watch.check(start, &wind(12.0));
        let alerts = watch.check(start + 5 * MINUTE, &wind(31.4));
        assert_eq!(
            alerts[0].body,
            "31 km/h from the northwest, a fresh breeze."
        );
        assert!(watch.check(start + 10 * MINUTE, &wind(24.0)).is_empty());
        assert!(watch.check(start + 15 * MINUTE, &wind(30.0)).is_empty());
    }

    #[test]
    fn pressure_falling_within_three_hours() {
        let mut watch = Watch::new(None);
        let start = Instant::now();
        let pressure = |hpa| Conditions {
            pressure: Some(hpa),
            ..Conditions::default()
        };
        // A slow fall of 0.5 hPa an hour, 1.5 hPa within 3 hours.
        for i in 0..=12 {
            let at = start + i * 30 * MINUTE;
            assert!(
                watch
                    .check(at, &pressure(1020.0 - i as f32 * 0.25))
                    .is_empty()
            );
        }
        // Then down to 3.2 hPa below the highest reading of the last 3 hours (1017.5 hPa).
        let at = start + 8 * 60 * MINUTE;
        let alerts = watch.check(at, &pressure(1014.3));
        assert_eq!(kinds(&alerts), [Kind::Pressure]);
        assert_eq!(
            alerts[0].body,
            "Down 3.2 hPa within 3 hours, to 1014.3 hPa. Unsettled weather may be on the way."
        );
    }

    #[test]
    fn disabled_alerts_stay_quiet() {
        let mut watch = Watch::new(Some("wind,rain"));
        assert!(!watch.is_enabled(Kind::Frost));
        let start = Instant::now();
        watch.check(start, &temperature(2.0));
        assert!(watch.check(start + MINUTE, &temperature(-1.0)).is_empty());
    }

    #[test]
    fn settings_round_trip() {
        let mut watch = Watch::new(None);
        assert_eq!(watch.settings(), "frost,wind,rain,pressure");
        watch.set_enabled(Kind::Wind, false);
        assert_eq!(watch.settings(), "frost,rain,pressure");
        let restored = Watch::new(Some(&watch.settings()));
        assert_eq!(restored.enabled, watch.enabled);
        assert_eq!(Watch::new(Some("")).settings(), "");
    }
}
