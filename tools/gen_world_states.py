#!/usr/bin/env python3
"""Writes data/states_world.bin: the states and provinces of the world but Germany (whose come
from BKG, see gen_energy.py), with outline, ISO 3166-2 code, name, country and population.

usage: gen_world_states.py [--from DIR] [--only FRA,AUT,...] data/states_world.bin

Without --from, the inputs are downloaded into a fresh temporary folder: Natural Earth's admin-1
states and provinces at 1:10m and its admin-0 countries at 1:50m (both public domain, from the
nvkelso/natural-earth-vector repository), and the population of each state's Wikidata item
(P1082, CC0) from the Wikidata query service. With --from, they are read from DIR under the
file names below. --only keeps the given countries, for test fixtures. Run with `python3 -I`.

Countries are keyed as in countries.bin (gen_owid.py): Natural Earth's ISO_A3, else ISO_A3_EH,
of the admin-0 feature whose ADM0_A3 the state names.

Population: Wikidata's figure for the state's item, the preferred one, else the latest by its
point in time; states on one item share it by area. Where Wikidata has none (or one above Natural
Earth's POP_EST of the whole country), the states without share what the others leave of POP_EST
by area, or, where they leave nothing, get the median density of the others times their area;
such states are marked estimated. Natural Earth's populated places were tried and left out: they
are cities only, and miss the countryside.

Integers are zigzag varints (as in gen_energy.py); strings are a u8 length and UTF-8.

states_world.bin: b"ORBS\\x01\\0\\0\\0", u16 steps per degree (SCALE), u16 number of
states, and for each: country code, ISO 3166-2 code (may be empty), name, kind (Natural Earth's
type_en, such as Province), u32 population, u8 flags (1: population estimated), label point
as varint latitude and longitude in steps, u16 number of rings, and for each ring a varint count of points (the
first not repeated at the end) and the points as varint steps in latitude and longitude
from the last point (the first from the label point). Rings are taken even-odd.
States are sorted by country, then ISO code, then name.
"""
import json
import math
import os
import statistics
import struct
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from energy_areas import inside, signed_area, simplify  # noqa: E402
from gen_energy import varint  # noqa: E402

NE = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/"
ADMIN1 = "ne_10m_admin_1_states_provinces.geojson"
ADMIN0 = "ne_50m_admin_0_countries.geojson"
WIKIDATA = "wikidata_population.json"
SPARQL = "https://query.wikidata.org/sparql"
# States whose outline Germany's own (BKG) replace.
SKIP = {"DEU"}
NE_CODES = {"KOS": "OWID_KOS"}
# Douglas-Peucker tolerance in degrees, and the smallest island kept apart from a state's
# largest ring, in square degrees.
TOLERANCE = 0.025
MIN_AREA = 0.005
# Positions are stored in steps of 1/SCALE degrees: 444 m, well below the tolerance.
SCALE = 250


def fetch(url, path, data=None):
    req = urllib.request.Request(url, data=data, headers={"User-Agent": "orbis gen_world_states.py (population of Natural Earth admin-1 items, one-off build)", "Accept": "application/sparql-results+json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        body = r.read()
    if path:
        open(path, "wb").write(body)
        print("fetched", url, len(body), "bytes")
    return body


def string(s):
    b = s.encode("utf-8")[:255]
    return bytes([len(b)]) + b


def ne_code(p):
    for key in ("ISO_A3", "ISO_A3_EH"):
        if p[key] != "-99":
            return p[key]
    return NE_CODES.get(p["ADM0_A3"])


def country_codes(path):
    out = {}
    for f in json.load(open(path, encoding="utf-8"))["features"]:
        p = f["properties"]
        code = ne_code(p)
        if code:
            out[p["ADM0_A3"]] = code
            out["pop:" + code] = out.get("pop:" + code, 0) + (p.get("POP_EST") or 0)
            out.setdefault("iso2:" + p["ISO_A2"], code)
    return out


def ask_wikidata(items):
    """{item: [population, rank, date]} for the items, in batches."""
    out = {}
    items = sorted(items)
    for k in range(0, len(items), 300):
        values = " ".join("wd:" + q for q in items[k : k + 300])
        query = (
            "SELECT ?item ?pop ?rank ?date WHERE { VALUES ?item { " + values + " } "
            "?item p:P1082 ?st. ?st ps:P1082 ?pop; wikibase:rank ?rank. "
            "OPTIONAL { ?st pq:P585 ?date } FILTER(?rank != wikibase:DeprecatedRank) }"
        )
        for attempt in range(6):
            try:
                body = fetch(SPARQL, None, urllib.parse.urlencode({"query": query}).encode())
                break
            except urllib.error.HTTPError as e:
                if e.code != 429 or attempt == 5:
                    raise
                wait = int(e.headers.get("Retry-After") or 30)
                print("wikidata asks to wait", wait, "s")
                time.sleep(wait)
        time.sleep(2)
        for b in json.loads(body)["results"]["bindings"]:
            q = b["item"]["value"].rsplit("/", 1)[1]
            row = [float(b["pop"]["value"]), b["rank"]["value"].endswith("PreferredRank"), b.get("date", {}).get("value", "")]
            out.setdefault(q, []).append(row)
        print("wikidata:", min(k + 300, len(items)), "of", len(items))
    return out


def best_population(rows):
    if not rows:
        return None
    rows = sorted(rows, key=lambda r: (r[1], r[2], r[0]))
    return rows[-1][0]


def area_km2(rings):
    total = 0.0
    for r in rings:
        lat = sum(p[1] for p in r) / len(r)
        total += signed_area(r) * 111.32**2 * max(0.01, math.cos(math.radians(lat)))
    return abs(total)


def main():
    args = sys.argv[1:]
    src, only = None, None
    while args and args[0].startswith("--"):
        flag, value, args = args[0], args[1], args[2:]
        if flag == "--from":
            src = value
        elif flag == "--only":
            only = set(value.split(","))
        else:
            sys.exit(__doc__)
    if len(args) != 1:
        sys.exit(__doc__)
    (target,) = args
    fresh = src is None
    if fresh:
        src = tempfile.mkdtemp(prefix="states-")
        print("inputs go to", src, "(give it to --from to build again from them)")
        fetch(NE + ADMIN1, os.path.join(src, ADMIN1))
        fetch(NE + ADMIN0, os.path.join(src, ADMIN0))
    codes = country_codes(os.path.join(src, ADMIN0))
    states = []
    for f in json.load(open(os.path.join(src, ADMIN1), encoding="utf-8"))["features"]:
        p = f["properties"]
        if p["adm0_a3"] in SKIP or f["geometry"] is None:
            continue
        code = codes.get(p["adm0_a3"]) or codes.get("iso2:" + (p["iso_a2"] or ""))
        if code is None or (only and code not in only):
            continue
        g = f["geometry"]
        polys = g["coordinates"] if g["type"] == "MultiPolygon" else [g["coordinates"]]
        raw = [[(lon, lat) for lon, lat in ring] for poly in polys for ring in poly]
        rings = [r for r in (simplify(r, TOLERANCE) for r in raw) if r]
        biggest = max((abs(signed_area(r)) for r in rings), default=0)
        rings = [r for r in rings if abs(signed_area(r)) >= min(MIN_AREA, biggest)]
        if not rings:
            # Too small for the tolerance: its largest ring at a finer one.
            big = max(raw, key=lambda r: abs(signed_area(r)))
            rings = [simplify(big, TOLERANCE / 20) or big]
        label = (p["longitude"], p["latitude"])
        if not inside(rings, *label):
            from energy_areas import label_point

            label = label_point(rings)
        iso = p["iso_3166_2"] or ""
        if iso.endswith("~") or iso.startswith("-"):
            iso = ""
        states.append({"code": code, "iso": iso, "name": p["name_en"] or p["name"] or "", "kind": p["type_en"] or "", "wikidata": p["wikidataid"] or "", "rings": rings, "label": label, "km2": area_km2(raw)})
    wd_path = os.path.join(src, WIKIDATA)
    if not os.path.exists(wd_path):
        json.dump(ask_wikidata({s["wikidata"] for s in states if s["wikidata"]}), open(wd_path, "w"))
    wd = json.load(open(wd_path))
    for s in states:
        s["pop"] = best_population(wd.get(s["wikidata"], []))
    # Several states on one item (a region Natural Earth splits) share its figure by area.
    sharing = {}
    for s in states:
        if s["wikidata"] and s["pop"]:
            sharing.setdefault(s["wikidata"], []).append(s)
    for group in sharing.values():
        if len(group) > 1:
            area = sum(s["km2"] for s in group) or 1.0
            for s in group:
                s["pop"] *= s["km2"] / area
    by_country = {}
    for s in states:
        by_country.setdefault(s["code"], []).append(s)
    estimated = 0
    for code, group in by_country.items():
        whole = codes.get("pop:" + code, 0)
        for s in group:
            # A state more populous than Natural Earth's whole country is a wrong item.
            if s["pop"] is not None and whole and s["pop"] > 1.05 * whole:
                s["pop"] = None
        known = [s for s in group if s["pop"]]
        missing = [s for s in group if not s["pop"]]
        if not missing:
            continue
        area = sum(s["km2"] for s in missing) or 1.0
        rest = whole - sum(s["pop"] for s in known)
        if rest > 0:
            # What the states with a figure leave of the country, spread by area.
            density = rest / area
        else:
            dens = [s["pop"] / s["km2"] for s in known if s["km2"] > 0]
            density = statistics.median(dens) if dens else 0.0
        for s in missing:
            s["pop"] = density * s["km2"]
            s["estimated"] = True
            estimated += 1
    states.sort(key=lambda s: (s["code"], s["iso"], s["name"]))
    out = bytearray(b"ORBS\x01\0\0\0") + struct.pack("<HH", SCALE, len(states))
    points = 0
    for s in states:
        lat0, lon0 = round(s["label"][1] * SCALE), round(s["label"][0] * SCALE)
        out += string(s["code"]) + string(s["iso"]) + string(s["name"]) + string(s["kind"])
        out += struct.pack("<IB", min(round(s["pop"]), 2**32 - 1), 1 if s.get("estimated") else 0)
        rings = []
        for r in s["rings"]:
            q = []
            for lon, lat in r[:-1]:
                p = (round(lat * SCALE), round(lon * SCALE))
                if not q or q[-1] != p:
                    q.append(p)
            while len(q) > 1 and q[0] == q[-1]:
                q.pop()
            if len(q) >= 3:
                rings.append(q)
        if not rings:
            # Smaller than a step: a triangle round its label point.
            rings = [[(lat0, lon0 - 1), (lat0 + 1, lon0), (lat0, lon0 + 1)]]
        out += varint(lat0) + varint(lon0) + struct.pack("<H", len(rings))
        for q in rings:
            out += varint(len(q))
            la, lo = lat0, lon0
            for a, o in q:
                out += varint(a - la) + varint(o - lo)
                la, lo = a, o
            points += len(q)
    open(target, "wb").write(out)
    print(len(states), "states in", len(by_country), "countries,", estimated, "populations estimated,", points, "points,", len(out), "bytes")


if __name__ == "__main__":
    main()
