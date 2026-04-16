#!/bin/bash
# End-to-end smoke test for the library demo.
# Ingests a single test library + sections, then exercises all query/customer APIs.
set -euo pipefail

BASE=${1:-http://localhost:3000}
PASS=0
FAIL=0

pass() { echo "  ok: $1"; PASS=$((PASS+1)); }
fail() { echo "  FAIL: $1"; echo "       $2"; FAIL=$((FAIL+1)); }

hit() {
    local method=$1 path=$2 body=${3:-}
    if [ -n "$body" ]; then
        curl -s -X "$method" "$BASE$path" -H 'Content-Type: application/json' -d "$body"
    else
        curl -s -X "$method" "$BASE$path"
    fi
}

echo "[1/6] health"
resp=$(hit GET /health)
echo "$resp" | grep -q '"status": "ok"' && pass "service up" || fail "health" "$resp"

echo "[2/6] ingest library with sections"
resp=$(hit POST /ingest/store '{
  "storeId": "test-001",
  "name": "Test Branch Library",
  "lat": 40.7128, "lon": -74.0060, "radius": 35,
  "address": "1 Example St, New York, NY",
  "departments": [
    {"name": "children",    "lat": 40.71267, "lon": -74.00612, "radius": 8},
    {"name": "fiction",     "lat": 40.71272, "lon": -74.00600, "radius": 8},
    {"name": "reference",   "lat": 40.71282, "lon": -74.00610, "radius": 6},
    {"name": "computers",   "lat": 40.71282, "lon": -74.00590, "radius": 6}
  ]
}')
echo "$resp" | grep -q '"status": "ok"' && pass "ingest" || fail "ingest" "$resp"

echo "[3/6] point query at library center"
resp=$(hit GET "/query/point/40.7128/-74.0060")
echo "$resp" | grep -q '"store"' && pass "point query finds library" || fail "point query" "$resp"

echo "[4/6] checkin inside children's section"
resp=$(hit GET "/api/v1/checkin?lat=40.71267&lon=-74.00612&deviceId=test-device")
echo "$resp" | grep -q '"inStore": true' && pass "checkin detects in-library" || fail "checkin" "$resp"

echo "[5/6] wayfinding find 'ref'"
resp=$(hit GET "/api/v1/stores/test-001/find?q=ref")
echo "$resp" | grep -q '"name": "reference"' && pass "find returns reference section" || fail "find" "$resp"

echo "[6/6] context in computers zone"
resp=$(hit GET "/api/v1/stores/test-001/context?lat=40.71282&lon=-74.00590")
echo "$resp" | grep -q '"zone": "computers"' && \
  echo "$resp" | grep -q 'Digital resources' && pass "context returns reading list for zone" \
  || fail "context" "$resp"

echo
echo "Results: $PASS passed, $FAIL failed"
[ $FAIL -eq 0 ]
