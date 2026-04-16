// ============================================================
// lib.rs — Spin HTTP component
// Geospatial Edge PoC — Akamai Functions (Fermyon Spin)
// ============================================================

mod geohash;
mod models;
mod spatial_store;

use spin_sdk::http::{IntoResponse, Params, Request, Response, Router};
use spin_sdk::http_component;
use spin_sdk::key_value::Store;

use geohash::{encode, haversine_distance};
use models::*;
use spatial_store::*;

// ============================================================
// Spin entry point
// ============================================================

#[http_component]
fn handle_request(req: Request) -> anyhow::Result<impl IntoResponse> {
    let mut router = Router::new();

    // System APIs
    router.post("/ingest/object", handle_ingest_object);
    router.post("/ingest/store", handle_ingest_store);
    router.post("/ingest/location", handle_ingest_location);
    router.get("/query/point/:lat/:lon/:precision", handle_query_point);
    router.get("/query/point/:lat/:lon", handle_query_point);
    router.get("/query/area/:geohash", handle_query_area);
    router.get("/query/store/:storeId", handle_query_store);
    router.get("/health", handle_health);

    // Customer-facing APIs
    router.get("/api/v1/checkin", handle_checkin);
    router.get("/api/v1/stores/:storeId/find", handle_find);
    router.get("/api/v1/stores/:storeId/position", handle_position);
    router.get("/api/v1/stores/:storeId/context", handle_context);

    Ok(router.handle(req))
}

// ============================================================
// Helper: open KV store
// ============================================================

fn open_store() -> anyhow::Result<Store> {
    Ok(Store::open_default()?)
}

// ============================================================
// POST /ingest/object
// ============================================================

fn handle_ingest_object(req: Request, _params: Params) -> anyhow::Result<impl IntoResponse> {
    let body: ObjectIngest = match parse_body(&req) {
        Ok(b) => b,
        Err(e) => return Ok(json_response(400, &ErrorResponse { error: e.to_string() })),
    };
    let store = open_store()?;
    match paint_object(&store, &body) {
        Ok(result) => Ok(json_response(
            200,
            &IngestObjectResponse {
                status: "ok".into(),
                result,
            },
        )),
        Err(e) => Ok(json_response(400, &ErrorResponse { error: e.to_string() })),
    }
}

// ============================================================
// POST /ingest/store
// ============================================================

fn handle_ingest_store(req: Request, _params: Params) -> anyhow::Result<impl IntoResponse> {
    let body: StoreIngest = match parse_body(&req) {
        Ok(b) => b,
        Err(e) => return Ok(json_response(400, &ErrorResponse { error: e.to_string() })),
    };

    if body.lat == 0.0 && body.lon == 0.0 {
        return Ok(json_response(
            400,
            &ErrorResponse {
                error: "storeId, lat, lon required".into(),
            },
        ));
    }

    let store = open_store()?;
    let mut results = Vec::new();

    // Paint the store itself
    let store_obj = ObjectIngest {
        obj_type: "store".into(),
        id: body.store_id.clone(),
        lat: body.lat,
        lon: body.lon,
        radius: Some(body.radius.unwrap_or(75.0)),
        precision: None,
        metadata: {
            let mut m = std::collections::HashMap::new();
            m.insert(
                "name".into(),
                serde_json::Value::String(
                    body.name
                        .clone()
                        .unwrap_or_else(|| format!("Store #{}", body.store_id)),
                ),
            );
            if let Some(ref addr) = body.address {
                m.insert("address".into(), serde_json::Value::String(addr.clone()));
            }
            m
        },
    };
    results.push(paint_object(&store, &store_obj)?);

    // Paint each department
    for dept in &body.departments {
        let dept_obj = ObjectIngest {
            obj_type: "department".into(),
            id: format!("{}-{}", body.store_id, dept.name),
            lat: dept.lat,
            lon: dept.lon,
            radius: Some(dept.radius.unwrap_or(10.0)),
            precision: None,
            metadata: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "storeId".into(),
                    serde_json::Value::String(body.store_id.clone()),
                );
                m.insert(
                    "department".into(),
                    serde_json::Value::String(dept.name.clone()),
                );
                if let Some(ref name) = body.name {
                    m.insert("storeName".into(), serde_json::Value::String(name.clone()));
                }
                m
            },
        };
        results.push(paint_object(&store, &dept_obj)?);
    }

    let total_cells: usize = results.iter().map(|r| r.cells_painted).sum();

    Ok(json_response(
        200,
        &StoreIngestResponse {
            status: "ok".into(),
            store_id: body.store_id,
            total_objects_painted: results.len(),
            total_cells_written: total_cells,
            details: results,
        },
    ))
}

// ============================================================
// POST /ingest/location
// ============================================================

fn handle_ingest_location(req: Request, _params: Params) -> anyhow::Result<impl IntoResponse> {
    let body: LocationIngest = match parse_body(&req) {
        Ok(b) => b,
        Err(e) => return Ok(json_response(400, &ErrorResponse { error: e.to_string() })),
    };

    let store = open_store()?;
    let timestamp = iso_timestamp();

    let opts = QueryOpts {
        precision: 9,
        min_precision: 7,
        include_neighbors: true,
        radius_m: None,
    };
    let context = query_point(&store, body.lat, body.lon, &opts);

    let previous_state = get_device_state(&store, &body.device_id);
    let (events, new_state) =
        detect_events(previous_state.as_ref(), &context, &body.device_id, &timestamp);

    set_device_state(&store, &body.device_id, &new_state)?;

    let response = LocationResponse {
        device_id: body.device_id.clone(),
        geohash: encode(body.lat, body.lon, 10),
        context: ContextData {
            store_id: new_state.store_id.clone(),
            department: new_state.department.clone(),
            objects_nearby: context
                .objects
                .iter()
                .map(|o| NearbyObject {
                    obj_type: o.obj_type.clone(),
                    id: o.id.clone(),
                    name: o
                        .metadata
                        .get("name")
                        .or_else(|| o.metadata.get("department"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(&o.id)
                        .to_string(),
                    distance: None,
                })
                .collect(),
        },
        events: if events.is_empty() {
            None
        } else {
            Some(events)
        },
        state: new_state,
        debug: DebugInfo {
            unique_objects_found: context.unique_object_ids,
            duplicates_eliminated: context.duplicates_eliminated,
            cells_queried: context.cells_queried.len(),
        },
    };

    Ok(json_response(200, &response))
}

// ============================================================
// GET /query/point/:lat/:lon(/:precision)
// ============================================================

fn handle_query_point(_req: Request, params: Params) -> anyhow::Result<impl IntoResponse> {
    let lat: f64 = params
        .get("lat")
        .and_then(|s| s.parse().ok())
        .unwrap_or(f64::NAN);
    let lon: f64 = params
        .get("lon")
        .and_then(|s| s.parse().ok())
        .unwrap_or(f64::NAN);

    if lat.is_nan() || lon.is_nan() {
        return Ok(json_response(
            400,
            &ErrorResponse {
                error: "Invalid lat/lon".into(),
            },
        ));
    }

    let precision: u8 = params
        .get("precision")
        .and_then(|s| s.parse().ok())
        .unwrap_or(9);

    let store = open_store()?;
    // Stores are painted at precisions 3, 4, 5, 7, 8 via upward write amplification.
    // Clamp minimum query precision to 3 (coarsest painted level).
    let max_precision = precision.max(3);
    let radius_m = crate::geohash::cell_max_dim_m(precision) / 2.0;
    let opts = QueryOpts {
        precision: max_precision,
        min_precision: precision,
        include_neighbors: true,
        radius_m: Some(radius_m),
    };
    let result = query_point(&store, lat, lon, &opts);

    Ok(json_response(200, &result))
}

// ============================================================
// GET /query/area/:geohash
// ============================================================

fn handle_query_area(_req: Request, params: Params) -> anyhow::Result<impl IntoResponse> {
    let geohash = params.get("geohash").unwrap_or("");
    let store = open_store()?;
    let result = query_area(&store, geohash, true);
    Ok(json_response(200, &result))
}

// ============================================================
// GET /query/store/:storeId
// ============================================================

fn handle_query_store(_req: Request, params: Params) -> anyhow::Result<impl IntoResponse> {
    let store_id = params.get("storeId").unwrap_or("");
    let store = open_store()?;

    match get_object(&store, &format!("store:{store_id}")) {
        Some(obj) => Ok(json_response(200, &obj)),
        None => Ok(json_response(
            404,
            &ErrorResponse {
                error: format!("Store {store_id} not found"),
            },
        )),
    }
}

// ============================================================
// GET /health
// ============================================================

fn handle_health(_req: Request, _params: Params) -> anyhow::Result<impl IntoResponse> {
    Ok(json_response(
        200,
        &HealthResponse {
            status: "ok".into(),
            service: "spin-geospatial-demo".into(),
            version: "0.1.0".into(),
            architecture: ArchitectureInfo {
                compute: "Fermyon Spin (Akamai Functions)".into(),
                storage: "Spin KV (key-value store, upward write amplification for prefix-free queries)".into(),
                spatial_index: "Geohash with cell painting + Spin KV direct reads + upward amplification".into(),
                event_detection: "Stateful enter/exit/dwell with 3-ping hysteresis".into(),
            },
            key_schema: KeySchemaInfo {
                spatial_index: "spatial:{precision}:{geohash} → JSON array of object refs".into(),
                object_store: "obj:{type}:{id} → JSON SpatialObject".into(),
                device_state: "dev:{device_id} → JSON DeviceState".into(),
            },
            endpoints: vec![
                "POST /ingest/object              — Paint any object across geohash cells",
                "POST /ingest/store               — Ingest store + departments",
                "POST /ingest/location            — Device check-in → context + events",
                "GET  /query/point/:lat/:lon(/:p) — Raw spatial query",
                "GET  /query/area/:geohash        — Area query",
                "GET  /query/store/:storeId       — Store lookup",
                "GET  /api/v1/checkin             — In-store mode trigger",
                "GET  /api/v1/stores/:id/find     — Wayfinding search",
                "GET  /api/v1/stores/:id/position — Blue dot positioning",
                "GET  /api/v1/stores/:id/context  — Contextual content",
            ],
        },
    ))
}

// ============================================================
// GET /api/v1/checkin — In-Store Mode Trigger
// ============================================================

fn handle_checkin(req: Request, _params: Params) -> anyhow::Result<impl IntoResponse> {
    let qs = parse_query_string(req.uri());
    let lat: f64 = qs.get("lat").and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
    let lon: f64 = qs.get("lon").and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
    let device_id = qs.get("deviceId").cloned().unwrap_or_default();

    if lat.is_nan() || lon.is_nan() || device_id.is_empty() {
        return Ok(json_response(400, &ErrorResponse {
            error: "lat, lon, and deviceId required".into(),
        }));
    }

    let store = open_store()?;

    // High-precision query for in-store detection
    let opts = QueryOpts { precision: 9, min_precision: 7, include_neighbors: true, radius_m: None };
    let context = query_point(&store, lat, lon, &opts);

    let stores: Vec<&SpatialObject> = context.objects.iter()
        .filter(|o| o.obj_type == "store")
        .collect();

    if let Some(s) = stores.first() {
        // IN STORE — run event detection
        let previous_state = get_device_state(&store, &device_id);
        let timestamp = iso_timestamp();
        let (events, new_state) = detect_events(previous_state.as_ref(), &context, &device_id, &timestamp);
        set_device_state(&store, &device_id, &new_state)?;

        let departments: Vec<CheckinDepartment> = context.objects.iter()
            .filter(|o| o.obj_type == "department")
            .map(|o| {
                let dept_name = o.metadata.get("department")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&o.id);
                CheckinDepartment {
                    name: dept_name.to_string(),
                    slug: dept_name.to_lowercase().replace(' ', "-"),
                }
            })
            .collect();

        let event_str = events.first().map(|e| e.event_type.clone());

        Ok(json_response(200, &CheckinResponse {
            in_store: true,
            store: Some(CheckinStore {
                store_id: s.id.clone(),
                name: s.metadata.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&s.id)
                    .to_string(),
                address: s.metadata.get("address")
                    .and_then(|v| v.as_str())
                    .map(|a| a.to_string()),
                departments,
                promos_endpoint: format!("/api/v1/stores/{}/context", s.id),
            }),
            zone: new_state.department.clone(),
            event: event_str,
            nearest_store: None,
        }))
    } else {
        // NOT IN STORE — find nearest at wider precision
        let wide_opts = QueryOpts { precision: 5, min_precision: 4, include_neighbors: true, radius_m: None };
        let wide_context = query_point(&store, lat, lon, &wide_opts);

        let nearest = wide_context.objects.iter()
            .filter(|o| o.obj_type == "store")
            .map(|o| {
                let dist = haversine_distance(lat, lon, o.lat, o.lon);
                (o, dist)
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(json_response(200, &CheckinResponse {
            in_store: false,
            store: None,
            zone: None,
            event: None,
            nearest_store: nearest.map(|(ns, dist)| NearestStore {
                store_id: ns.id.clone(),
                name: ns.metadata.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&ns.id)
                    .to_string(),
                distance: dist,
                distance_text: format_distance(dist),
            }),
        }))
    }
}

// ============================================================
// GET /api/v1/stores/:storeId/find — Wayfinding
// ============================================================

fn handle_find(req: Request, params: Params) -> anyhow::Result<impl IntoResponse> {
    let store_id = params.get("storeId").unwrap_or("");
    let qs = parse_query_string(req.uri());
    let query = qs.get("q").cloned().unwrap_or_default();

    if query.is_empty() {
        return Ok(json_response(400, &ErrorResponse {
            error: "q (search query) required".into(),
        }));
    }

    let store = open_store()?;

    // Look up store
    let store_obj: SpatialObject = match get_object(&store, &format!("store:{store_id}")) {
        Some(s) => s,
        None => return Ok(json_response(404, &ErrorResponse {
            error: format!("Store {store_id} not found"),
        })),
    };

    // Get all departments for this store
    let departments = get_store_departments(&store, store_id);
    let query_lower = query.to_lowercase();

    let results: Vec<FindResult> = departments.iter()
        .filter(|d| {
            let dept_name = d.metadata.get("department")
                .and_then(|v| v.as_str())
                .unwrap_or(&d.id);
            dept_name.to_lowercase().contains(&query_lower)
        })
        .map(|d| {
            let dept_name = d.metadata.get("department")
                .and_then(|v| v.as_str())
                .unwrap_or(&d.id)
                .to_string();
            let (x, y) = to_floor_plan(&store_obj, d.lat, d.lon);
            let dist = haversine_distance(store_obj.lat, store_obj.lon, d.lat, d.lon);
            FindResult {
                result_type: "department".into(),
                name: dept_name,
                location: FindLocation {
                    floor_plan: FloorPlanCoord { x: round2(x), y: round2(y) },
                    distance: Some(round2(dist)),
                },
            }
        })
        .collect();

    Ok(json_response(200, &FindResponse {
        store_id: store_id.to_string(),
        results,
    }))
}

// ============================================================
// GET /api/v1/stores/:storeId/position — Blue Dot
// ============================================================

fn handle_position(req: Request, params: Params) -> anyhow::Result<impl IntoResponse> {
    let store_id = params.get("storeId").unwrap_or("");
    let qs = parse_query_string(req.uri());
    let lat: f64 = qs.get("lat").and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
    let lon: f64 = qs.get("lon").and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);

    if lat.is_nan() || lon.is_nan() {
        return Ok(json_response(400, &ErrorResponse {
            error: "lat and lon required".into(),
        }));
    }

    let store = open_store()?;

    let store_obj: SpatialObject = match get_object(&store, &format!("store:{store_id}")) {
        Some(s) => s,
        None => return Ok(json_response(404, &ErrorResponse {
            error: format!("Store {store_id} not found"),
        })),
    };

    // Floor plan position
    let (x, y) = to_floor_plan(&store_obj, lat, lon);

    // Zone detection
    let opts = QueryOpts { precision: 9, min_precision: 8, include_neighbors: true, radius_m: None };
    let context = query_point(&store, lat, lon, &opts);

    let current_dept = context.objects.iter()
        .filter(|o| o.obj_type == "department")
        .next()
        .and_then(|o| o.metadata.get("department").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    // Nearby departments with distances
    let departments = get_store_departments(&store, store_id);
    let mut nearby: Vec<NearbyItem> = departments.iter()
        .map(|d| {
            let dept_name = d.metadata.get("department")
                .and_then(|v| v.as_str())
                .unwrap_or(&d.id)
                .to_string();
            let dist = haversine_distance(lat, lon, d.lat, d.lon);
            NearbyItem {
                item_type: "department".into(),
                name: dept_name,
                distance: round2(dist),
                distance_text: format_distance(dist),
            }
        })
        .collect();
    nearby.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));

    Ok(json_response(200, &PositionResponse {
        position: PositionData {
            floor_plan: FloorPlanCoord { x: round2(x), y: round2(y) },
            accuracy: 8,
        },
        zone: ZoneInfo { department: current_dept },
        nearby,
    }))
}

// ============================================================
// GET /api/v1/stores/:storeId/context — Contextual Content
// ============================================================

fn handle_context(req: Request, params: Params) -> anyhow::Result<impl IntoResponse> {
    let store_id = params.get("storeId").unwrap_or("");
    let qs = parse_query_string(req.uri());
    let lat: f64 = qs.get("lat").and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);
    let lon: f64 = qs.get("lon").and_then(|s| s.parse().ok()).unwrap_or(f64::NAN);

    if lat.is_nan() || lon.is_nan() {
        return Ok(json_response(400, &ErrorResponse {
            error: "lat and lon required".into(),
        }));
    }

    let store = open_store()?;

    let store_obj: SpatialObject = match get_object(&store, &format!("store:{store_id}")) {
        Some(s) => s,
        None => return Ok(json_response(404, &ErrorResponse {
            error: format!("Store {store_id} not found"),
        })),
    };

    // Zone detection
    let opts = QueryOpts { precision: 9, min_precision: 8, include_neighbors: true, radius_m: None };
    let context = query_point(&store, lat, lon, &opts);

    let current_dept = context.objects.iter()
        .filter(|o| o.obj_type == "department")
        .next()
        .and_then(|o| o.metadata.get("department").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    // Stub hero content based on current zone. In a real deployment this would
    // come from a CMS keyed by (venue_id, zone) — the spatial layer only needs
    // to tell the content layer *where* the visitor is.
    let hero = current_dept.as_ref().map(|dept| {
        match dept.as_str() {
            "children" => HeroPromo {
                title: "Storytime today at 3:00pm — all ages welcome".into(),
                coupon_code: Some("EVT-STORYTIME".into()),
            },
            "reference" => HeroPromo {
                title: "Librarian on duty — ask for research help".into(),
                coupon_code: None,
            },
            "fiction" => HeroPromo {
                title: "New arrivals: this week's bestsellers are in".into(),
                coupon_code: Some("NEW-ARRIVALS".into()),
            },
            "periodicals" => HeroPromo {
                title: "Today's papers and magazines have arrived".into(),
                coupon_code: None,
            },
            "computers" => HeroPromo {
                title: "Free Wi-Fi — computers reservable in 1-hour blocks".into(),
                coupon_code: None,
            },
            _ => HeroPromo {
                title: format!("Welcome to the {dept} section"),
                coupon_code: None,
            },
        }
    });

    // Departments nearby with distances and floor plan coords
    let departments = get_store_departments(&store, store_id);
    let mut dept_contexts: Vec<DepartmentContext> = departments.iter()
        .map(|d| {
            let dept_name = d.metadata.get("department")
                .and_then(|v| v.as_str())
                .unwrap_or(&d.id)
                .to_string();
            let dist = haversine_distance(lat, lon, d.lat, d.lon);
            let (fx, fy) = to_floor_plan(&store_obj, d.lat, d.lon);
            DepartmentContext {
                name: dept_name.clone(),
                slug: dept_name.to_lowercase().replace(' ', "-"),
                floor_plan: FloorPlanCoord { x: round2(fx), y: round2(fy) },
                distance: round2(dist),
                distance_text: format_distance(dist),
            }
        })
        .collect();
    dept_contexts.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));

    Ok(json_response(200, &ContextResponse {
        zone: current_dept,
        content: ContextContent {
            hero,
            departments_nearby: dept_contexts,
        },
    }))
}

// ============================================================
// Helpers
// ============================================================

/// Parse query string from URI into a HashMap.
fn parse_query_string(uri: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Some(qs) = uri.split('?').nth(1) {
        for pair in qs.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(k.to_string(), v.to_string());
            }
        }
    }
    map
}

/// Round to 2 decimal places.
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn json_response<T: serde::Serialize>(status: u16, body: &T) -> Response {
    let json = serde_json::to_string_pretty(body).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(json)
        .build()
}

fn parse_body<T: serde::de::DeserializeOwned>(req: &Request) -> anyhow::Result<T> {
    serde_json::from_slice(req.body()).map_err(|e| anyhow::anyhow!("Invalid JSON: {}", e))
}
