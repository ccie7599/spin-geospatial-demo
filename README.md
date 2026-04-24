# Step Inside

*From the globe down to the shelf.*

**Live demo:** [geospatial.connected-cloud.io](https://geospatial.connected-cloud.io/)

Edge-native venue proximity on **[Akamai Functions](https://techdocs.akamai.com/cloud-computing/docs/fermyon-wasm-functions)** (Fermyon Spin, WASM) — the same spatial primitives work for libraries, retail, stadiums, hotels, or anywhere visitors cross a threshold. Deployable from zero to live in minutes on Akamai's managed serverless edge, no cluster, no VMs, no runtime to patch.

The reference demo ingests **17,980 US public libraries** (OpenStreetMap, ODbL) and exposes an API that answers, in ~2–4 ms of compute time:

- *Am I inside a library right now? Which one, and which section am I in?*
- *What are the nearest libraries to this lat/lon?*
- *Given I'm at this position, what's my coordinate on the branch floor plan?*
- *What contextual content should we show for the section I'm standing in?*

All state lives in **Spin KV** (no external database). The spatial index is a geohash-based **cell-painting** scheme with **upward write amplification**, so point queries at any precision are a small, fixed number of direct KV reads — no scans, no prefix wildcards, no secondary index to maintain.

```
┌──────────────┐    HTTPS    ┌─────────────────────────────┐    KV    ┌───────────┐
│ Mobile / web ├────────────▶│  Spin WASM  (Rust / wasip1) ├─────────▶│  Spin KV  │
└──────────────┘             │  ~440 KB binary             │          └───────────┘
                             └─────────────────────────────┘
```

---

## Why this exists

There are plenty of ways to do geospatial: PostGIS, Redis GEO, H3 + a table scan, a managed service like Radar. This repo demonstrates a different shape: **ship the spatial logic to the edge, keep all state in a key-value store, and use geohash prefix math to avoid scans entirely.** The whole system compiles to a ~440 KB WASM binary, cold-starts in under a millisecond, and reads from a local KV store that can be replicated across edge locations.

It's also a reference for the **cell painting + upward amplification** pattern, which sidesteps a long-standing limitation of flat KV stores (no `PREFIX`/`LIKE` queries) by writing each object at multiple precisions at ingest time so point queries at any precision are a single-key read.

See [`docs/DIAGRAMS.md`](docs/DIAGRAMS.md) for architecture + data-flow diagrams and [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the API design.

---

## Quick start

Prerequisites: [Spin CLI](https://developer.fermyon.com/spin/install) ≥ 2.x, Rust (`rustup target add wasm32-wasip1`), Python 3 (for the load script).

```bash
git clone https://github.com/ccie7599/spin-geospatial-demo
cd spin-geospatial-demo

spin build                                  # compiles to wasm32-wasip1
spin up                                      # listens on :3000

# In another terminal:
./load-libraries.sh http://localhost:3000    # full 17,980 libraries (~15-30 min)
# or, for a quick look:
LIMIT=200 ./load-libraries.sh http://localhost:3000

./test-flow.sh http://localhost:3000         # 6 smoke tests
```

Open http://localhost:3000 for the interactive demo page.

Deploy to Akamai Functions:

```bash
spin aka deploy                              # or: spin deploy (Fermyon Cloud)
```

---

## API surface

| Method | Path | Purpose |
|--- |--- |--- |
| `POST` | `/ingest/store` | Paint a venue (library branch) + its internal sections |
| `POST` | `/ingest/object` | Paint any spatial object (generic) |
| `POST` | `/ingest/location` | S2S device check-in: context + enter/exit/dwell events |
| `GET`  | `/query/point/:lat/:lon[/:precision]` | Raw spatial query at a coordinate |
| `GET`  | `/query/area/:geohash` | All objects in a geohash cell |
| `GET`  | `/query/store/:id` | Fetch a stored object by id |
| `GET`  | `/api/v1/checkin` | Am-I-inside-a-venue + nearest-venue fallback |
| `GET`  | `/api/v1/stores/:id/find` | Search sections inside a venue |
| `GET`  | `/api/v1/stores/:id/position` | Floor-plan coordinate + current zone |
| `GET`  | `/api/v1/stores/:id/context` | Zone-aware content (hero + nearby sections) |
| `GET`  | `/health` | Version, storage, schema |

A note on naming: the code calls venues "stores" and internal zones "departments" — those names come from the original retail origin story and are deliberately generic. In the library dataset, a `store` is a branch and a `department` is a section (`children`, `reference`, `fiction`, etc.).

---

## Data model

Three Spin KV namespaces, all values JSON:

```
spatial:{precision}:{geohash}   → ["type:id|lat|lon", ...]      # spatial index
obj:{type}:{id}                 → full SpatialObject             # object store
dev:{deviceId}                  → DeviceState                    # per-device state
store-depts:{storeId}           → ["department:id", ...]         # dept lookup
```

### Cell painting at ingest

When a venue with a 35 m radius is ingested, the service computes **every geohash cell its circular footprint intersects** (at the natural precision for that radius — ~precision 8 for a 35 m radius). Internal sections at ~6 m radius land at precision 9. The object's id is appended to each cell's array.

### Upward write amplification

Spin KV has no prefix query. A venue painted at precision 8 is *also* written at precisions 7, 5, 4, and 3. That turns a coarse-resolution area query ("find libraries in this ~5 km neighborhood") into a direct single-key read. The trade is write cost for read simplicity — worth it because ingest is infrequent and queries are constant.

### Query path

A check-in at `(lat, lon)` encodes to a 12-character geohash, iterates over precision levels, reads `spatial:{p}:{prefix}` plus the relevant **neighbor cells** (4 at precision 9, 8 at precision ≤ 8, none at precision 10 because the paint algorithm covers overlap), deduplicates refs in memory using embedded lat/lon for a cheap pre-hydration radius filter, then hydrates the surviving object records.

Typical cost: **~15 KV reads** for a full check-in, most served from the Spin KV process cache on repeats. P50 function-internal latency: **2–4 ms** on Akamai Functions.

Full details and mermaid diagrams: [`docs/DIAGRAMS.md`](docs/DIAGRAMS.md).

---

## Project layout

```
src/
  lib.rs              Spin HTTP component: router + endpoint handlers
  geohash.rs          Pure geohash: encode/decode/neighbors/paint_cells/haversine
  spatial_store.rs    Spin KV spatial store: paint_object, query_point, events
  models.rs           serde request/response/internal types
docs/
  DIAGRAMS.md         System architecture + ingest + query mermaid diagrams
  ARCHITECTURE.md     API design + venue/section model
sample-data/
  us_libraries.csv    17,980 US public libraries (OpenStreetMap, ODbL)
static/
  index.html          Minimal Leaflet-based live demo page
load-libraries.sh     Ingests the CSV via /ingest/store
test-flow.sh          6 smoke tests
spin.toml             Spin manifest
Cargo.toml            Rust crate manifest
```

---

## Dataset

`sample-data/us_libraries.csv` is extracted from [OpenStreetMap](https://www.openstreetmap.org) (`amenity=library` within the US, nodes + way centroids) and is **© OpenStreetMap contributors, ODbL 1.0**. 17,980 rows after de-duplication and filtering of law/medical/university-internal libraries. The lat/lon columns reflect what OSM has; some outlying tags may be approximate or incomplete. For a production deployment, combine this with the [IMLS Public Libraries Survey](https://www.imls.gov/research-evaluation/surveys-data/public-libraries-survey) facilities file for authoritative locations + operating hours.

The demo loader also synthesizes a fixed set of internal sections per library (`children`, `fiction`, `nonfiction`, `reference`, `periodicals`, `audiovisual`, `computers`, `community`) as offsets relative to the library center. These are illustrative — OSM doesn't generally tag internal sections.

---

## License

MIT — see [`LICENSE`](LICENSE).

OpenStreetMap data (ODbL 1.0) is credited per section above.
