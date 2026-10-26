//! Reads measurements back out of the station's RRDtool graph images: finds the plot
//! area, reads the axis labels with templates of the graph font, and follows each
//! series' line color column by column.

use image::RgbImage;

/// A value read from a graph, at `minutes` past midnight of the day the graph starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub minutes: f64,
    pub value: f32,
}

/// The graph font is monospaced; every label character fills a 6×7 pixel cell.
const PITCH: usize = 6;
const GLYPH_HEIGHT: usize = 7;
/// The darkness of the margin color (#f3f3f3) is 0; text is darker than this.
const INK: u8 = 40;
const BACKGROUND_LUMA: i32 = 243;
/// Labels whose cells match a template worse than this on average aren't trusted.
const MAX_CELL_SCORE: u32 = 600;
/// How far line pixels blended with a grid line may stray from the series color.
const COLOR_TOLERANCE: i32 = 72;

/// Darkness of each pixel of the font's characters, taken from the station's graphs.
#[rustfmt::skip]
const GLYPHS: [(char, [[u8; PITCH]; GLYPH_HEIGHT]); 11] = [
    ('0', [
        [  0,  82, 216, 217,  86,   0],
        [ 11, 222,  48,  47, 224,  15],
        [ 60, 178,   0,   0, 176,  67],
        [ 78, 160, 111, 111, 157,  85],
        [ 60, 178,   0,   0, 176,  67],
        [ 12, 222,  46,  46, 225,  15],
        [  0,  84, 217, 218,  88,   0],
    ]),
    ('1', [
        [  0, 194, 243, 202,   0,   0],
        [  0,   0,  34, 202,   0,   0],
        [  0,   0,  34, 202,   0,   0],
        [  0,   0,  34, 202,   0,   0],
        [  0,   0,  34, 202,   0,   0],
        [  0,   0,  34, 202,   0,   0],
        [  0, 168, 243, 243, 243,  84],
    ]),
    ('2', [
        [  3, 127, 225, 215,  92,   0],
        [ 46, 121,  20,  53, 236,  11],
        [  0,   0,   0,   8, 219,   4],
        [  0,   0,   1, 148,  84,   0],
        [  0,   3, 147,  93,   0,   0],
        [  7, 162,  88,   0,   0,   0],
        [ 61, 243, 243, 243, 243,  42],
    ]),
    ('3', [
        [  1, 106, 221, 214,  86,   0],
        [ 25, 130,  20,  44, 229,   6],
        [  0,   0,   1,  46, 218,   5],
        [  0,  19, 243, 239,  88,   0],
        [  0,   0,   1,  36, 208,  28],
        [ 70,  96,  12,  34, 214,  44],
        [ 10, 143, 228, 219, 118,   0],
    ]),
    ('4', [
        [  0,   0,  12, 205, 141,   0],
        [  0,   0, 137, 129, 141,   0],
        [  0,  80,  86,  99, 141,   0],
        [ 26, 141,   0,  99, 141,   0],
        [119, 243, 243, 243, 243, 133],
        [  0,   0,   0,  99, 141,   0],
        [  0,   0,   0,  99, 141,   0],
    ]),
    ('5', [
        [  0, 240, 243, 243, 172,   0],
        [  0, 217,   0,   0,   0,   0],
        [  0, 235, 232, 210,  75,   0],
        [  0,   0,   4,  68, 226,  12],
        [  0,   0,   0,   0, 197,  43],
        [  0,   0,   3,  65, 223,  11],
        [ 72, 243, 241, 205,  70,   0],
    ]),
    ('6', [
        [  0,  53, 196, 239, 206,   0],
        [  7, 218,  95,   6,   0,   0],
        [ 56, 192,   0,   0,   0,   0],
        [ 77, 168, 197, 231, 151,   2],
        [ 61, 215,  29,  16, 190,  69],
        [ 14, 206,  29,  15, 190,  68],
        [  0,  87, 215, 231, 150,   2],
    ]),
    ('7', [
        [ 80, 243, 243, 243, 243,  47],
        [  0,   0,   0,  18, 193,   0],
        [  0,   0,   0, 113, 106,   0],
        [  0,   0,   3, 206,  17,   0],
        [  0,   0,  77, 156,   0,   0],
        [  0,   0, 180,  60,   0,   0],
        [  0,  40, 207,   0,   0,   0],
    ]),
    ('8', [
        [  0, 121, 226, 227, 121,   0],
        [ 28, 224,  29,  26, 219,  31],
        [ 14, 211,  27,  24, 208,  17],
        [  0, 105, 241, 242, 110,   0],
        [ 48, 197,  21,  21, 195,  51],
        [ 69, 198,  21,  19, 194,  72],
        [  3, 147, 229, 229, 147,   3],
    ]),
    ('9', [
        [  2, 147, 230, 214,  86,   0],
        [ 69, 189,  16,  29, 206,  14],
        [ 69, 189,  15,  29, 215,  60],
        [  3, 152, 232, 194, 171,  76],
        [  0,   0,   0,   0, 197,  54],
        [  0,   0,   6,  97, 217,   7],
        [  0, 206, 239, 197,  53,   0],
    ]),
    ('.', [
        [0; PITCH],
        [0; PITCH],
        [0; PITCH],
        [0; PITCH],
        [0; PITCH],
        [0; PITCH],
        [  0,   0, 149, 152,   0,   0],
    ]),
];

/// Reads the line of each of `colors` from a graph.
pub fn digitize(image: &RgbImage, colors: &[[u8; 3]]) -> Result<Vec<Vec<Point>>, String> {
    let plot = Plot::find(image).ok_or("no plot area found")?;
    let value = value_axis(image, &plot).ok_or("could not read the value axis")?;
    let time = time_axis(image, &plot).ok_or("could not read the time axis")?;
    Ok(colors
        .iter()
        .map(|&color| trace(image, &plot, color, &time, &value))
        .collect())
}

/// The white canvas the lines are drawn on; bounds are inclusive.
#[derive(Debug, PartialEq)]
struct Plot {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

impl Plot {
    fn find(image: &RgbImage) -> Option<Self> {
        let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
        for (x, y, pixel) in image.enumerate_pixels() {
            if pixel.0 == [255, 255, 255] {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x);
                bottom = bottom.max(y);
            }
        }
        (right > left + 100 && bottom > top + 50).then_some(Self {
            left,
            top,
            right,
            bottom,
        })
    }
}

/// `y = slope · x + offset`, fitted by least squares.
#[derive(Debug, Clone, Copy)]
struct Linear {
    slope: f64,
    offset: f64,
}

impl Linear {
    fn fit(pairs: &[(f64, f64)]) -> Option<Self> {
        if pairs.len() < 2 {
            return None;
        }
        let n = pairs.len() as f64;
        let mean_x = pairs.iter().map(|p| p.0).sum::<f64>() / n;
        let mean_y = pairs.iter().map(|p| p.1).sum::<f64>() / n;
        let sxx: f64 = pairs.iter().map(|p| (p.0 - mean_x).powi(2)).sum();
        let sxy: f64 = pairs.iter().map(|p| (p.0 - mean_x) * (p.1 - mean_y)).sum();
        (sxx > 0.0).then(|| {
            let slope = sxy / sxx;
            Self {
                slope,
                offset: mean_y - slope * mean_x,
            }
        })
    }

    fn at(&self, x: f64) -> f64 {
        self.slope * x + self.offset
    }
}

fn darkness(image: &RgbImage, x: usize, y: usize) -> u8 {
    let Some(pixel) = image.get_pixel_checked(x as u32, y as u32) else {
        return 0;
    };
    let [r, g, b] = pixel.0.map(i32::from);
    let luma = (299 * r + 587 * g + 114 * b) / 1000;
    (BACKGROUND_LUMA - luma).clamp(0, 255) as u8
}

/// Runs of consecutive rows in `rows` that have ink between columns `x0..x1`.
fn ink_bands(
    image: &RgbImage,
    x0: usize,
    x1: usize,
    rows: std::ops::Range<usize>,
) -> Vec<(usize, usize)> {
    let mut bands: Vec<(usize, usize)> = Vec::new();
    for y in rows {
        if !(x0..x1).any(|x| darkness(image, x, y) > INK) {
            continue;
        }
        match bands.last_mut() {
            Some(band) if band.1 + 1 == y => band.1 = y,
            _ => bands.push((y, y)),
        }
    }
    bands
}

fn cell(image: &RgbImage, left: usize, top: usize) -> [[u8; PITCH]; GLYPH_HEIGHT] {
    std::array::from_fn(|row| std::array::from_fn(|col| darkness(image, left + col, top + row)))
}

/// The character in a cell and how far it is from the template.
fn classify(cell: &[[u8; PITCH]; GLYPH_HEIGHT]) -> (char, u32) {
    // No graph has shown a minus sign yet to take a template from: it is a short bar
    // in the middle rows.
    let ink_rows: Vec<usize> = (0..GLYPH_HEIGHT)
        .filter(|&row| cell[row].iter().any(|&d| d > INK))
        .collect();
    let ink: usize = cell.iter().flatten().filter(|&&d| d > INK).count();
    if !ink_rows.is_empty() && ink_rows.iter().all(|row| (2..=4).contains(row)) && ink >= 3 {
        return ('-', 0);
    }
    GLYPHS
        .iter()
        .map(|(ch, glyph)| {
            let score = glyph
                .iter()
                .flatten()
                .zip(cell.iter().flatten())
                .map(|(&a, &b)| u32::from(a.abs_diff(b)))
                .sum();
            (*ch, score)
        })
        .min_by_key(|&(_, score)| score)
        .expect("there are templates")
}

/// The text of the label whose ink lies in columns `x0..x1` of the glyph row at `top`,
/// and the first column and number of its cells.
fn read_label(
    image: &RgbImage,
    x0: usize,
    x1: usize,
    top: usize,
) -> Option<(String, usize, usize)> {
    let has_ink = |x: usize| (top..top + GLYPH_HEIGHT).any(|y| darkness(image, x, y) > INK);
    let first = (x0..x1).find(|&x| has_ink(x))?;
    let last = (x0..x1).rev().find(|&x| has_ink(x))?;
    // A character's ink needn't fill its cell, so try every alignment of the cells.
    (first.saturating_sub(PITCH - 1)..=first)
        .map(|origin| {
            let cells = (last + 1 - origin).div_ceil(PITCH);
            let (text, score) = (0..cells)
                .map(|k| classify(&cell(image, origin + PITCH * k, top)))
                .fold((String::new(), 0), |(mut text, total), (ch, score)| {
                    text.push(ch);
                    (text, total + score)
                });
            (text, score, origin, cells)
        })
        .min_by_key(|&(_, score, _, _)| score)
        .filter(|&(_, score, _, cells)| score <= MAX_CELL_SCORE * cells as u32)
        .map(|(text, _, origin, cells)| (text, origin, cells))
}

/// Maps a row to a value, from the labels left of the plot, each centered on its grid line.
fn value_axis(image: &RgbImage, plot: &Plot) -> Option<Linear> {
    let (left, top, bottom) = (plot.left as usize, plot.top as usize, plot.bottom as usize);
    // Stops short of the axis line, whose end would read as part of the lowest label.
    let x1 = left.checked_sub(4)?;
    let rows = top.saturating_sub(8)..bottom + 8;
    let pairs: Vec<(f64, f64)> = ink_bands(image, 0, x1, rows)
        .into_iter()
        .filter(|&(first, last)| last + 1 - first == GLYPH_HEIGHT)
        .filter_map(|(first, _)| {
            let (text, _, _) = read_label(image, 0, x1, first)?;
            let value: f64 = text.parse().ok()?;
            Some(((first + GLYPH_HEIGHT / 2) as f64, value))
        })
        .collect();
    Linear::fit(&pairs)
}

/// Maps a column to minutes past the first midnight, from the hour labels below the plot.
fn time_axis(image: &RgbImage, plot: &Plot) -> Option<Linear> {
    let (left, right, bottom) = (
        plot.left as usize,
        plot.right as usize,
        plot.bottom as usize,
    );
    let x0 = left.saturating_sub(12);
    let x1 = (right + 13).min(image.width() as usize);
    // The tick marks just below the axis aren't labels.
    let (first, _) = ink_bands(image, x0, x1, bottom + 5..bottom + 20)
        .into_iter()
        .find(|&(first, last)| last + 1 - first == GLYPH_HEIGHT)?;

    let mut labels: Vec<(usize, usize)> = Vec::new();
    for x in x0..x1 {
        if !(first..first + GLYPH_HEIGHT).any(|y| darkness(image, x, y) > INK) {
            continue;
        }
        match labels.last_mut() {
            Some(label) if x <= label.1 + 3 => label.1 = x,
            _ => labels.push((x, x)),
        }
    }

    let mut pairs = Vec::new();
    let mut previous: Option<f64> = None;
    for (start, end) in labels {
        let Some((text, origin, cells)) = read_label(image, start, end + 1, first) else {
            continue;
        };
        let Ok(hour) = text.parse::<u32>() else {
            continue;
        };
        if hour >= 24 {
            continue;
        }
        let mut minutes = f64::from(hour * 60);
        // Hours restart after midnight; keep counting from the first day.
        if let Some(previous) = previous {
            while minutes <= previous {
                minutes += 24.0 * 60.0;
            }
        }
        previous = Some(minutes);
        let center = origin as f64 + (cells * PITCH) as f64 / 2.0 - 0.5;
        pairs.push((snap_to_grid(image, plot, center), minutes));
    }
    Linear::fit(&pairs)
}

/// The column of the hour grid line under a label, when there is one close by.
fn snap_to_grid(image: &RgbImage, plot: &Plot, center: f64) -> f64 {
    let is_major_grid = |pixel: &image::Rgb<u8>| {
        let [r, g, b] = pixel.0.map(i32::from);
        g == b && g >= 150 && r >= 200 && r - g >= 40
    };
    let height = plot.bottom - plot.top + 1;
    let x = center.round() as i64;
    (x - 1..=x + 1)
        .filter(|&x| x >= i64::from(plot.left) && x <= i64::from(plot.right))
        .map(|x| {
            let count = (plot.top..=plot.bottom)
                .filter(|&y| is_major_grid(image.get_pixel(x as u32, y)))
                .count();
            (x, count)
        })
        .filter(|&(_, count)| count as u32 * 4 >= height)
        .max_by_key(|&(_, count)| count)
        .map_or(center, |(x, _)| x as f64)
}

/// Follows the line drawn in `color`, one point per column where it is visible.
fn trace(
    image: &RgbImage,
    plot: &Plot,
    color: [u8; 3],
    time: &Linear,
    value: &Linear,
) -> Vec<Point> {
    let matches = |x: u32, y: u32| {
        let pixel = image.get_pixel(x, y).0;
        let distance: i32 = pixel
            .iter()
            .zip(color)
            .map(|(&a, b)| (i32::from(a) - i32::from(b)).pow(2))
            .sum();
        distance <= COLOR_TOLERANCE * COLOR_TOLERANCE
    };
    let columns: Vec<(u32, u32, u32)> = (plot.left..=plot.right)
        .filter_map(|x| {
            let mut rows = (plot.top..=plot.bottom).filter(|&y| matches(x, y));
            let first = rows.next()?;
            let last = rows.next_back().unwrap_or(first);
            Some((x, first, last))
        })
        .collect();

    // The line's thickness is the most common height of a column without a step.
    let mut heights = [0_usize; 8];
    for &(_, first, last) in &columns {
        if let Some(count) = heights.get_mut((last - first) as usize) {
            *count += 1;
        }
    }
    let thickness = (0..heights.len()).max_by_key(|&h| heights[h]).unwrap_or(0) as f64;

    // A column with a step holds the line from the previous level to the new one; the
    // new level is the end farther from where the line was.
    let mut points = Vec::with_capacity(columns.len());
    let mut level: Option<f64> = None;
    for (x, first, last) in columns {
        let (top, bottom) = (f64::from(first), f64::from(last));
        let row = match level {
            Some(previous) if bottom - top > thickness => {
                if (top - previous).abs() > (bottom - previous).abs() {
                    top + thickness / 2.0
                } else {
                    bottom - thickness / 2.0
                }
            }
            _ => (top + bottom) / 2.0,
        };
        level = Some(row);
        points.push(Point {
            minutes: time.at(f64::from(x)),
            value: value.at(row) as f32,
        });
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> RgbImage {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        image::open(path).expect("fixture").to_rgb8()
    }

    #[test]
    fn finds_the_plot_area() {
        let plot = Plot::find(&fixture("graf.png")).unwrap();
        assert_eq!(
            plot,
            Plot {
                left: 51,
                top: 15,
                right: 750,
                bottom: 214
            }
        );
    }

    #[test]
    fn reads_the_axis_labels() {
        let image = fixture("graf_tlak.png");
        let plot = Plot::find(&image).unwrap();
        let value = value_axis(&image, &plot).unwrap();
        // 1200 at the top grid line, 1000 on the axis.
        assert!((value.at(15.0) - 1200.0).abs() < 1e-6);
        assert!((value.at(165.0) - 1000.0).abs() < 1e-6);

        let time = time_axis(&image, &plot).unwrap();
        // "10" is under the grid line at x = 81, and the graph spans 24 hours.
        assert!((time.at(81.0) - 600.0).abs() < 0.5, "{}", time.at(81.0));
        let span = time.at(750.0) - time.at(51.0);
        assert!((span - 24.0 * 60.0).abs() < 15.0, "{span}");
    }

    #[test]
    fn recognizes_a_minus_sign() {
        let mut minus = [[0; PITCH]; GLYPH_HEIGHT];
        minus[3] = [0, 180, 243, 243, 180, 0];
        assert_eq!(classify(&minus).0, '-');
        assert_eq!(classify(&GLYPHS[0].1).0, '0');
        assert_eq!(classify(&GLYPHS[10].1).0, '.');
    }

    fn extremes(points: &[Point]) -> (Point, Point) {
        let min = *points
            .iter()
            .min_by(|a, b| a.value.total_cmp(&b.value))
            .unwrap();
        let max = *points
            .iter()
            .max_by(|a, b| a.value.total_cmp(&b.value))
            .unwrap();
        (min, max)
    }

    fn hour_of(point: Point) -> f64 {
        point.minutes.rem_euclid(24.0 * 60.0) / 60.0
    }

    #[test]
    fn reads_the_temperature_and_wind_chill() {
        let traces = digitize(
            &fixture("graf.png"),
            &[[0x00, 0x00, 0xa4], [0xff, 0xcc, 0x00], [0xff, 0x00, 0x00]],
        )
        .unwrap();
        let [temperature, wind_chill, heat_index] = &traces[..] else {
            panic!("three traces");
        };
        assert!(temperature.len() > 650, "{}", temperature.len());
        assert!(temperature.windows(2).all(|w| w[0].minutes < w[1].minutes));

        let (min, max) = extremes(temperature);
        assert!((min.value - 0.6).abs() < 0.15, "{min:?}");
        assert!((6.0..7.5).contains(&hour_of(min)), "{min:?}");
        assert!((max.value - 8.8).abs() < 0.15, "{max:?}");
        assert!((14.5..16.5).contains(&hour_of(max)), "{max:?}");
        let last = temperature.last().unwrap();
        assert!((last.value - 5.9).abs() < 0.2, "{last:?}");
        assert!((8.5..9.2).contains(&hour_of(*last)), "{last:?}");

        // Wind chill only shows where it differs from the temperature, in a dip near 23:00.
        let (dip, _) = extremes(wind_chill);
        assert!((dip.value - 3.6).abs() < 0.2, "{dip:?}");
        assert!((22.0..23.5).contains(&hour_of(dip)), "{dip:?}");
        assert!(heat_index.is_empty());
    }

    #[test]
    fn reads_pressure_and_humidity() {
        let pressure = &digitize(&fixture("graf_tlak.png"), &[[0x99, 0x00, 0xcc]]).unwrap()[0];
        let last = pressure.last().unwrap();
        assert!((1020.0..1025.0).contains(&last.value), "{last:?}");
        let (min, _) = extremes(pressure);
        assert!((1010.0..1015.0).contains(&min.value), "{min:?}");

        let humidity = &digitize(&fixture("graf_hum.png"), &[[0xff, 0x33, 0x00]]).unwrap()[0];
        let (min, max) = extremes(humidity);
        assert!((min.value - 84.0).abs() < 0.6, "{min:?}");
        assert!((12.5..14.0).contains(&hour_of(min)), "{min:?}");
        assert!((max.value - 93.0).abs() < 0.6, "{max:?}");
    }
}
