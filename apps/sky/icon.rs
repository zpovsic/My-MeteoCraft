//! Weather symbols built from a few signed-distance shapes, so the header and the
//! tray icon draw the same artwork at any pixel size.

use super::{Cover, Precipitation, Sky};

type Point = (f32, f32);
type Rgb = [u8; 3];

const SUN: Rgb = [255, 196, 64];
const MOON: Rgb = [238, 228, 196];
const CLOUD: Rgb = [226, 232, 240];
const CLOUD_BACK: Rgb = [146, 158, 178];
const STORM_CLOUD: Rgb = [128, 138, 158];
const RAIN: Rgb = [86, 170, 255];
const SNOW: Rgb = [246, 250, 255];
const BOLT: Rgb = [255, 210, 64];
const FOG: Rgb = [186, 196, 210];
/// Drawn around every shape so the symbol reads on light and dark backgrounds,
/// and so a cloud stands out from the sun behind it.
const OUTLINE: Rgb = [10, 14, 22];
const OUTLINE_ALPHA: f32 = 0.85;

/// A `size`×`size` image as unpremultiplied RGBA bytes. `outline` is in pixels.
pub fn render(sky: &Sky, size: usize, outline: f32) -> Vec<u8> {
    let layers = layers(sky);
    let pixel = 1.0 / size as f32;
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let p = ((x as f32 + 0.5) * pixel, (y as f32 + 0.5) * pixel);
            let mut color = [0.0_f32; 4];
            for (shape, rgb) in &layers {
                let distance = shape.distance(p) / pixel;
                if outline > 0.0 {
                    over(
                        &mut color,
                        OUTLINE,
                        coverage(distance - outline) * OUTLINE_ALPHA,
                    );
                }
                over(&mut color, *rgb, coverage(distance));
            }
            let [r, g, b, a] = color;
            rgba.extend([r, g, b, a].map(|c| (c * 255.0).round() as u8));
        }
    }
    rgba
}

/// The shapes to paint, back to front, in a unit square.
fn layers(sky: &Sky) -> Vec<(Shape, Rgb)> {
    let celestial = |at: Point, scale: f32| {
        if sky.night {
            (moon(at, scale), MOON)
        } else {
            (sun(at, scale), SUN)
        }
    };

    if sky.precipitation.is_some() || sky.thunder {
        let mut layers = Vec::new();
        let broken = matches!(
            sky.cover,
            Cover::Clear | Cover::FewClouds | Cover::PartlyCloudy
        );
        if sky.showers || broken {
            layers.push(celestial((0.34, 0.3), 0.7));
        }
        let cloud_color = if sky.thunder { STORM_CLOUD } else { CLOUD };
        layers.push((cloud((0.52, 0.4), 1.0), cloud_color));
        if sky.thunder {
            layers.push((
                Shape::Polygon(vec![
                    (0.56, 0.56),
                    (0.4, 0.8),
                    (0.5, 0.8),
                    (0.44, 0.97),
                    (0.66, 0.7),
                    (0.55, 0.7),
                    (0.63, 0.56),
                ]),
                BOLT,
            ));
        }
        layers.extend(falling(sky.precipitation, sky.thunder));
        return layers;
    }

    match sky.cover {
        Cover::Clear => vec![celestial((0.5, 0.5), 1.25)],
        Cover::FewClouds => vec![
            celestial((0.44, 0.42), 1.05),
            (cloud((0.66, 0.7), 0.6), CLOUD),
        ],
        Cover::PartlyCloudy => vec![
            celestial((0.38, 0.38), 0.9),
            (cloud((0.6, 0.64), 0.85), CLOUD),
        ],
        Cover::MostlyCloudy => vec![
            celestial((0.34, 0.36), 0.75),
            (cloud((0.55, 0.6), 1.1), CLOUD),
        ],
        Cover::Overcast => vec![
            (cloud((0.42, 0.42), 0.85), CLOUD_BACK),
            (cloud((0.55, 0.6), 1.05), CLOUD),
        ],
        Cover::Fog => [(0.3, 0.18, 0.7), (0.5, 0.28, 0.84), (0.7, 0.16, 0.76)]
            .into_iter()
            .map(|(y, left, right)| (Shape::Capsule((left, y), (right, y), 0.05), FOG))
            .collect(),
    }
}

/// Drops and flakes under a cloud whose bottom edge is at y ≈ 0.62.
fn falling(precipitation: Option<Precipitation>, thunder: bool) -> Vec<(Shape, Rgb)> {
    let drop = |x: f32| (Shape::Capsule((x, 0.7), (x - 0.05, 0.84), 0.035), RAIN);
    let flake = |x: f32, y: f32| (Shape::Circle((x, y), 0.05), SNOW);
    // The bolt takes the middle column.
    let columns: &[f32] = if thunder {
        &[0.3, 0.78]
    } else {
        &[0.38, 0.54, 0.7]
    };
    columns
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let low = if i % 2 == 0 { 0.76 } else { 0.86 };
            match precipitation {
                Some(Precipitation::Drizzle) => (Shape::Circle((x - 0.02, low), 0.045), RAIN),
                Some(Precipitation::Snow) => flake(x, low),
                Some(Precipitation::Hail) => (Shape::Circle((x, low), 0.055), SNOW),
                Some(Precipitation::Sleet) if i % 2 == 1 => flake(x, low),
                _ => drop(x),
            }
        })
        .collect()
}

fn sun(at: Point, scale: f32) -> Shape {
    let mut parts = vec![Shape::Circle(at, 0.17 * scale)];
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::FRAC_PI_4;
        let (sin, cos) = angle.sin_cos();
        let ray = |r: f32| (at.0 + cos * r * scale, at.1 + sin * r * scale);
        parts.push(Shape::Capsule(ray(0.26), ray(0.36), 0.032 * scale));
    }
    Shape::Union(parts)
}

fn moon(at: Point, scale: f32) -> Shape {
    Shape::Cut(
        Box::new(Shape::Circle(at, 0.27 * scale)),
        Box::new(Shape::Circle(
            (at.0 + 0.14 * scale, at.1 - 0.1 * scale),
            0.22 * scale,
        )),
    )
}

/// About 0.58 × 0.43 at scale 1, centered on `at`.
fn cloud(at: Point, scale: f32) -> Shape {
    let p = |x: f32, y: f32| (at.0 + x * scale, at.1 + y * scale);
    Shape::Union(vec![
        Shape::Circle(p(-0.15, 0.065), 0.14 * scale),
        Shape::Circle(p(0.02, -0.025), 0.19 * scale),
        Shape::Circle(p(0.17, 0.085), 0.12 * scale),
        Shape::Capsule(p(-0.17, 0.115), p(0.18, 0.115), 0.1 * scale),
    ])
}

enum Shape {
    Circle(Point, f32),
    Capsule(Point, Point, f32),
    Polygon(Vec<Point>),
    Union(Vec<Shape>),
    /// The first shape with the second cut out of it.
    Cut(Box<Shape>, Box<Shape>),
}

impl Shape {
    /// Signed distance from `p` to the edge; negative inside.
    fn distance(&self, p: Point) -> f32 {
        match self {
            Shape::Circle(center, radius) => length(sub(p, *center)) - radius,
            Shape::Capsule(a, b, radius) => {
                let (pa, ba) = (sub(p, *a), sub(*b, *a));
                let t = (dot(pa, ba) / dot(ba, ba)).clamp(0.0, 1.0);
                length(sub(pa, (ba.0 * t, ba.1 * t))) - radius
            }
            Shape::Polygon(points) => polygon_distance(points, p),
            Shape::Union(shapes) => shapes
                .iter()
                .map(|shape| shape.distance(p))
                .fold(f32::INFINITY, f32::min),
            Shape::Cut(shape, hole) => shape.distance(p).max(-hole.distance(p)),
        }
    }
}

fn polygon_distance(points: &[Point], p: Point) -> f32 {
    let mut nearest = f32::INFINITY;
    let mut inside = false;
    for (i, &a) in points.iter().enumerate() {
        let b = points[(i + points.len() - 1) % points.len()];
        let (edge, to_p) = (sub(b, a), sub(p, a));
        let t = (dot(to_p, edge) / dot(edge, edge)).clamp(0.0, 1.0);
        nearest = nearest.min(length(sub(to_p, (edge.0 * t, edge.1 * t))));
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) {
            inside = !inside;
        }
    }
    if inside { -nearest } else { nearest }
}

fn sub(a: Point, b: Point) -> Point {
    (a.0 - b.0, a.1 - b.1)
}

fn dot(a: Point, b: Point) -> f32 {
    a.0 * b.0 + a.1 * b.1
}

fn length(a: Point) -> f32 {
    dot(a, a).sqrt()
}

/// Anti-aliased coverage of a pixel whose center is `distance` pixels from the edge.
fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

/// Paints `rgb` at `alpha` over a straight-alpha pixel.
fn over(pixel: &mut [f32; 4], rgb: Rgb, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let below = pixel[3] * (1.0 - alpha);
    let total = alpha + below;
    for (channel, value) in pixel.iter_mut().zip(rgb) {
        *channel = (f32::from(value) / 255.0 * alpha + *channel * below) / total;
    }
    pixel[3] = total;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sky(cover: Cover, precipitation: Option<Precipitation>, thunder: bool) -> Sky {
        Sky {
            cover,
            precipitation,
            thunder,
            showers: false,
            night: false,
            description: String::new(),
            observed: String::new(),
        }
    }

    fn pixel(rgba: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * size + x) * 4;
        rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn clear_sky_is_a_sun_on_transparency() {
        let rgba = render(&sky(Cover::Clear, None, false), 32, 1.5);
        assert_eq!(rgba.len(), 32 * 32 * 4);
        assert_eq!(pixel(&rgba, 32, 16, 16), [SUN[0], SUN[1], SUN[2], 255]);
        assert_eq!(pixel(&rgba, 32, 0, 0)[3], 0);
    }

    #[test]
    fn every_symbol_draws_something() {
        use Cover::*;
        use Precipitation::*;
        let cases = [
            sky(FewClouds, None, false),
            sky(PartlyCloudy, None, false),
            sky(MostlyCloudy, None, false),
            sky(Overcast, None, false),
            sky(Fog, None, false),
            sky(Overcast, Some(Drizzle), false),
            sky(Overcast, Some(Rain), false),
            sky(Overcast, Some(Sleet), false),
            sky(Overcast, Some(Snow), false),
            sky(Overcast, Some(Hail), false),
            sky(Overcast, Some(Rain), true),
        ];
        for case in &cases {
            let rgba = render(case, 32, 1.5);
            let opaque = rgba.chunks(4).filter(|p| p[3] > 200).count();
            assert!(opaque > 150, "{case:?} drew only {opaque} pixels");
        }
    }
}
