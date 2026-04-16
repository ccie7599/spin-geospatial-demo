// ============================================================
// spatial_store.rs — Spin KV-backed spatial data store
//
// Key schema:
//   spatial:{precision}:{geohash}  → JSON array of object refs
//   obj:{type}:{id}               → JSON SpatialObject
//   dev:{device_id}               → JSON DeviceState
//   store-depts:{store_id}        → JSON array of department refs
//
// Upward write amplification: objects painted at native precision
// are also painted at coarser levels for prefix-free queries.
// ============================================================

use std::collections::HashSet;

use serde::de::DeserializeOwned;
use serde::Serialize;
use spin_sdk::key_value::Store;

use crate::geohash::{cell_size_description, encode, haversine_distance, neighbors, paint_cells, precision_for_radius};
use crate::models::*;

// ============================================================
// KV helpers
// ============================================================

fn kv_get_json<T: DeserializeOwned>(store: &Store, key: &str) -> Option<T> {
    let bytes = store.get(key).ok()??;
    serde_json::from_slice(&bytes).ok()
}

fn kv_set_json<T: Serialize>(store: &Store, key: &str, value: &T) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    store.set(key, &bytes)?;
    Ok(())
}

fn kv_get_refs(store: &Store, key: &str) -> Vec<String> {
    kv_get_json::<Vec<String>>(store, key).unwrap_or_default()
}

fn append_ref_to_cell(store: &Store, key: &str, object_ref: &str) -> anyhow::Result<()> {
    let mut refs: Vec<String> = kv_get_json(store, key).unwrap_or_default();
    // Match on ref name prefix (before |lat|lon) to avoid duplicates across format changes
    let ref_name = object_ref.split('|').next().unwrap_or(object_ref);
    if !refs.iter().any(|r| r.split('|').next().unwrap_or(r) == ref_name) {
        refs.push(object_ref.to_string());
        kv_set_json(store, key, &refs)?;
    }
    Ok(())
}

/// Parse a ref string that may contain embedded coordinates: "type:id|lat|lon"
fn parse_ref_coords(ref_str: &str) -> (&str, Option<f64>, Option<f64>) {
    let mut parts = ref_str.splitn(3, '|');
    let name = parts.next().unwrap_or(ref_str);
    let lat = parts.next().and_then(|s| s.parse().ok());
    let lon = parts.next().and_then(|s| s.parse().ok());
    (name, lat, lon)
}

/// Determine which neighbors to check based on precision.
/// p10 (~1m): no neighbors — cell painting covers overlap
/// p9 (~5m): cardinal only (4 reads instead of 8)
/// p8 and below: all 8 neighbors
fn neighbor_directions_for_precision(precision: u8) -> &'static [&'static str] {
    match precision {
        10.. => &[],
        9 => &["n", "e", "s", "w"],
        _ => &["n", "ne", "e", "se", "s", "sw", "w", "nw"],
    }
}

// ============================================================
// Upward write amplification
// ============================================================

fn upward_paint_precisions(native_precision: u8) -> Vec<u8> {
    match native_precision {
        10 => vec![9],
        9 => vec![8],
        8 => vec![7, 5, 4, 3],
        _ => vec![],
    }
}

// ============================================================
// PAINT OBJECT — Write to spatial index + object store
// ============================================================

pub fn paint_object(store: &Store, obj: &ObjectIngest) -> anyhow::Result<PaintResult> {
    let radius_meters = obj.radius.unwrap_or(50.0);
    let precision = obj.precision.unwrap_or_else(|| precision_for_radius(radius_meters, obj.lat));
    let cells = paint_cells(obj.lat, obj.lon, radius_meters, precision);
    let object_ref = format!("{}:{}", obj.obj_type, obj.id);
    let ref_with_coords = format!("{object_ref}|{:.6}|{:.6}", obj.lat, obj.lon);
    let center_geohash = encode(obj.lat, obj.lon, precision);

    // Write object reference (with embedded coords) to each native spatial index cell
    for cell in &cells {
        let key = format!("spatial:{precision}:{cell}");
        append_ref_to_cell(store, &key, &ref_with_coords)?;
    }

    // Upward write amplification — paint at coarser precisions
    let mut upward_cells_written = 0usize;
    for parent_p in upward_paint_precisions(precision) {
        let mut parent_hashes = HashSet::new();
        for cell in &cells {
            let len = parent_p as usize;
            if cell.len() >= len {
                parent_hashes.insert(&cell[..len]);
            }
        }
        for parent_hash in &parent_hashes {
            let key = format!("spatial:{parent_p}:{parent_hash}");
            append_ref_to_cell(store, &key, &ref_with_coords)?;
            upward_cells_written += 1;
        }
    }

    // Write full object to object store
    let total_cells = cells.len() + upward_cells_written;
    let full_object = SpatialObject {
        obj_type: obj.obj_type.clone(),
        id: obj.id.clone(),
        lat: obj.lat,
        lon: obj.lon,
        radius: radius_meters,
        geohash: center_geohash.clone(),
        precision,
        cells_painted: total_cells,
        metadata: obj.metadata.clone(),
        ingested_at: iso_timestamp(),
        distance_mi: None,
    };
    let obj_key = format!("obj:{}:{}", obj.obj_type, obj.id);
    kv_set_json(store, &obj_key, &full_object)?;

    // If department, append to store-depts index
    if obj.obj_type == "department" {
        if let Some(store_id) = obj.metadata.get("storeId").and_then(|v| v.as_str()) {
            let depts_key = format!("store-depts:{store_id}");
            append_ref_to_cell(store, &depts_key, &object_ref)?;
        }
    }

    Ok(PaintResult {
        object_ref,
        center_geohash,
        precision,
        cell_size: cell_size_description(precision),
        cells_painted: total_cells,
        cells: cells,
    })
}

// ============================================================
// QUERY POINT — Direct KV reads with dedup + hydration
// ============================================================

pub fn query_point(store: &Store, lat: f64, lon: f64, opts: &QueryOpts) -> QueryResult {
    let full_geohash = encode(lat, lon, 12);
    // Map: ref_name → (lat, lon) from embedded coords
    let mut ref_coords: std::collections::HashMap<String, (f64, f64)> = std::collections::HashMap::new();
    let mut cells_queried = Vec::new();
    let mut total_refs = 0usize;

    for p in opts.min_precision..=opts.precision {
        let cell = &full_geohash[..p as usize];
        let key = format!("spatial:{p}:{cell}");

        let refs = kv_get_refs(store, &key);
        let ref_count = refs.len();
        total_refs += ref_count;
        for r in &refs {
            let (name, olat, olon) = parse_ref_coords(r);
            if !ref_coords.contains_key(name) {
                ref_coords.insert(name.to_string(), (olat.unwrap_or(0.0), olon.unwrap_or(0.0)));
            }
        }
        cells_queried.push(CellQuery {
            precision: Some(p),
            cell: cell.to_string(),
            direction: None,
            refs: ref_count,
        });

        // Check neighbors based on precision level
        if opts.include_neighbors {
            let allowed_dirs = neighbor_directions_for_precision(p);
            if !allowed_dirs.is_empty() {
                let nbrs = neighbors(cell);
                for dir in allowed_dirs {
                    if let Some(nbr_hash) = nbrs.get(dir) {
                        let nbr_key = format!("spatial:{p}:{nbr_hash}");
                        let nbr_refs = kv_get_refs(store, &nbr_key);
                        let nbr_count = nbr_refs.len();
                        total_refs += nbr_count;
                        for r in &nbr_refs {
                            let (name, olat, olon) = parse_ref_coords(r);
                            if !ref_coords.contains_key(name) {
                                ref_coords.insert(name.to_string(), (olat.unwrap_or(0.0), olon.unwrap_or(0.0)));
                            }
                        }
                        if nbr_count > 0 {
                            cells_queried.push(CellQuery {
                                precision: Some(p),
                                cell: nbr_hash.clone(),
                                direction: Some(dir.to_string()),
                                refs: nbr_count,
                            });
                        }
                    }
                }
            }
        }
    }

    // Pre-hydration radius filter using embedded coordinates
    let unique_before_filter = ref_coords.len();
    let refs_to_hydrate: HashSet<String> = if let Some(radius_m) = opts.radius_m {
        ref_coords
            .iter()
            .filter(|(_, &(olat, olon))| {
                // If coords are 0,0 (old format without embedded coords), always hydrate
                if olat == 0.0 && olon == 0.0 { return true; }
                haversine_distance(lat, lon, olat, olon) <= radius_m
            })
            .map(|(name, _)| name.clone())
            .collect()
    } else {
        ref_coords.keys().cloned().collect()
    };

    let mut objects = hydrate_objects(store, &refs_to_hydrate);

    // Annotate distance
    let meters_per_mile = 1609.344;
    for obj in &mut objects {
        let dist_m = haversine_distance(lat, lon, obj.lat, obj.lon);
        obj.distance_mi = Some((dist_m / meters_per_mile * 10.0).round() / 10.0);
    }

    // Post-hydration radius filter (catches old-format refs that were hydrated unconditionally)
    if let Some(radius_m) = opts.radius_m {
        objects.retain(|obj| {
            let dist_m = haversine_distance(lat, lon, obj.lat, obj.lon);
            dist_m <= radius_m
        });
    }

    let skipped = unique_before_filter - refs_to_hydrate.len();

    QueryResult {
        query: QueryMeta {
            lat: Some(lat),
            lon: Some(lon),
            geohash: Some(full_geohash),
            max_precision: Some(opts.precision),
            min_precision: Some(opts.min_precision),
            precision: None,
            cell_size: None,
            include_neighbors: None,
        },
        unique_object_ids: objects.len(),
        duplicates_eliminated: total_refs.saturating_sub(unique_before_filter) + skipped,
        objects,
        cells_queried,
    }
}

// ============================================================
// QUERY AREA — Direct KV reads on a geohash cell
// ============================================================

pub fn query_area(store: &Store, geohash: &str, include_neighbors: bool) -> QueryResult {
    let precision = geohash.len() as u8;
    let mut object_ids = HashSet::new();
    let mut cells_queried = Vec::new();

    // Primary cell
    let key = format!("spatial:{precision}:{geohash}");
    let refs = kv_get_refs(store, &key);
    let ref_count = refs.len();
    for r in &refs {
        let (name, _, _) = parse_ref_coords(r);
        object_ids.insert(name.to_string());
    }
    cells_queried.push(CellQuery {
        precision: None,
        cell: geohash.to_string(),
        direction: None,
        refs: ref_count,
    });

    // Neighbors — use precision-based strategy
    if include_neighbors {
        let allowed_dirs = neighbor_directions_for_precision(precision);
        if !allowed_dirs.is_empty() {
            let nbrs = neighbors(geohash);
            for dir in allowed_dirs {
                if let Some(nbr_hash) = nbrs.get(dir) {
                    let nbr_key = format!("spatial:{precision}:{nbr_hash}");
                    let nbr_refs = kv_get_refs(store, &nbr_key);
                    let nbr_count = nbr_refs.len();
                    for r in &nbr_refs {
                        let (name, _, _) = parse_ref_coords(r);
                        object_ids.insert(name.to_string());
                    }
                    if nbr_count > 0 {
                        cells_queried.push(CellQuery {
                            precision: None,
                            cell: nbr_hash.clone(),
                            direction: Some(dir.to_string()),
                            refs: nbr_count,
                        });
                    }
                }
            }
        }
    }

    let objects = hydrate_objects(store, &object_ids);

    QueryResult {
        query: QueryMeta {
            lat: None,
            lon: None,
            geohash: Some(geohash.to_string()),
            max_precision: None,
            min_precision: None,
            precision: Some(precision),
            cell_size: Some(cell_size_description(precision)),
            include_neighbors: Some(include_neighbors),
        },
        unique_object_ids: object_ids.len(),
        duplicates_eliminated: 0,
        objects,
        cells_queried,
    }
}

// ============================================================
// DEVICE STATE
// ============================================================

pub fn get_device_state(store: &Store, device_id: &str) -> Option<DeviceState> {
    let key = format!("dev:{device_id}");
    kv_get_json(store, &key)
}

pub fn set_device_state(store: &Store, device_id: &str, state: &DeviceState) -> anyhow::Result<()> {
    let key = format!("dev:{device_id}");
    kv_set_json(store, &key, state)
}

// ============================================================
// EVENT DETECTION — unchanged logic
// ============================================================

pub fn detect_events(
    previous_state: Option<&DeviceState>,
    current_context: &QueryResult,
    device_id: &str,
    timestamp: &str,
) -> (Vec<Event>, DeviceState) {
    let mut events = Vec::new();

    let current_stores: Vec<&str> = current_context
        .objects
        .iter()
        .filter(|o| o.obj_type == "store")
        .map(|o| o.id.as_str())
        .collect();
    let current_depts: Vec<String> = current_context
        .objects
        .iter()
        .filter(|o| o.obj_type == "department")
        .map(|o| {
            o.metadata
                .get("department")
                .and_then(|v| v.as_str())
                .unwrap_or(&o.id)
                .to_string()
        })
        .collect();

    let is_in_store = !current_stores.is_empty();
    let current_store_id = current_stores.first().copied();
    let current_dept = current_depts.first().cloned();

    let was_in_store = previous_state.and_then(|s| s.store_id.as_ref()).is_some();
    let prev_store_id = previous_state.and_then(|s| s.store_id.clone());
    let prev_dept = previous_state.and_then(|s| s.department.clone());

    let mut new_state = DeviceState {
        device_id: device_id.to_string(),
        store_id: current_store_id.map(|s| s.to_string()),
        department: current_dept.clone(),
        last_seen: timestamp.to_string(),
        consecutive_outside: 0,
        entered_at: previous_state.and_then(|s| s.entered_at.clone()),
    };

    if !was_in_store && is_in_store {
        // ENTER
        events.push(Event {
            event_type: "device.entered_store".to_string(),
            store_id: current_store_id.unwrap().to_string(),
            department: None,
            previous_department: None,
            dwell_seconds: None,
            timestamp: timestamp.to_string(),
        });
        new_state.entered_at = Some(timestamp.to_string());
    } else if was_in_store && !is_in_store {
        // POTENTIALLY EXITED — hysteresis
        let consecutive = previous_state
            .map(|s| s.consecutive_outside)
            .unwrap_or(0)
            + 1;
        new_state.consecutive_outside = consecutive;
        new_state.store_id = prev_store_id.clone();
        new_state.department = prev_dept;
        new_state.entered_at = previous_state.and_then(|s| s.entered_at.clone());

        if consecutive >= 3 {
            // CONFIRMED EXIT
            let dwell_seconds = compute_dwell_seconds(
                previous_state.and_then(|s| s.entered_at.as_deref()),
                timestamp,
            );
            events.push(Event {
                event_type: "device.exited_store".to_string(),
                store_id: prev_store_id.unwrap_or_default(),
                department: None,
                previous_department: None,
                dwell_seconds,
                timestamp: timestamp.to_string(),
            });
            new_state.store_id = None;
            new_state.department = None;
            new_state.entered_at = None;
            new_state.consecutive_outside = 0;
        }
    } else if was_in_store && is_in_store {
        // STILL IN STORE
        new_state.entered_at = previous_state.and_then(|s| s.entered_at.clone());

        if let Some(ref dept) = current_dept {
            if prev_dept.as_ref() != Some(dept) {
                events.push(Event {
                    event_type: "device.entered_department".to_string(),
                    store_id: current_store_id.unwrap().to_string(),
                    department: Some(dept.clone()),
                    previous_department: prev_dept,
                    dwell_seconds: None,
                    timestamp: timestamp.to_string(),
                });
            }
        }
    }

    (events, new_state)
}

// ============================================================
// Helpers
// ============================================================

/// Convert lat/lon to floor plan coordinates relative to a store's bounding box.
/// Returns (x, y) in [0, 1] range where (0,0) is SW corner and (1,1) is NE corner.
pub fn to_floor_plan(store: &SpatialObject, lat: f64, lon: f64) -> (f64, f64) {
    let delta_lat = store.radius / 111_320.0;
    let delta_lon = store.radius / (111_320.0 * store.lat.to_radians().cos());
    let sw_lat = store.lat - delta_lat;
    let sw_lon = store.lon - delta_lon;
    let ne_lat = store.lat + delta_lat;
    let ne_lon = store.lon + delta_lon;
    let x = (lon - sw_lon) / (ne_lon - sw_lon);
    let y = (lat - sw_lat) / (ne_lat - sw_lat);
    (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))
}

/// Fetch all department objects for a given store ID.
pub fn get_store_departments(store: &Store, store_id: &str) -> Vec<SpatialObject> {
    let depts_key = format!("store-depts:{store_id}");
    let refs = kv_get_refs(store, &depts_key);
    let ref_set: HashSet<String> = refs.into_iter().collect();
    hydrate_objects(store, &ref_set)
}

/// Look up a spatial object by key from the KV store.
pub fn get_object(store: &Store, key: &str) -> Option<SpatialObject> {
    let kv_key = format!("obj:{key}");
    kv_get_json(store, &kv_key)
}

/// Format a distance in meters as human-readable text.
pub fn format_distance(meters: f64) -> String {
    if meters < 1000.0 {
        format!("{:.0}m", meters)
    } else {
        let miles = meters / 1609.34;
        format!("{:.1} mi", miles)
    }
}

fn hydrate_objects(store: &Store, object_ids: &HashSet<String>) -> Vec<SpatialObject> {
    let mut objects = Vec::new();
    for ref_str in object_ids {
        let key = format!("obj:{ref_str}");
        if let Some(obj) = kv_get_json::<SpatialObject>(store, &key) {
            objects.push(obj);
        }
    }
    objects
}

/// Compute dwell time in seconds between two ISO timestamps.
fn compute_dwell_seconds(entered_at: Option<&str>, exited_at: &str) -> Option<i64> {
    let entered = entered_at?;
    let enter_ms = parse_iso_ms(entered)?;
    let exit_ms = parse_iso_ms(exited_at)?;
    Some((exit_ms - enter_ms) / 1000)
}

fn parse_iso_ms(ts: &str) -> Option<i64> {
    let ts = ts.trim_end_matches('Z');
    let (date, time) = ts.split_once('T')?;
    let date_parts: Vec<&str> = date.split('-').collect();
    let time_parts: Vec<&str> = time.split(':').collect();
    if date_parts.len() != 3 || time_parts.len() != 3 {
        return None;
    }
    let year: i64 = date_parts[0].parse().ok()?;
    let month: i64 = date_parts[1].parse().ok()?;
    let day: i64 = date_parts[2].parse().ok()?;
    let hour: i64 = time_parts[0].parse().ok()?;
    let min: i64 = time_parts[1].parse().ok()?;
    let sec_str = time_parts[2];
    let (sec, millis) = if let Some((s, ms)) = sec_str.split_once('.') {
        let s: i64 = s.parse().ok()?;
        let ms: i64 = ms.get(..3).unwrap_or(ms).parse().ok()?;
        (s, ms)
    } else {
        (sec_str.parse::<i64>().ok()?, 0)
    };
    let days = (year - 1970) * 365 + (month - 1) * 30 + day;
    let total_ms = ((days * 24 + hour) * 60 + min) * 60_000 + sec * 1000 + millis;
    Some(total_ms)
}

pub fn iso_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();

    let mut days = (secs / 86400) as i64;
    let day_secs = (secs % 86400) as u32;
    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let sec = day_secs % 60;

    let mut year = 1970i32;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut month = 0u32;
    for (i, &md) in month_days.iter().enumerate() {
        if days < md {
            month = i as u32 + 1;
            break;
        }
        days -= md;
    }
    let day = days + 1;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}.{millis:03}Z")
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
