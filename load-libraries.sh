#!/bin/bash
# Load US public libraries into the spatial index.
# Each library is ingested as a "store" with a set of "departments" that
# model a canonical public-library floor plan: the ten Dewey Decimal classes
# (000-900) plus the usual non-Dewey zones (children, fiction, reference,
# periodicals, audiovisual, community).
set -euo pipefail

BASE=${1:-http://localhost:3000}
CSV="sample-data/us_libraries.csv"
LIMIT=${LIMIT:-0}   # 0 = load all; set LIMIT=100 to load a subset
NDJSON=$(mktemp -t libraries.XXXXXX.ndjson)
trap 'rm -f "$NDJSON"' EXIT

if [ ! -f "$CSV" ]; then
  echo "ERROR: $CSV not found. Run from repo root." >&2
  exit 1
fi

echo "Loading libraries from $CSV into $BASE (LIMIT=$LIMIT)..."

python3 - "$CSV" "$LIMIT" > "$NDJSON" <<'PY'
import csv, json, math, sys

path, limit_s = sys.argv[1], sys.argv[2]
limit = int(limit_s) if limit_s.isdigit() else 0

# Canton-style library floor plan: 80m x 80m building centered on venue lat/lon.
# West half: adult stacks (10 Dewey rows + fiction rows) + reference desk.
# East half: children's library (large zone) + audiovisual + community room.
# South central: periodicals reading area.
# Each "section" = center coord + detection radius. Stack aisles are narrow;
# rooms/children's area are wider.  (x: west-east meters, y: south-north meters)
SECTIONS = [
    # Adult Non-Fiction — 10 Dewey shelf aisles, stacked along west side
    ("dewey-000",  -22.0,  26.0, 3.5),
    ("dewey-100",  -22.0,  22.0, 3.5),
    ("dewey-200",  -22.0,  18.0, 3.5),
    ("dewey-300",  -22.0,  14.0, 3.5),
    ("dewey-400",  -22.0,  10.0, 3.5),
    ("dewey-500",  -22.0,   6.0, 3.5),
    ("dewey-600",  -22.0,   2.0, 3.5),
    ("dewey-700",  -22.0,  -2.0, 3.5),
    ("dewey-800",  -22.0,  -6.0, 3.5),
    ("dewey-900",  -22.0, -10.0, 3.5),
    # Adult Fiction — multi-row zone, west central
    ("fiction",    -15.0, -17.0, 6.0),
    # Reference desk — SW
    ("reference",  -18.0, -25.0, 4.0),
    # Periodicals reading area — central south
    ("periodicals",  0.0, -25.0, 5.0),
    # Children's Library — large east zone
    ("children",    18.0,  12.0, 14.0),
    # Audiovisual — east mid (DVDs, music, video games)
    ("audiovisual", 22.0,  -5.0, 5.0),
    # Community Room — SE enclosed room
    ("community",   18.0, -24.0, 8.0),
]

M_PER_DEG_LAT = 111_320.0
def m_per_deg_lon(lat): return 111_320.0 * abs(math.cos(math.radians(lat)))

with open(path) as f:
    reader = csv.DictReader(f)
    count = 0
    for row in reader:
        if limit and count >= limit: break
        try:
            lat = float(row["latitude"]); lon = float(row["longitude"])
        except ValueError:
            continue
        mlon = m_per_deg_lon(lat) or 1.0
        depts = []
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
            "radius": 40,
            "address": row["full_address"],
            "departments": depts,
        }
        print(json.dumps(obj))
        count += 1
PY

TOTAL=$(wc -l < "$NDJSON")
echo "Prepared $TOTAL venue payloads. Ingesting..."

COUNT=0
while IFS= read -r payload; do
    curl -s -o /dev/null -X POST "$BASE/ingest/store" \
        -H 'Content-Type: application/json' -d "$payload"
    COUNT=$((COUNT+1))
    if [ $((COUNT % 100)) -eq 0 ]; then echo "  ...$COUNT / $TOTAL"; fi
done < "$NDJSON"

echo "Done. Ingested $COUNT venues."
