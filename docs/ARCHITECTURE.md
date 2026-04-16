# Architecture

Companion to [DIAGRAMS.md](DIAGRAMS.md). This document focuses on the **API contract** and the **venue/section data model**.

---

## Data model

The code uses two generic spatial object types — `store` and `department` — deliberately named to stay compatible with any venue-with-internal-zones pattern (retail stores with departments, libraries with sections, museums with wings, stadiums with concourses, etc.).

```
SpatialObject
  ├── type: "store" | "department" | (any string)
  ├── id: string
  ├── lat, lon: f64
  ├── radius: f64 (meters)
  ├── geohash: string
  ├── precision: u8
  ├── cellsPainted: usize     ← how many spatial:{p}:{geohash} keys hold this ref
  └── metadata: map<string, json>
```

In the library demo:

| Generic type | Library meaning | Radius |
|--- |--- |--- |
| `store` | A library branch | ~35 m (typical building footprint) |
| `department` | A section within a branch | ~6 m (section zone) |

Section names are: `children`, `fiction`, `nonfiction`, `reference`, `periodicals`, `audiovisual`, `computers`, `community`.

---

## API reference

### `POST /ingest/store`

Paint a venue + its sections. Computes cell coverage, writes the spatial index, and writes the full objects to the object store.

```json
{
  "storeId": "nypl-mid-manhattan",
  "name": "Mid-Manhattan Library",
  "lat": 40.7531, "lon": -73.9822,
  "radius": 35,
  "address": "455 5th Ave, New York, NY 10016",
  "departments": [
    { "name": "children",   "lat": 40.75297, "lon": -73.98232, "radius": 8 },
    { "name": "fiction",    "lat": 40.75305, "lon": -73.98220, "radius": 8 },
    { "name": "reference",  "lat": 40.75322, "lon": -73.98230, "radius": 6 }
  ]
}
```

### `GET /api/v1/checkin?lat=…&lon=…&deviceId=…`

The primary client API. Returns either an `inStore: true` payload (with zone detection) or an `inStore: false` payload with the nearest venue.

**In venue:**
```json
{
  "inStore": true,
  "store": {
    "storeId": "nypl-mid-manhattan",
    "name": "Mid-Manhattan Library",
    "address": "455 5th Ave, New York, NY 10016",
    "departments": [
      { "name": "children",  "slug": "children"  },
      { "name": "fiction",   "slug": "fiction"   },
      { "name": "reference", "slug": "reference" }
    ],
    "promosEndpoint": "/api/v1/stores/nypl-mid-manhattan/context"
  },
  "zone": "reference",
  "event": "device.entered_department"
}
```

**Outside any venue:**
```json
{
  "inStore": false,
  "nearestStore": {
    "storeId": "nypl-mid-manhattan",
    "name": "Mid-Manhattan Library",
    "distance": 1240.6,
    "distanceText": "0.8 mi"
  }
}
```

### `GET /api/v1/stores/:id/find?q=…`

Search sections inside a venue by name. Used for wayfinding (*"where's the reference section?"*).

### `GET /api/v1/stores/:id/position?lat=…&lon=…`

Floor-plan coordinate conversion. Maps a real-world lat/lon into the `[0, 1] × [0, 1]` box of the venue's bounding circle and reports the current zone + nearby sections sorted by distance.

### `GET /api/v1/stores/:id/context?lat=…&lon=…`

Zone-aware content. Returns a `hero` block chosen by the current zone plus a distance-sorted list of nearby sections. In the library demo, the hero is a **reading-list teaser** — `title` names the list (e.g. *"Fiction staff picks: new this month"*) and `couponCode` (kept as-is from the underlying generic schema) carries the list identifier a client would dereference to fetch the full items. In a real deployment the hero block would come from a CMS.

### System / ingest APIs

| Method | Path | Purpose |
|--- |--- |--- |
| `POST` | `/ingest/object` | Paint any spatial object (generic, venue-agnostic) |
| `POST` | `/ingest/location` | S2S device check-in with event stream |
| `GET`  | `/query/point/:lat/:lon[/:precision]` | Raw spatial query |
| `GET`  | `/query/area/:geohash` | All objects in a geohash cell |
| `GET`  | `/query/store/:id` | Fetch a stored venue |
| `GET`  | `/health` | Version, storage, schema |

---

## Event model

`detect_events` is called on every `/api/v1/checkin` and `/ingest/location` call. It compares the current spatial result against the stored `DeviceState` and emits zero or one event:

| Event | Triggered when |
|--- |--- |
| `device.entered_store` | Previously outside any venue → now inside one |
| `device.entered_department` | Same venue, different section than last time |
| `device.exited_store` | `consecutive_outside ≥ 3` pings, with `dwell_seconds` computed against `entered_at` |

There is no `stayed_in` event — callers can infer dwell from successive `entered_at` timestamps.

---

## Design decisions worth knowing

- **Geohash over H3 / S2 / MGRS.** Geohash prefixes nest cleanly (a p5 geohash is a prefix of a p8 geohash in the same cell), which maps directly onto `spatial:{precision}:{geohash}` keys. H3 and S2 have better cell shapes but their ids don't prefix-nest, so a flat KV store can't exploit them without a secondary index.
- **Upward amplification over prefix scans.** Spin KV has no prefix/scan operator. Writing the same ref at multiple precisions at ingest time is a write-once, read-many trade: queries stay simple direct reads at any precision.
- **Radius filter before hydration.** Embedding lat/lon in each `spatial:…` ref lets us haversine-filter refs *before* fetching the full object from `obj:…`, which is a large win in dense geohashes.
- **Precision-tuned neighbor expansion.** At precision 10 (~1 m cells) the paint algorithm already covers enough overlap that neighbor reads are redundant — skip them. At precision 9 we read 4 cardinal neighbors, at precision ≤ 8 we read all 8. This keeps the per-call KV fan-out bounded.
- **3-ping exit hysteresis.** GPS jitter near a geofence boundary produces spurious "outside" readings — we require three consecutive to trigger an exit event.
