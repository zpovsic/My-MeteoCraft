//! A map of Koseze in a dark style to suit the theme: from the Google Maps Static API
//! when an API key is set, otherwise stitched from OpenStreetMap tiles and darkened.

use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::ColorImage;
use image::RgbaImage;

/// The environment variable holding a Google Maps Platform API key.
pub const GOOGLE_KEY_VARIABLE: &str = "METEOCRAFT_GOOGLE_MAPS_KEY";
const OSM_ATTRIBUTION: &str = "© OpenStreetMap contributors";
/// The Koseze quarter as placed by OpenStreetMap; the station's exact position isn't published.
const CENTER: (f64, f64) = (46.07134, 14.46956);
const ZOOM: u32 = 15;
const TILE: u32 = 256;
/// Map size in pixels; wide like the station's graphs next to it.
const SIZE: [u32; 2] = [768, 384];
/// The largest size the Static API serves, drawn at twice the resolution.
const GOOGLE_SIZE: [u32; 2] = [640, 320];
/// Google's dark "night" colors, close to the app's theme.
const GOOGLE_STYLE: &[&str] = &[
    "element:geometry|color:0x1d2c3e",
    "element:labels.text.stroke|color:0x1d2c3e",
    "element:labels.text.fill|color:0x8a96a8",
    "feature:poi|element:labels|visibility:off",
    "feature:transit|visibility:off",
    "feature:road|element:geometry|color:0x34445a",
    "feature:road|element:geometry.stroke|color:0x1a2433",
    "feature:road.highway|element:geometry|color:0x4b5c75",
    "feature:water|element:geometry|color:0x0f2238",
    "feature:poi.park|element:geometry|color:0x1f3a35",
];
/// OpenStreetMap's tile policy asks clients to cache, and the streets rarely change.
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const USER_AGENT: &str = concat!(
    "meteocraft/",
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("CARGO_PKG_REPOSITORY"),
    ")"
);

type BoxError = Box<dyn Error + Send + Sync>;

pub struct Map {
    pub image: ColorImage,
    /// Credit for the map data, plus why Google's map isn't shown if a key was set.
    pub attribution: String,
}

/// Koseze in Google Maps.
pub fn browser_url() -> String {
    let (lat, lon) = CENTER;
    format!("https://www.google.com/maps/search/?api=1&query={lat},{lon}")
}

/// Loads the map, centered on Koseze: Google's when a key is set and it downloads,
/// otherwise OpenStreetMap's.
pub fn load() -> Result<Map, String> {
    let key = std::env::var(GOOGLE_KEY_VARIABLE).unwrap_or_default();
    let mut attribution = OSM_ATTRIBUTION.to_owned();
    if !key.trim().is_empty() {
        match download_google(key.trim()) {
            Ok(map) => {
                return Ok(Map {
                    image: color_image(&map),
                    attribution: "Map data © Google".into(),
                });
            }
            Err(e) => attribution.push_str(&format!(" · Google Maps unavailable: {e}")),
        }
    }
    Ok(Map {
        image: color_image(&load_osm()?),
        attribution,
    })
}

fn color_image(map: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([map.width() as usize, map.height() as usize], map.as_raw())
}

/// Google's terms don't allow storing its map images, so this downloads on every start.
fn download_google(key: &str) -> Result<RgbaImage, BoxError> {
    let (lat, lon) = CENTER;
    let [width, height] = GOOGLE_SIZE;
    let mut url = format!(
        "https://maps.googleapis.com/maps/api/staticmap?center={lat},{lon}&zoom={ZOOM}\
         &size={width}x{height}&scale=2&maptype=roadmap&key={key}"
    );
    for style in GOOGLE_STYLE {
        url.push_str("&style=");
        url.push_str(&style.replace('|', "%7C"));
    }
    let mut response = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .config()
        .http_status_as_error(false)
        .build()
        .call()?;
    let status = response.status();
    let bytes = response.body_mut().read_to_vec()?;
    if !status.is_success() {
        // The Static API explains a rejected key or request in a short text body.
        let reason = String::from_utf8_lossy(&bytes);
        let reason = reason.lines().next().unwrap_or("").trim();
        return Err(format!("HTTP {} {reason}", status.as_u16()).into());
    }
    Ok(image::load_from_memory(&bytes)?.to_rgba8())
}

/// Loads the OpenStreetMap map from the cache, downloading it when missing or stale.
fn load_osm() -> Result<RgbaImage, String> {
    let path = cache_path();
    let fresh = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < MAX_AGE);
    let cached = || image::open(&path).map(|map| map.to_rgba8());

    let mut map = match fresh.then(cached).and_then(Result::ok) {
        Some(map) => map,
        None => match download() {
            Ok(map) => {
                let _ = map.save(&path);
                map
            }
            // An outdated map beats no map.
            Err(e) => cached().map_err(|_| format!("could not download the map: {e}"))?,
        },
    };
    darken(&mut map);
    Ok(map)
}

fn cache_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "meteocraft-map-osm-{ZOOM}-{}x{}.png",
        SIZE[0], SIZE[1]
    ))
}

/// Position in pixels on the Web Mercator world map made of `TILE`-sized tiles.
fn world_pixel(lat: f64, lon: f64, zoom: u32) -> (f64, f64) {
    let scale = f64::from(TILE) * f64::from(1_u32 << zoom);
    let x = (lon + 180.0) / 360.0 * scale;
    let y = (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * scale;
    (x, y)
}

fn download() -> Result<RgbaImage, BoxError> {
    let (cx, cy) = world_pixel(CENTER.0, CENTER.1, ZOOM);
    let left = (cx - f64::from(SIZE[0]) / 2.0).round() as i64;
    let top = (cy - f64::from(SIZE[1]) / 2.0).round() as i64;
    let tile = i64::from(TILE);

    let mut map = RgbaImage::new(SIZE[0], SIZE[1]);
    for y in top.div_euclid(tile)..=(top + i64::from(SIZE[1]) - 1).div_euclid(tile) {
        for x in left.div_euclid(tile)..=(left + i64::from(SIZE[0]) - 1).div_euclid(tile) {
            let piece = fetch_tile(x, y)?;
            image::imageops::replace(&mut map, &piece, x * tile - left, y * tile - top);
        }
    }
    Ok(map)
}

fn fetch_tile(x: i64, y: i64) -> Result<RgbaImage, BoxError> {
    let url = format!("https://tile.openstreetmap.org/{ZOOM}/{x}/{y}.png");
    let mut response = ureq::get(&url).header("User-Agent", USER_AGENT).call()?;
    let bytes = response.body_mut().read_to_vec()?;
    let tile = image::load_from_memory(&bytes)?.to_rgba8();
    // Anything else is an error placeholder, which mustn't end up in the cache.
    if tile.dimensions() != (TILE, TILE) {
        return Err(format!("unexpected {}×{} tile", tile.width(), tile.height()).into());
    }
    Ok(tile)
}

/// Inverts brightness but keeps each color's hue, muted, so the light map turns
/// dark while water stays blue and parks green.
fn darken(map: &mut RgbaImage) {
    const SATURATION: f32 = 0.5;
    for pixel in map.pixels_mut() {
        let [r, g, b, _] = pixel.0.map(|c| f32::from(c) / 255.0);
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        let lightness = 0.1 + 0.8 * (1.0 - luma);
        for (channel, tint) in pixel.0[..3].iter_mut().zip([0.0, 0.015, 0.04]) {
            let chroma = f32::from(*channel) / 255.0 - luma;
            let value = lightness + chroma * SATURATION + tint;
            *channel = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_pixel_matches_web_mercator() {
        let half = f64::from(TILE) / 2.0;
        assert_eq!(world_pixel(0.0, 0.0, 0), (half, half));
        let (x, y) = world_pixel(85.051_128_78, -180.0, 1);
        assert!(x.abs() < 1e-6 && y.abs() < 1e-3, "({x}, {y})");
    }
}
