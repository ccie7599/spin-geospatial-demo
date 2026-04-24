use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================
// Request types
// ============================================================

#[derive(Debug, Deserialize)]
pub struct ObjectIngest {
    #[serde(rename = "type")]
    pub obj_type: String,
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    pub radius: Option<f64>,
    pub precision: Option<u8>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct StoreIngest {
    #[serde(rename = "storeId")]
    pub store_id: String,
    pub name: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub radius: Option<f64>,
    pub address: Option<String>,
    #[serde(default)]
    pub departments: Vec<DepartmentIngest>,
}

#[derive(Debug, Deserialize)]
pub struct DepartmentIngest {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub radius: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct LocationIngest {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub lat: f64,
    pub lon: f64,
    pub accuracy: Option<f64>,
}

// ============================================================
// Response types
// ============================================================

#[derive(Debug, Serialize)]
pub struct PaintResult {
    #[serde(rename = "objectRef")]
    pub object_ref: String,
    #[serde(rename = "centerGeohash")]
    pub center_geohash: String,
    pub precision: u8,
    #[serde(rename = "cellSize")]
    pub cell_size: String,
    #[serde(rename = "cellsPainted")]
    pub cells_painted: usize,
    pub cells: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IngestObjectResponse {
    pub status: String,
    #[serde(flatten)]
    pub result: PaintResult,
}

#[derive(Debug, Serialize)]
pub struct StoreIngestResponse {
    pub status: String,
    #[serde(rename = "storeId")]
    pub store_id: String,
    #[serde(rename = "totalObjectsPainted")]
    pub total_objects_painted: usize,
    #[serde(rename = "totalCellsWritten")]
    pub total_cells_written: usize,
    pub details: Vec<PaintResult>,
}

#[derive(Debug, Serialize)]
pub struct QueryResult {
    pub query: QueryMeta,
    #[serde(rename = "uniqueObjectIds")]
    pub unique_object_ids: usize,
    #[serde(rename = "duplicatesEliminated")]
    pub duplicates_eliminated: usize,
    pub objects: Vec<SpatialObject>,
    #[serde(rename = "cellsQueried")]
    pub cells_queried: Vec<CellQuery>,
}

#[derive(Debug, Serialize)]
pub struct QueryMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geohash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "maxPrecision")]
    pub max_precision: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "minPrecision")]
    pub min_precision: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "cellSize")]
    pub cell_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "includeNeighbors")]
    pub include_neighbors: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct CellQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
    pub cell: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    pub refs: usize,
}

#[derive(Debug, Serialize)]
pub struct LocationResponse {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub geohash: String,
    pub context: ContextData,
    pub events: Option<Vec<Event>>,
    pub state: DeviceState,
    pub debug: DebugInfo,
}

#[derive(Debug, Serialize)]
pub struct ContextData {
    #[serde(rename = "storeId")]
    pub store_id: Option<String>,
    pub department: Option<String>,
    #[serde(rename = "objectsNearby")]
    pub objects_nearby: Vec<NearbyObject>,
}

#[derive(Debug, Serialize)]
pub struct NearbyObject {
    #[serde(rename = "type")]
    pub obj_type: String,
    pub id: String,
    pub name: String,
    pub distance: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct DebugInfo {
    #[serde(rename = "uniqueObjectsFound")]
    pub unique_objects_found: usize,
    #[serde(rename = "duplicatesEliminated")]
    pub duplicates_eliminated: usize,
    #[serde(rename = "cellsQueried")]
    pub cells_queried: usize,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
    pub architecture: ArchitectureInfo,
    #[serde(rename = "keySchema")]
    pub key_schema: KeySchemaInfo,
    pub endpoints: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct ArchitectureInfo {
    pub compute: String,
    pub storage: String,
    #[serde(rename = "spatialIndex")]
    pub spatial_index: String,
    #[serde(rename = "eventDetection")]
    pub event_detection: String,
}

#[derive(Debug, Serialize)]
pub struct KeySchemaInfo {
    #[serde(rename = "spatialIndex")]
    pub spatial_index: String,
    #[serde(rename = "objectStore")]
    pub object_store: String,
    #[serde(rename = "deviceState")]
    pub device_state: String,
}

// ============================================================
// Internal / stored types
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpatialObject {
    #[serde(rename = "type")]
    pub obj_type: String,
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    pub radius: f64,
    pub geohash: String,
    pub precision: u8,
    #[serde(rename = "cellsPainted")]
    pub cells_painted: usize,
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(rename = "ingestedAt")]
    pub ingested_at: String,
    #[serde(rename = "distanceMi", skip_serializing_if = "Option::is_none", skip_deserializing)]
    pub distance_mi: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceState {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    #[serde(rename = "storeId")]
    pub store_id: Option<String>,
    pub department: Option<String>,
    #[serde(rename = "lastSeen")]
    pub last_seen: String,
    #[serde(rename = "consecutiveOutside")]
    pub consecutive_outside: u32,
    #[serde(rename = "enteredAt")]
    pub entered_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Event {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "storeId")]
    pub store_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "previousDepartment")]
    pub previous_department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "dwellSeconds")]
    pub dwell_seconds: Option<i64>,
    pub timestamp: String,
}

// ============================================================
// Query options
// ============================================================

#[derive(Debug, Clone)]
pub struct QueryOpts {
    pub precision: u8,
    pub min_precision: u8,
    pub include_neighbors: bool,
    pub radius_m: Option<f64>,
}

impl Default for QueryOpts {
    fn default() -> Self {
        Self {
            precision: 9,
            min_precision: 5,
            include_neighbors: true,
            radius_m: None,
        }
    }
}

// ============================================================
// API #2 — Checkin response
// ============================================================

#[derive(Debug, Serialize)]
pub struct CheckinResponse {
    #[serde(rename = "inStore")]
    pub in_store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<CheckinStore>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "nearestStore")]
    pub nearest_store: Option<NearestStore>,
}

#[derive(Debug, Serialize)]
pub struct CheckinStore {
    #[serde(rename = "storeId")]
    pub store_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    pub departments: Vec<CheckinDepartment>,
    #[serde(rename = "promosEndpoint")]
    pub promos_endpoint: String,
}

#[derive(Debug, Serialize)]
pub struct CheckinDepartment {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Serialize)]
pub struct NearestStore {
    #[serde(rename = "storeId")]
    pub store_id: String,
    pub name: String,
    pub distance: f64,
    #[serde(rename = "distanceText")]
    pub distance_text: String,
}

// ============================================================
// API #3 — Find (wayfinding) response
// ============================================================

#[derive(Debug, Serialize)]
pub struct FindResponse {
    #[serde(rename = "storeId")]
    pub store_id: String,
    pub results: Vec<FindResult>,
}

#[derive(Debug, Serialize)]
pub struct FindResult {
    #[serde(rename = "type")]
    pub result_type: String,
    pub name: String,
    pub location: FindLocation,
}

#[derive(Debug, Serialize)]
pub struct FindLocation {
    #[serde(rename = "floorPlan")]
    pub floor_plan: FloorPlanCoord,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f64>,
}

#[derive(Debug, Serialize, Clone)]
pub struct FloorPlanCoord {
    pub x: f64,
    pub y: f64,
}

// ============================================================
// API #4 — Position (blue dot) response
// ============================================================

#[derive(Debug, Serialize)]
pub struct PositionResponse {
    pub position: PositionData,
    pub zone: ZoneInfo,
    pub nearby: Vec<NearbyItem>,
}

#[derive(Debug, Serialize)]
pub struct PositionData {
    #[serde(rename = "floorPlan")]
    pub floor_plan: FloorPlanCoord,
    pub accuracy: u8,
}

#[derive(Debug, Serialize)]
pub struct ZoneInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NearbyItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub name: String,
    pub distance: f64,
    #[serde(rename = "distanceText")]
    pub distance_text: String,
}

// ============================================================
// API #5 — Context response
// ============================================================

#[derive(Debug, Serialize)]
pub struct ContextResponse {
    pub zone: Option<String>,
    pub content: ContextContent,
}

#[derive(Debug, Serialize)]
pub struct ContextContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero: Option<HeroPromo>,
    #[serde(rename = "readingList", skip_serializing_if = "Vec::is_empty")]
    pub reading_list: Vec<Book>,
    #[serde(rename = "departmentsNearby")]
    pub departments_nearby: Vec<DepartmentContext>,
}

#[derive(Debug, Serialize)]
pub struct Book {
    pub title: String,
    pub author: String,
    #[serde(rename = "callNumber")]
    pub call_number: String,
}

#[derive(Debug, Serialize)]
pub struct HeroPromo {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "couponCode")]
    pub coupon_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DepartmentContext {
    pub name: String,
    pub slug: String,
    #[serde(rename = "floorPlan")]
    pub floor_plan: FloorPlanCoord,
    pub distance: f64,
    #[serde(rename = "distanceText")]
    pub distance_text: String,
}

// ============================================================
// Error response
// ============================================================

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}
