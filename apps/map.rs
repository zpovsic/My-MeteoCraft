//! A map of Koseze, stitched from OpenStreetMap tiles and darkened to suit the theme.

use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::ColorImage;
use image::RgbaImage;

pub const ATTRIBUTION: &str = "© OpenStreetMap contributors";
/// The Koseze quarter as placed by OpenStreetMap; the station's exact position isn't published.
const CENTER: (f64, f64) = (46.07134, 14.46956);
const ZOOM: u32 = 15;
const TILE: u32 = 256;
/// Map size in pixels; wide like the station's graphs next to it.
const SIZE: [u32; 2] = [768, 384];
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

pub fn browser_url() -> String {
    let (lat, lon) = CENTER;
    format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}")
}

/// Loads the map from the cache, downloading it when missing or stale. The map's
/// center is the location of Koseze.
pub fn load() -> Result<ColorImage, String> {
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
    Ok(ColorImage::from_rgba_unmultiplied(
        [map.width() as usize, map.height() as usize],
        map.as_raw(),
    ))
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
