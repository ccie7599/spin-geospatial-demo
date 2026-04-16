// ============================================================
// geohash.rs — Geohash encoding, decoding, neighbors, and
//              cell painting (center + radius → cell set)
//
// Zero external dependencies. Ported from geohash.js.
// ============================================================

use std::collections::{HashMap, HashSet};

const BASE32: &[u8] = b"0123456789bcdefghjkmnpqrstuvwxyz";
const EARTH_RADIUS: f64 = 6_371_000.0;

/// Approximate cell dimensions at equator (meters) by precision.
/// Width shrinks with cos(latitude); height is constant.
struct CellDim {
    w: f64,
    h: f64,
}

const CELL_DIMS: [CellDim; 13] = [
    CellDim { w: 0.0, h: 0.0 },         // index 0 (unused)
    CellDim { w: 5_009_400.0, h: 4_992_600.0 },  // 1
    CellDim { w: 1_252_300.0, h: 624_100.0 },     // 2
    CellDim { w: 156_500.0, h: 156_000.0 },       // 3
    CellDim { w: 39_100.0, h: 19_500.0 },         // 4
    CellDim { w: 4_900.0, h: 4_900.0 },           // 5
    CellDim { w: 1_200.0, h: 610.0 },             // 6
    CellDim { w: 153.0, h: 153.0 },               // 7
    CellDim { w: 38.0, h: 19.0 },                 // 8
    CellDim { w: 4.8, h: 4.8 },                   // 9
    CellDim { w: 1.2, h: 0.6 },                   // 10
    CellDim { w: 0.15, h: 0.15 },                 // 11
    CellDim { w: 0.037, h: 0.019 },               // 12
];

fn base32_inv(c: u8) -> Option<u8> {
    BASE32.iter().position(|&b| b == c).map(|i| i as u8)
}

// ============================================================
// Encode lat/lon to geohash
// ============================================================

pub fn encode(lat: f64, lon: f64, precision: u8) -> String {
    let mut idx: u8 = 0;
    let mut bit: u8 = 0;
    let mut even_bit = true;
    let mut lat_min = -90.0_f64;
    let mut lat_max = 90.0_f64;
    let mut lon_min = -180.0_f64;
    let mut lon_max = 180.0_f64;
    let mut geohash = String::with_capacity(precision as usize);

    while geohash.len() < precision as usize {
        if even_bit {
            let mid = (lon_min + lon_max) / 2.0;
            if lon >= mid {
                idx = idx * 2 + 1;
                lon_min = mid;
            } else {
                idx *= 2;
                lon_max = mid;
            }
        } else {
            let mid = (lat_min + lat_max) / 2.0;
            if lat >= mid {
                idx = idx * 2 + 1;
                lat_min = mid;
            } else {
                idx *= 2;
                lat_max = mid;
            }
        }
        even_bit = !even_bit;
        bit += 1;
        if bit == 5 {
            geohash.push(BASE32[idx as usize] as char);
            bit = 0;
            idx = 0;
        }
    }
    geohash
}

// ============================================================
// Decode geohash to lat/lon + error bounds
// ============================================================

pub struct DecodedHash {
    pub lat: f64,
    pub lon: f64,
    pub lat_min: f64,
    pub lat_max: f64,
    pub lon_min: f64,
    pub lon_max: f64,
    pub lat_err: f64,
    pub lon_err: f64,
}

pub fn decode(geohash: &str) -> DecodedHash {
    let mut lat_min = -90.0_f64;
    let mut lat_max = 90.0_f64;
    let mut lon_min = -180.0_f64;
    let mut lon_max = 180.0_f64;
    let mut even_bit = true;

    for ch in geohash.bytes() {
        let idx = base32_inv(ch).expect("invalid geohash character");
        for bit in (0..5).rev() {
            let val = (idx >> bit) & 1;
            if even_bit {
                let mid = (lon_min + lon_max) / 2.0;
                if val == 1 {
                    lon_min = mid;
                } else {
                    lon_max = mid;
                }
            } else {
                let mid = (lat_min + lat_max) / 2.0;
                if val == 1 {
                    lat_min = mid;
                } else {
                    lat_max = mid;
                }
            }
            even_bit = !even_bit;
        }
    }

    DecodedHash {
        lat: (lat_min + lat_max) / 2.0,
        lon: (lon_min + lon_max) / 2.0,
        lat_min,
        lat_max,
        lon_min,
        lon_max,
        lat_err: (lat_max - lat_min) / 2.0,
        lon_err: (lon_max - lon_min) / 2.0,
    }
}

// ============================================================
// Compute 8 neighbor cells
// ============================================================

pub fn neighbors(geohash: &str) -> HashMap<&'static str, String> {
    let d = decode(geohash);
    let precision = geohash.len() as u8;
    let dlat = d.lat_err * 2.0;
    let dlon = d.lon_err * 2.0;

    let offsets: &[(&str, f64, f64)] = &[
        ("n", 1.0, 0.0),
        ("ne", 1.0, 1.0),
        ("e", 0.0, 1.0),
        ("se", -1.0, 1.0),
        ("s", -1.0, 0.0),
        ("sw", -1.0, -1.0),
        ("w", 0.0, -1.0),
        ("nw", 1.0, -1.0),
    ];

    let mut result = HashMap::new();
    for &(name, dy, dx) in offsets {
        let nlat = d.lat + dy * dlat;
        let nlon = d.lon + dx * dlon;
        if (-90.0..=90.0).contains(&nlat) {
            let wrapped_lon = ((nlon + 540.0) % 360.0) - 180.0;
            result.insert(name, encode(nlat, wrapped_lon, precision));
        }
    }
    result
}

// ============================================================
// Degree / meter conversions
// ============================================================

pub fn meters_to_lat_deg(meters: f64) -> f64 {
    meters / 111_320.0
}

pub fn meters_to_lon_deg(meters: f64, lat: f64) -> f64 {
    meters / (111_320.0 * (lat * std::f64::consts::PI / 180.0).cos())
}

// ============================================================
// Suggest optimal geohash precision for a given radius
// ============================================================

pub fn precision_for_radius(radius_meters: f64, lat: f64) -> u8 {
    let cos_lat = (lat * std::f64::consts::PI / 180.0).cos();
    for p in 1..=12u8 {
        let dim = &CELL_DIMS[p as usize];
        let cell_w = dim.w * cos_lat;
        let cell_h = dim.h;
        if cell_w <= radius_meters * 2.0 && cell_h <= radius_meters * 2.0 {
            return p;
        }
    }
    12
}

// ============================================================
// CELL PAINTING — The core algorithm
//
// Given a center point, radius (meters), and precision, compute
// all geohash cells that intersect the circle.
// ============================================================

pub fn paint_cells(lat: f64, lon: f64, radius_meters: f64, precision: u8) -> Vec<String> {
    let mut cells = HashSet::new();

    let d_lat = meters_to_lat_deg(radius_meters);
    let d_lon = meters_to_lon_deg(radius_meters, lat);

    let lat_min = lat - d_lat;
    let lat_max = lat + d_lat;
    let lon_min = lon - d_lon;
    let lon_max = lon + d_lon;

    // Add corner cells
    for &clat in &[lat_min, lat_max] {
        for &clon in &[lon_min, lon_max] {
            let cl = clat.clamp(-89.999, 89.999);
            let co = clon.clamp(-179.999, 179.999);
            cells.insert(encode(cl, co, precision));
        }
    }

    // Get cell step size from decoding the center
    let center_hash = encode(lat, lon, precision);
    let d = decode(&center_hash);
    let step_lat = d.lat_err * 2.0;
    let step_lon = d.lon_err * 2.0;

    // Sweep bounding box at sub-cell resolution
    let mut y = lat_min;
    while y <= lat_max + step_lat {
        let mut x = lon_min;
        while x <= lon_max + step_lon {
            let clamped_lat = y.clamp(-89.999, 89.999);
            let clamped_lon = x.clamp(-179.999, 179.999);

            let dist = haversine_distance(lat, lon, clamped_lat, clamped_lon);
            if dist <= radius_meters * 1.2 {
                cells.insert(encode(clamped_lat, clamped_lon, precision));
            }

            x += step_lon * 0.5;
        }
        y += step_lat * 0.5;
    }

    cells.into_iter().collect()
}

// ============================================================
// Haversine distance in meters
// ============================================================

pub fn haversine_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    EARTH_RADIUS * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}

// ============================================================
// Human-readable cell size
// ============================================================

/// Returns the larger of the two cell dimensions (width/height) in meters at equator.
pub fn cell_max_dim_m(precision: u8) -> f64 {
    if precision == 0 || precision as usize >= CELL_DIMS.len() {
        return 5_009_400.0;
    }
    let dim = &CELL_DIMS[precision as usize];
    dim.w.max(dim.h)
}

pub fn cell_size_description(precision: u8) -> String {
    if precision == 0 || precision as usize >= CELL_DIMS.len() {
        return "unknown".to_string();
    }
    let dim = &CELL_DIMS[precision as usize];
    if dim.w >= 1000.0 {
        format!("~{}km x {}km", (dim.w / 1000.0) as u32, (dim.h / 1000.0) as u32)
    } else {
        format!("~{}m x {}m", dim.w, dim.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let lat = 33.6846;
        let lon = -117.8265;
        let hash = encode(lat, lon, 10);
        let d = decode(&hash);
        assert!((d.lat - lat).abs() < 0.001);
        assert!((d.lon - lon).abs() < 0.001);
    }

    #[test]
    fn test_neighbors_count() {
        let hash = encode(33.6846, -117.8265, 7);
        let nbrs = neighbors(&hash);
        assert_eq!(nbrs.len(), 8);
    }

    #[test]
    fn test_precision_for_radius() {
        // 75m store should paint at precision 7 (~153m cells)
        let p = precision_for_radius(75.0, 33.6846);
        assert_eq!(p, 7);

        // 10m department should paint at precision 9 (~4.8m cells)
        let p = precision_for_radius(10.0, 33.6846);
        assert_eq!(p, 9);
    }

    #[test]
    fn test_paint_cells_nonempty() {
        let cells = paint_cells(33.6846, -117.8265, 75.0, 7);
        assert!(!cells.is_empty());
        // A 75m radius at precision 7 (~153m cells) should paint 1-9 cells
        assert!(cells.len() <= 20);
    }

    #[test]
    fn test_haversine_distance() {
        // Approx distance between two known points
        let d = haversine_distance(33.6846, -117.8265, 33.7350, -117.8131);
        // Should be roughly 5-6km
        assert!(d > 5000.0 && d < 7000.0);
    }
}
