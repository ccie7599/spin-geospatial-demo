#!/bin/bash
# Load US public libraries into the spatial index.
# Each library is ingested as a "store" with a set of "departments" (library
# sections) — the data model uses generic names that work for any venue that
# has a physical footprint and internal zones.
set -euo pipefail

BASE=${1:-http://localhost:3000}
CSV="sample-data/us_libraries.csv"
LIMIT=${LIMIT:-0}   # 0 = load all; set LIMIT=100 to load a subset

if [ ! -f "$CSV" ]; then
  echo "ERROR: $CSV not found. Run from repo root." >&2
  exit 1
fi

echo "Loading libraries from $CSV into $BASE (LIMIT=$LIMIT)..."

python3 - "$CSV" "$LIMIT" <<'PY' | while IFS= read -r payload; do
    curl -s -o /dev/null -X POST "$BASE/ingest/store" \
        -H 'Content-Type: application/json' -d "$payload"
done
import csv, json, random, sys

path, limit_s = sys.argv[1], sys.argv[2]
limit = int(limit_s) if limit_s.isdigit() else 0

# Library sections — used as generic "departments" so the spatial index can
# resolve to a zone inside the venue during /api/v1/checkin calls.
# Each has a relative position (meters from library center) and radius.
SECTIONS = [
    ("children",    -15.0, -10.0, 8.0),
    ("fiction",       0.0, -10.0, 8.0),
    ("nonfiction",   15.0, -10.0, 8.0),
    ("reference",   -10.0,  10.0, 6.0),
    ("periodicals",  10.0,   5.0, 5.0),
    ("audiovisual",  15.0,  10.0, 6.0),
    ("computers",   -15.0,  10.0, 6.0),
    ("community",     0.0,  15.0, 7.0),
]

# Meters → degrees
M_PER_DEG_LAT = 111_320.0
def m_per_deg_lon(lat): return 111_320.0 * abs(__import__('math').cos(__import__('math').radians(lat)))

with open(path) as f:
    reader = csv.DictReader(f)
    count = 0
    for row in reader:
        if limit and count >= limit: break
        try:
            lat = float(row["latitude"]); lon = float(row["longitude"])
        except ValueError:
            continue
        depts = []
        mlon = m_per_deg_lon(lat) or 1.0
        for name, dx, dy, r in SECTIONS:
            depts.append({
                "name": name,
                "lat": lat + (dy / M_PER_DEG_LAT),
                "lon": lon + (dx / mlon),
                "radius": r,
            })
        obj = {
            "storeId": row["library_id"],
            "name": row["name"],
            "lat": lat, "lon": lon,
            "radius": 35,   # ~most public library footprints in meters
            "address": row["full_address"],
            "departments": depts,
        }
        print(json.dumps(obj))
        count += 1
PY

echo "Done."
