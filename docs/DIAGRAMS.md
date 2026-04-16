# Architecture & Data Flow

Three diagrams:

1. **System architecture** — where the WASM function sits on the Akamai Connected Cloud stack (or any Spin host).
2. **Ingest / cell painting** — how a library and its sections enter the spatial index.
3. **Query / location lookup** — how a visitor check-in resolves to a branch + section + entry/exit event.

All state lives in **Spin KV**. There are no external databases, scans, or secondary indexes.

---

## 1. System architecture

```mermaid
flowchart LR
    Client["Mobile / web client"]

    subgraph HOST ["Spin host (Akamai Functions / Fermyon Cloud / self-hosted)"]
      direction TB
      Ingress["HTTPS ingress<br/>(CDN, WAF, DDoS — when fronted by Akamai)"]
      subgraph FUNC ["Spin runtime"]
        direction TB
        WASM["geospatial.wasm<br/>(Rust / wasm32-wasip1, ~440 KB)"]
        KV[("Spin KV<br/>default store")]
      end
      Logs["Structured access logs<br/>(e.g. Akamai DataStream 2)"]
    end

    Mgmt["Venue management<br/>(POST /ingest/store)"]
    Obs["Observability sink<br/>(ClickHouse, ELK, etc.)"]

    Client -- "HTTPS" --> Ingress
    Ingress --> WASM
    Mgmt -- "ingest traffic" --> Ingress
    WASM <--> KV
    WASM -. "async egress" .-> Logs
    Logs -- "webhook" --> Obs
```

**Notes.** The WASM binary is deterministic and stateless — it can cold-start per-request in under a millisecond on Akamai Functions. State is in Spin KV, which is edge-local by default; replication across regions is a function of the host's KV implementation.

---

## 2. Ingest — cell painting + upward write amplification

```mermaid
flowchart TB
    In["POST /ingest/store<br/>{ storeId, name, lat, lon, radius:35m, departments[] }"]

    subgraph PAINT ["paint_object (per library + per section)"]
      direction TB
      Pick["precision_for_radius(radius, lat)<br/>→ 35m branch ≈ p8 (~38m cell)<br/>→ 6m section ≈ p9 (~4.8m cell)"]
      Sweep["paint_cells(lat, lon, radius, p)<br/>• bounding-box sweep at ½ cell step<br/>• haversine filter &lt; radius × 1.2<br/>• encode(lat, lon, p) → cell set"]
      Up["upward amplification<br/>• p8 → also write at p7, p5, p4, p3<br/>• p9 → also write at p8<br/>(direct reads at coarse levels)"]
    end

    subgraph KVKEYS ["Spin KV key schema"]
      direction TB
      SK["spatial:{p}:{geohash}<br/>→ JSON array of &quot;type:id|lat|lon&quot; refs"]
      OK["obj:{type}:{id}<br/>→ full SpatialObject JSON"]
      DK["store-depts:{storeId}<br/>→ JSON array of section refs"]
    end

    In --> Pick --> Sweep --> Up
    Up --> SK
    In --> OK
    In --> DK
```

**Why embed lat/lon in the ref.** `spatial:8:dr5ru6b` might hold `["store:nypl-mid|40.753182|-73.982253", ...]`. On the query path, we can run a haversine radius filter *before* fetching `obj:store:nypl-mid`, avoiding wasted KV reads when a dense geohash contains objects that are technically in-cell but far outside the caller's radius.

**Why upward amplification.** Spin KV is a flat key-value store — there's no `PREFIX` or `LIKE`. Writing a p8 library at p7/p5/p4/p3 converts a coarse-precision area query ("libraries in this neighborhood") into a direct single-key read.

---

## 3. Query — location lookup, dedup, event detection

```mermaid
flowchart TB
    In["GET /api/v1/checkin?lat=40.7531&lon=-73.9822&deviceId=phone-001"]
    Enc["encode(lat, lon, 12)<br/>→ 12-char geohash"]

    subgraph LOOP ["For p in min_precision..=max_precision"]
      direction TB
      Center["kv.get spatial:{p}:{geohash[0..p]}"]
      Nbrs["neighbors(cell)<br/>• p10: no neighbors (painting covers overlap)<br/>• p9: N/E/S/W only (4 reads)<br/>• p≤8: all 8 directions"]
      ReadN["kv.get each neighbor key"]
    end

    subgraph DEDUP ["In-memory aggregate"]
      direction TB
      RefMap["HashMap&lt;ref_name, (lat, lon)&gt;<br/>deduplicated across cells + precisions"]
      PreFilt["pre-hydration filter<br/>haversine(caller, ref_lat/lon) ≤ radius_m"]
    end

    Hyd["kv.get obj:{type}:{id}<br/>for each surviving ref"]
    PostFilt["post-hydration radius filter<br/>(catches legacy refs without embedded coords)"]
    Ann["annotate distance_mi<br/>meters / 1609.344"]

    subgraph EVENTS ["detect_events"]
      direction TB
      Prev["kv.get dev:{deviceId}<br/>→ previous DeviceState"]
      Logic["• was_in_venue × is_in_venue<br/>→ enter / still / exit+hysteresis<br/>• section change → entered_section<br/>• 3-ping consecutive_outside → confirmed exit + dwell_seconds"]
      Write["kv.set dev:{deviceId} → new state"]
    end

    Resp["JSON response:<br/>inStore, store, zone, event,<br/>nearestStore, nearby[]"]

    In --> Enc --> LOOP
    LOOP --> RefMap
    RefMap --> PreFilt --> Hyd --> PostFilt --> Ann --> Resp
    Ann --> EVENTS
    EVENTS --> Resp
```

**Typical cost.** A check-in at p9 with 4 neighbors + upward amplification reads at p8/p7 works out to ~15 KV reads per call — most served from the Spin KV process cache on repeat hits. P50 function-internal latency: **2–4 ms** on Akamai Functions.

**Event hysteresis.** A single "I don't see a venue" reading doesn't trigger an exit — `consecutive_outside` must reach 3 (typically ~30–60 seconds of pings) before we emit `device.exited_store`. This suppresses spurious exits from GPS jitter at the edge of a geofence.
