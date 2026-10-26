//! The last 24 hours of measurements. The station publishes them only as graph images,
//! so the values are read back out of the images.

mod digitize;

pub use digitize::Point;

use crate::station;

/// The station's graphs, and the line colors of the series drawn in each.
const TEMPERATURE_GRAPH: &str = "graf.png";
const TEMPERATURE: [u8; 3] = [0x00, 0x00, 0xa4];
const WIND_CHILL: [u8; 3] = [0xff, 0xcc, 0x00];
const HEAT_INDEX: [u8; 3] = [0xff, 0x00, 0x00];
const PRESSURE_GRAPH: &str = "graf_tlak.png";
const PRESSURE: [u8; 3] = [0x99, 0x00, 0xcc];
const HUMIDITY_GRAPH: &str = "graf_hum.png";
const HUMIDITY: [u8; 3] = [0xff, 0x33, 0x00];

#[derive(Debug, Clone)]
pub struct History {
    pub temperature: Vec<Point>,
    /// Only where it differs from the temperature; the graph draws it underneath.
    pub wind_chill: Vec<Point>,
    /// Likewise, only where it differs from the temperature.
    pub heat_index: Vec<Point>,
    pub pressure: Vec<Point>,
    pub humidity: Vec<Point>,
}

pub fn fetch() -> Result<History, String> {
    let [temperature, wind_chill, heat_index] =
        read(TEMPERATURE_GRAPH, [TEMPERATURE, WIND_CHILL, HEAT_INDEX])?;
    let [pressure] = read(PRESSURE_GRAPH, [PRESSURE])?;
    let [humidity] = read(HUMIDITY_GRAPH, [HUMIDITY])?;
    if temperature.is_empty() {
        return Err("the temperature graph shows no data".into());
    }
    Ok(History {
        temperature,
        wind_chill,
        heat_index,
        pressure,
        humidity,
    })
}

fn read<const N: usize>(graph: &str, colors: [[u8; 3]; N]) -> Result<[Vec<Point>; N], String> {
    let failed = |e: &dyn std::fmt::Display| format!("{graph}: {e}");
    let bytes = ureq::get(station::image_url(graph))
        .call()
        .and_then(|mut response| response.body_mut().read_to_vec())
        .map_err(|e| failed(&e))?;
    let image = image::load_from_memory(&bytes)
        .map_err(|e| failed(&e))?
        .to_rgb8();
    let traces = digitize::digitize(&image, &colors).map_err(|e| failed(&e))?;
    Ok(traces.try_into().expect("one trace per color"))
}
