//! Downloads the frames: radar tiles from RainViewer, satellite images from EUMETSAT's
//! map service and the OpenStreetMap map under them, all on the same Web Mercator grid.

use eframe::egui::ColorImage;
use image::RgbaImage;

use crate::map::{self, BoxError};

/// The overlays are drawn on this zoom level's pixel grid, the map at twice the resolution.
const ZOOM: u32 = 8;
const MAP_ZOOM: u32 = ZOOM + 1;
/// RainViewer's finest zoom; its 512-pixel tiles have the pixels of `ZOOM`.
const RADAR_ZOOM: u32 = 7;
const RADAR_TILE: i64 = 512;
/// The overlays' size in pixels: one row of radar tiles, so a frame takes three downloads.
pub const SIZE: [u32; 2] = [1024, RADAR_TILE as u32];
/// RainViewer's free API offers only its "Universal Blue" colors, with smoothing and snow.
const RADAR_STYLE: &str = "2/1_1";
const RAINVIEWER_MAPS: &str = "https://api.rainviewer.com/public/weather-maps.json";
const CLOUD_LAYER: &str = "mtg_fd:rgb_geocolour";
const CLOUD_CAPABILITIES: &str = "https://view.eumetsat.int/geoserver/mtg_fd/rgb_geocolour/ows?service=WMS&version=1.3.0&request=GetCapabilities";
const CLOUD_MAP: &str = "https://view.eumetsat.int/geoserver/ows";
/// The satellite sees about 2 km per pixel here, so a finer image only shows the map
/// service's blocky resampling; smoothed when drawn larger, half the pixels look better.
const CLOUD_SIZE: [u32; 2] = [SIZE[0] / 2, SIZE[1] / 2];
/// Both sources publish an image every 10 minutes, on the clock's tens.
pub const STEP: i64 = 600;

/// The times of one frame of the loop, in seconds since 1970.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub radar: i64,
    /// The satellite lags behind the radar, so the newest frames share its latest image.
    pub clouds: i64,
}

pub struct Update {
    pub map: Option<ColorImage>,
    pub frames: Vec<Frame>,
    /// Only the images that weren't loaded before.
    pub radar: Vec<(i64, ColorImage)>,
    pub clouds: Vec<(i64, ColorImage)>,
    /// What went wrong with the parts that are missing.
    pub problem: Option<String>,
}

/// The top left corner on the world map at `ZOOM`: centered on Koseze across, and on the
/// row of radar tiles holding Koseze, which happens to be near its middle.
fn origin() -> (i64, i64) {
    let (x, y) = map::world_pixel(map::CENTER.0, map::CENTER.1, ZOOM);
    let left = (x - f64::from(SIZE[0]) / 2.0).round() as i64;
    let top = (y as i64).div_euclid(RADAR_TILE) * RADAR_TILE;
    (left, top)
}

/// Where Koseze is, as a fraction of the map's width and height.
pub fn koseze() -> [f32; 2] {
    let (x, y) = map::world_pixel(map::CENTER.0, map::CENTER.1, ZOOM);
    let (left, top) = origin();
    [
        ((x - left as f64) / f64::from(SIZE[0])) as f32,
        ((y - top as f64) / f64::from(SIZE[1])) as f32,
    ]
}

/// Downloads the frames of the last `count` radar images, skipping images already loaded.
pub fn fetch(count: usize, have_map: bool, have_radar: &[i64], have_clouds: &[i64]) -> Update {
    let mut problem = None;
    let mut note = |what: &str, e: &dyn std::fmt::Display| {
        problem.get_or_insert_with(|| format!("{what}: {e}"));
    };

    let map = if have_map {
        None
    } else {
        let (left, top) = origin();
        let size = SIZE.map(|side| side * 2);
        match map::dark_osm(MAP_ZOOM, (left * 2, top * 2), size) {
            Ok(mut map) => {
                fade(&mut map);
                Some(map::color_image(&map))
            }
            Err(e) => {
                note("map", &e);
                None
            }
        }
    };

    let (host, radar_frames) = match get_text(RAINVIEWER_MAPS).and_then(|json| {
        parse_radar_frames(&json).ok_or_else(|| "unexpected RainViewer response".into())
    }) {
        Ok(found) => found,
        Err(e) => {
            note("radar", &e);
            (String::new(), Vec::new())
        }
    };
    let latest_clouds = get_text(CLOUD_CAPABILITIES).and_then(|xml| {
        parse_latest_time(&xml).ok_or_else(|| "unexpected EUMETSAT response".into())
    });
    let latest_clouds = match latest_clouds {
        Ok(time) => Some(time),
        Err(e) => {
            note("clouds", &e);
            None
        }
    };

    let recent = &radar_frames[radar_frames.len().saturating_sub(count)..];
    let times: Vec<i64> = match latest_clouds {
        Some(latest) if recent.is_empty() => (0..count as i64)
            .rev()
            .map(|back| latest - back * STEP)
            .collect(),
        _ => recent.iter().map(|(time, _)| *time).collect(),
    };
    let frames = frames(&times, latest_clouds.unwrap_or(i64::MIN));

    let mut radar = Vec::new();
    for (time, path) in recent {
        if have_radar.contains(time) {
            continue;
        }
        match download_radar(&host, path) {
            Ok(image) => radar.push((*time, image)),
            Err(e) => note("radar", &e),
        }
    }
    let mut clouds = Vec::new();
    if latest_clouds.is_some() {
        let mut wanted: Vec<i64> = frames.iter().map(|f| f.clouds).collect();
        wanted.dedup();
        for time in wanted {
            if have_clouds.contains(&time) {
                continue;
            }
            match download_clouds(time) {
                Ok(image) => clouds.push((time, image)),
                Err(e) => note("clouds", &e),
            }
        }
    }

    Update {
        map,
        frames,
        radar,
        clouds,
        problem,
    }
}

/// Pairs each radar time with the satellite image of the same time, or its latest.
fn frames(radar_times: &[i64], latest_clouds: i64) -> Vec<Frame> {
    radar_times
        .iter()
        .map(|&radar| Frame {
            radar,
            clouds: (radar - radar.rem_euclid(STEP)).min(latest_clouds),
        })
        .collect()
}

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>, BoxError> {
    Ok(ureq::get(url).header("User-Agent", map::USER_AGENT).call()?)
}

fn get_text(url: &str) -> Result<String, BoxError> {
    Ok(get(url)?.body_mut().read_to_string()?)
}

fn get_image(url: &str) -> Result<RgbaImage, BoxError> {
    let mut response = get(url)?;
    let is_xml = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("xml"));
    let bytes = response.body_mut().with_config().limit(16 << 20).read_to_vec()?;
    if is_xml {
        // A map service explains a refused request in an XML exception report.
        let text = String::from_utf8_lossy(&bytes);
        let reason = element(&text, "ServiceException").unwrap_or("refused");
        return Err(reason.trim().into());
    }
    Ok(image::load_from_memory(&bytes)?.to_rgba8())
}

/// The radar image at `path`, put together from the row of tiles under the map.
fn download_radar(host: &str, path: &str) -> Result<ColorImage, BoxError> {
    let (left, top) = origin();
    let row = top / RADAR_TILE;
    let width = i64::from(SIZE[0]);
    let mut radar = RgbaImage::new(SIZE[0], SIZE[1]);
    for column in left.div_euclid(RADAR_TILE)..=(left + width - 1).div_euclid(RADAR_TILE) {
        let url = format!("{host}{path}/{RADAR_TILE}/{RADAR_ZOOM}/{column}/{row}/{RADAR_STYLE}.png");
        let tile = get_image(&url)?;
        // RainViewer answers with a smaller picture saying so when it won't serve a tile.
        if tile.dimensions() != (RADAR_TILE as u32, RADAR_TILE as u32) {
            return Err(format!("unexpected {}×{} radar tile", tile.width(), tile.height()).into());
        }
        image::imageops::replace(&mut radar, &tile, column * RADAR_TILE - left, 0);
    }
    Ok(map::color_image(&radar))
}

fn download_clouds(time: i64) -> Result<ColorImage, BoxError> {
    let (left, top) = origin();
    let (west, north) = mercator_meters(left, top);
    let (east, south) = mercator_meters(left + i64::from(SIZE[0]), top + i64::from(SIZE[1]));
    let [width, height] = CLOUD_SIZE;
    let url = format!(
        "{CLOUD_MAP}?service=WMS&version=1.3.0&request=GetMap&layers={CLOUD_LAYER}&styles=\
         &crs=EPSG:3857&bbox={west},{south},{east},{north}&width={width}&height={height}\
         &format=image/png&time={}",
        iso_time(time)
    );
    let layer = cloud_layer(&get_image(&url)?);
    // Rounds off the stair steps of the satellite's pixels.
    Ok(map::color_image(&image::imageops::blur(&layer, 1.2)))
}

/// Web Mercator coordinates in meters of a pixel on the world map at `ZOOM`.
fn mercator_meters(x: i64, y: i64) -> (f64, f64) {
    let half = std::f64::consts::PI * 6_378_137.0;
    let world = f64::from(map::TILE) * f64::from(1_u32 << ZOOM);
    (
        x as f64 / world * 2.0 * half - half,
        half - y as f64 / world * 2.0 * half,
    )
}

/// Mutes the map's colors further, so forests and fields don't compete with the clouds
/// and the rain over them.
fn fade(map: &mut RgbaImage) {
    const KEEP: f32 = 0.35;
    for pixel in map.pixels_mut() {
        let [r, g, b, _] = pixel.0.map(f32::from);
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        for channel in &mut pixel.0[..3] {
            let value = luma + (f32::from(*channel) - luma) * KEEP;
            *channel = (value * 0.85).round() as u8;
        }
    }
}

/// Keeps the clouds of a GeoColour image, which are white or grey, as white haze of
/// matching density; the brown land, the green fields and the dark sea become clear.
fn cloud_layer(image: &RgbaImage) -> RgbaImage {
    let mut layer = image.clone();
    for pixel in layer.pixels_mut() {
        let [r, g, b, a] = pixel.0.map(|c| f32::from(c) / 255.0);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
        let density = smoothstep(0.28, 0.62, luma) * (1.0 - smoothstep(0.12, 0.32, saturation));
        let shade = (200.0 + 55.0 * luma).round() as u8;
        pixel.0 = [
            shade,
            shade,
            shade.saturating_add(8),
            (density * a * 215.0).round() as u8,
        ];
    }
    layer
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// RainViewer's tile host and its past radar frames, oldest first, as (time, path).
fn parse_radar_frames(json: &str) -> Option<(String, Vec<(i64, String)>)> {
    let host = json.split("\"host\":\"").nth(1)?.split('"').next()?.to_owned();
    let past = json.split("\"past\":[").nth(1)?.split(']').next()?;
    let frames = past
        .split('{')
        .skip(1)
        .map(|entry| {
            let time = entry.split("\"time\":").nth(1)?;
            let time = time.split([',', '}']).next()?.trim().parse().ok()?;
            let path = entry.split("\"path\":\"").nth(1)?.split('"').next()?;
            Some((time, path.to_owned()))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((host, frames))
}

/// The newest image of a map service layer: the default of its time dimension.
fn parse_latest_time(xml: &str) -> Option<i64> {
    let dimension = xml.split("<Dimension name=\"time\"").nth(1)?;
    let latest = dimension.split("default=\"").nth(1)?.split('"').next()?;
    parse_iso_time(latest)
}

/// The text of the first `<name …>…</name>`.
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let start = xml.match_indices(&open).map(|(at, _)| at).find(|&at| {
        xml[at + open.len()..].starts_with([' ', '>'])
    })?;
    let start = start + xml[start..].find('>')? + 1;
    let len = xml[start..].find(&format!("</{name}>"))?;
    Some(&xml[start..start + len])
}

/// `2026-10-27T07:50:00Z` (UTC) → seconds since 1970.
fn parse_iso_time(text: &str) -> Option<i64> {
    let (date, time) = text.trim().trim_end_matches('Z').split_once('T')?;
    let mut date = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let mut time = time.split(':').map(|part| part.split('.').next()?.parse::<i64>().ok());
    let (hours, minutes) = (time.next()??, time.next()??);
    let seconds = time.next().flatten().unwrap_or(0);
    Some(days_from_civil(year, month, day) * 86_400 + hours * 3600 + minutes * 60 + seconds)
}

fn iso_time(time: i64) -> String {
    let (year, month, day) = civil_from_days(time.div_euclid(86_400));
    let seconds = time.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// "09:10" in Ljubljana, which keeps Central European Time with the EU's summer time.
pub fn ljubljana_clock(time: i64) -> String {
    let local = time + 3600 * if is_eu_summer_time(time) { 2 } else { 1 };
    let seconds = local.rem_euclid(86_400);
    format!("{:02}:{:02}", seconds / 3600, seconds / 60 % 60)
}

/// Summer time runs from 01:00 UTC on the last Sunday of March to the same on October's.
fn is_eu_summer_time(time: i64) -> bool {
    let (year, _, _) = civil_from_days(time.div_euclid(86_400));
    let last_sunday_at_one = |month| {
        let last = days_from_civil(year, month, 31);
        // 1 January 1970 was a Thursday.
        let sunday = last - (last + 4).rem_euclid(7);
        sunday * 86_400 + 3600
    };
    (last_sunday_at_one(3)..last_sunday_at_one(10)).contains(&time)
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar (Howard Hinnant).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_holds_koseze_near_its_middle() {
        let [x, y] = koseze();
        assert!((x - 0.5).abs() < 0.01, "{x}");
        assert!((0.3..0.7).contains(&y), "{y}");
        let (left, top) = origin();
        assert_eq!(top % RADAR_TILE, 0);
        let columns = (left + i64::from(SIZE[0]) - 1).div_euclid(RADAR_TILE) - left.div_euclid(RADAR_TILE);
        assert_eq!(columns + 1, 3);
    }

    #[test]
    fn the_map_covers_slovenia() {
        let (left, top) = origin();
        let (west, north) = mercator_meters(left, top);
        let (east, south) = mercator_meters(left + i64::from(SIZE[0]), top + i64::from(SIZE[1]));
        let lon = |x: f64| x / 20_037_508.34 * 180.0;
        let lat = |y: f64| (2.0 * (y / 6_378_137.0).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
        assert!(lon(west) < 13.37 && lon(east) > 16.61, "{} {}", lon(west), lon(east));
        assert!(lat(south) < 45.42 && lat(north) > 46.88, "{} {}", lat(south), lat(north));
    }

    #[test]
    fn reads_rainviewer_frames() {
        let json = r#"{"version":"2.0","generated":1793088322,"host":"https://tilecache.rainviewer.com","radar":{"past":[{"time":1793087400,"path":"/v2/radar/0f0acefdafd5"},{"time":1793088000,"path":"/v2/radar/7b009d0ef4a5"}],"nowcast":[]},"satellite":{"infrared":[]}}"#;
        let (host, frames) = parse_radar_frames(json).unwrap();
        assert_eq!(host, "https://tilecache.rainviewer.com");
        assert_eq!(
            frames,
            [
                (1_793_087_400, "/v2/radar/0f0acefdafd5".to_owned()),
                (1_793_088_000, "/v2/radar/7b009d0ef4a5".to_owned()),
            ]
        );
        assert!(parse_radar_frames("<html>").is_none());
    }

    #[test]
    fn reads_the_latest_satellite_time() {
        let xml = r#"<Layer><Dimension name="time" default="2026-10-27T07:50:00Z" units="ISO8601" nearestValue="1">2024-09-23T00:00:00.000Z/2026-10-27T07:50:00.000Z/PT10M</Dimension></Layer>"#;
        assert_eq!(parse_latest_time(xml), Some(1_793_087_400));
        assert_eq!(iso_time(1_793_087_400), "2026-10-27T07:50:00Z");
    }

    #[test]
    fn reads_service_exceptions() {
        let xml = r#"<ServiceExceptionReport><ServiceException code="InvalidDimensionValue" locator="time">
      No nearest match found</ServiceException></ServiceExceptionReport>"#;
        assert_eq!(element(xml, "ServiceException").map(str::trim), Some("No nearest match found"));
    }

    #[test]
    fn newest_frames_share_the_latest_clouds() {
        let frames = frames(&[1_793_087_400, 1_793_088_000, 1_793_088_600], 1_793_088_000);
        let clouds: Vec<i64> = frames.iter().map(|f| f.clouds).collect();
        assert_eq!(clouds, [1_793_087_400, 1_793_088_000, 1_793_088_000]);
    }

    #[test]
    fn keeps_clouds_and_clears_the_ground() {
        let mut image = RgbaImage::new(3, 1);
        image.put_pixel(0, 0, image::Rgba([235, 238, 240, 255]));
        image.put_pixel(1, 0, image::Rgba([96, 84, 52, 255]));
        image.put_pixel(2, 0, image::Rgba([16, 52, 70, 255]));
        let layer = cloud_layer(&image);
        let alpha: Vec<u8> = layer.pixels().map(|p| p.0[3]).collect();
        assert!(alpha[0] > 200, "{alpha:?}");
        assert_eq!(alpha[1..], [0, 0]);
    }

    #[test]
    fn tells_the_time_in_ljubljana() {
        // 27 October 2026, two days after summer time ended.
        assert_eq!(ljubljana_clock(1_793_088_000), "09:00");
        // 1 July 2026 12:00 UTC.
        assert_eq!(ljubljana_clock(parse_iso_time("2026-07-01T12:00:00Z").unwrap()), "14:00");
        // Summer time starts at 01:00 UTC on 29 March 2026.
        let start = parse_iso_time("2026-03-29T01:00:00Z").unwrap();
        assert_eq!(ljubljana_clock(start - 60), "01:59");
        assert_eq!(ljubljana_clock(start), "03:00");
    }

    #[test]
    fn calendar_round_trips() {
        for days in [-719_468, -1, 0, 1, 10_957, 20_753, 60_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }
}
