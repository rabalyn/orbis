#!/usr/bin/env python3
"""Builds data/energy_de.bin: renewable generating units in Germany from the Bundesnetzagentur's
Marktstammdatenregister (MaStR), summed over BKG's administrative areas (VG250).

usage: gen_energy.py Gesamtdatenexport_YYYYMMDD_*.zip vg250_ebenen_0101/ data/energy_de.bin

Both sources are under the Datenlizenz Deutschland - Namensnennung 2.0.

Units read: solar, wind (on land and at sea), hydro, biomass, and for comparison combustion
(lignite, hard coal, natural gas, oil, other: other gases, non-biogenic waste, heat, hydrogen)
and nuclear units, that are in operation,
temporarily shut down, or shut down for good with a date. Planned units are left out. Capacity
is the net rated power (Nettonennleistung, AC) in kW, the gross power where that is missing.
Months are counted from January 1900.

Each unit belongs to a municipality, by its municipality key, else by its position. A unit whose
published position lies in its municipality (within a margin) belongs to a site: units of one
source within about 11 m of each other. The others, mostly rooftop solar of private owners, which
the register publishes without a position, are summed per municipality. Most large power plants
are published with only a district key and no position; they are summed per district. Offshore wind turbines
belong to their wind park, and parks to their sea.

The file, little endian:
  b"ORBE\\x01\\0\\0\\0", u32 export date as YYYYMMDD
  u32 area count, then each area:
    u8 level (0 state, 1 district, 2 municipality), u8 kind (0 administrative, 1 sea, 2 park),
    u32 parent index or 0xFFFFFFFF, u8 name length, UTF-8 name,
    i32 label latitude and longitude in microdegrees,
    u16 ring count, then each ring: u32 point count, i32 first latitude and longitude in
    microdegrees, then zigzag varint steps of latitude and longitude in units of 1e-5 degrees
  u32 count of series of units without a position, each:
    u32 area, u8 source, u32 change count, then each change: varint months since the last
    change (the first since January 1900), varint change in tenths of a kW
  u32 site count, then each site, sorted by area and position:
    varint steps from the last site's area, latitude and longitude (in 1e-4 degrees), u8 source,
    varint unit count, then each unit: varint first month, varint months it ran until shut down
    for good plus one, or 0 while it runs, varint tenths of a kW
Varints are zigzag LEB128.
Sources: 0 solar, 1 wind on land, 2 wind at sea, 3 hydro, 4 biomass, 5 lignite, 6 hard coal,
7 natural gas, 8 oil, 9 other fossil, 10 nuclear. The register began in 2019: units shut down
before then are not in it.
"""
import math
import multiprocessing
import os
import re
import struct
import sys
import zipfile
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import energy_areas  # noqa: E402

SOLAR, WIND_ON, WIND_SEA, HYDRO, BIOMASS, LIGNITE, HARD_COAL, GAS, OIL, OTHER_FOSSIL, NUCLEAR = range(11)
FILES = {
    "EinheitenSolar": "EinheitSolar", "EinheitenWind": "EinheitWind", "EinheitenWasser": "EinheitWasser", "EinheitenBiomasse": "EinheitBiomasse",
    "EinheitenVerbrennung": "EinheitVerbrennung", "EinheitenKernkraft": "EinheitKernkraft",
}
# The register's codes for the fuel of combustion units (Energietraeger).
FUELS = {"2408": LIGNITE, "2407": HARD_COAL, "2410": GAS, "2409": OIL}
FIELDS = [
    "EinheitBetriebsstatus", "Inbetriebnahmedatum", "DatumEndgueltigeStilllegung", "Nettonennleistung", "Bruttoleistung",
    "Laengengrad", "Breitengrad", "Gemeindeschluessel", "WindAnLandOderAufSee", "NameWindpark", "Energietraeger",
]
TAG = re.compile(r"<(" + "|".join(FIELDS) + r")>([^<]*)</")
IN_OPERATION, PAUSED, RETIRED = "35", "37", "38"
OPEN = 0xFFFF
SEA_LAGE = "889"


def month(date):
    """Months from January 1900 for "YYYY-MM-DD", or None."""
    if len(date) < 7:
        return None
    try:
        y, m = int(date[:4]), int(date[5:7])
    except ValueError:
        return None
    return max(0, min(OPEN - 1, (y - 1900) * 12 + m - 1))


def read_member(job):
    """The units of one file in the export, as tuples."""
    zip_path, name, record = job
    out, skipped = [], defaultdict(int)
    open_tag, close_tag = f"<{record}>", f"</{record}>"
    with zipfile.ZipFile(zip_path) as z, z.open(name) as f:
        import codecs

        decoder = codecs.getincrementaldecoder("utf-16")()
        rest = ""
        while True:
            chunk = f.read(1 << 24)
            text = rest + decoder.decode(chunk, final=not chunk)
            end = text.rfind(close_tag)
            if end < 0:
                rest = text
                if not chunk:
                    break
                continue
            body, rest = text[: end + len(close_tag)], text[end + len(close_tag) :]
            for rec in body.split(open_tag)[1:]:
                fields = dict(TAG.findall(rec))
                unit = make_unit(name, fields, skipped)
                if unit:
                    out.append(unit)
            if not chunk:
                break
    return out, dict(skipped)


def make_unit(name, f, skipped):
    status = f.get("EinheitBetriebsstatus")
    if status not in (IN_OPERATION, PAUSED, RETIRED):
        skipped["planned or unknown status"] += 1
        return None
    start = month(f.get("Inbetriebnahmedatum", ""))
    if start is None:
        skipped["no start date"] += 1
        return None
    end = OPEN
    if status == RETIRED:
        end = month(f.get("DatumEndgueltigeStilllegung", ""))
        if end is None:
            skipped["shut down without a date"] += 1
            return None
    try:
        kw = float(f.get("Nettonennleistung") or f.get("Bruttoleistung") or "nan")
    except ValueError:
        kw = float("nan")
    if not (kw > 0) or kw > 5e6:
        skipped["no usable power"] += 1
        return None
    if name.startswith("EinheitenSolar"):
        source = SOLAR
    elif name.startswith("EinheitenWind"):
        source = WIND_SEA if f.get("WindAnLandOderAufSee") == SEA_LAGE else WIND_ON
    elif name.startswith("EinheitenWasser"):
        source = HYDRO
    elif name.startswith("EinheitenBiomasse"):
        source = BIOMASS
    elif name.startswith("EinheitenKernkraft"):
        source = NUCLEAR
    else:
        # Other gases, non-biogenic waste, heat and hydrogen are summed as other.
        source = FUELS.get(f.get("Energietraeger"), OTHER_FOSSIL)
    try:
        lat, lon = float(f["Breitengrad"]), float(f["Laengengrad"])
        if not (46.0 < lat < 56.0 and 4.0 < lon < 16.0):
            lat = lon = None
    except (KeyError, ValueError):
        lat = lon = None
    return (source, start, end, kw, lat, lon, f.get("Gemeindeschluessel", ""), f.get("NameWindpark", "").strip())


class TownIndex:
    """Municipalities found by position, through a grid of their bounding boxes."""

    CELL = 0.1

    def __init__(self, towns):
        self.cells = defaultdict(list)
        self.boxes = {}
        for t in towns:
            lats = [p[0] for r in t.rings for p in r]
            lons = [p[1] for r in t.rings for p in r]
            box = (min(lats), min(lons), max(lats), max(lons))
            self.boxes[t.key] = box
            for y in range(int(box[0] / self.CELL), int(box[2] / self.CELL) + 1):
                for x in range(int(box[1] / self.CELL), int(box[3] / self.CELL) + 1):
                    self.cells[(y, x)].append(t)

    def find(self, lat, lon):
        for t in self.cells.get((int(lat / self.CELL), int(lon / self.CELL)), []):
            b = self.boxes[t.key]
            if b[0] <= lat <= b[2] and b[1] <= lon <= b[3] and energy_areas.inside(t.rings, lat, lon):
                return t
        return None

    def near(self, town, lat, lon, margin=0.03):
        b = self.boxes[town.key]
        return b[0] - margin <= lat <= b[2] + margin and b[1] - margin <= lon <= b[3] + margin


def hull(points, buffer_m):
    """Convex hull, in degrees, of the points widened by about `buffer_m` metres."""
    lat0 = sum(p[0] for p in points) / len(points)
    k = math.cos(math.radians(lat0))
    d_lat = buffer_m / 111_320
    spread = []
    for lat, lon in points:
        for a in range(8):
            t = a * math.pi / 4
            spread.append(((lon + d_lat * math.cos(t) / k) * k, lat + d_lat * math.sin(t)))
    spread = sorted(set(spread))

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    lower, upper = [], []
    for p in spread:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    for p in reversed(spread):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    ring = [(y, x / k) for x, y in lower[:-1] + upper[:-1]]
    return ring + [ring[0]]


def sea_of(lat, lon):
    return "Nordsee" if lon < 9.0 else "Ostsee"


def varint(v):
    z = (v << 1) ^ (v >> 63)
    out = bytearray()
    while True:
        b = z & 0x7F
        z >>= 7
        if z:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def micro(v):
    return round(v * 1e6)


def write(path, date, areas, unplaced, sites):
    out = bytearray(b"ORBE\x01\0\0\0")
    out += struct.pack("<I", date)
    out += struct.pack("<I", len(areas))
    for a in areas:
        name = a.name.encode("utf-8")[:255]
        parent = a.parent.index if a.parent else 0xFFFFFFFF
        out += struct.pack("<BBIB", a.level, a.kind, parent, len(name)) + name
        out += struct.pack("<ii", micro(a.label[0]), micro(a.label[1]))
        out += struct.pack("<H", len(a.rings))
        for ring in a.rings:
            q = [(round(lat * 1e5), round(lon * 1e5)) for lat, lon in ring]
            out += struct.pack("<Iii", len(q), q[0][0] * 10, q[0][1] * 10)
            for (y0, x0), (y1, x1) in zip(q, q[1:]):
                out += varint(y1 - y0) + varint(x1 - x0)
    out += struct.pack("<I", len(unplaced))
    for (area, source), changes in sorted(unplaced.items()):
        steps = [(m, round(kw * 10)) for m, kw in sorted(changes.items())]
        steps = [st for st in steps if st[1] != 0]
        out += struct.pack("<IBI", area, source, len(steps))
        last = 0
        for m, tenths in steps:
            out += varint(m - last) + varint(tenths)
            last = m
    rows = []
    for (source, lat, lon), (area, units) in sites.items():
        for at in range(0, len(units), 60000):
            rows.append((area, round(lat * 1e4), round(lon * 1e4), source, sorted(units[at : at + 60000])))
    rows.sort()
    out += struct.pack("<I", len(rows))
    last = (0, 0, 0)
    for area, lat, lon, source, units in rows:
        out += varint(area - last[0]) + varint(lat - last[1]) + varint(lon - last[2]) + bytes([source]) + varint(len(units))
        last = (area, lat, lon)
        for start, end, kw in units:
            out += varint(start) + varint(0 if end == OPEN else end - start + 1) + varint(max(1, round(kw * 10)))
    open(path, "wb").write(out)
    return len(out)


def main():
    zip_path, vg250, target = sys.argv[1:4]
    date = int(re.search(r"(\d{8})", os.path.basename(zip_path)).group(1))
    states, districts, towns = energy_areas.load(vg250)
    print(f"{len(states)} states, {len(districts)} districts, {len(towns)} municipalities")
    with zipfile.ZipFile(zip_path) as z:
        jobs = [(zip_path, n, rec) for n in z.namelist() for prefix, rec in FILES.items() if re.fullmatch(prefix + r"(_\d+)?\.xml", n)]
    print(f"reading {len(jobs)} files")
    units, skipped = [], defaultdict(int)
    with multiprocessing.Pool(min(len(jobs), os.cpu_count() or 4)) as pool:
        for got, skip in pool.imap_unordered(read_member, jobs):
            units += got
            for k, v in skip.items():
                skipped[k] += v
    print(f"{len(units)} units; skipped: {dict(skipped)}")

    by_ags = {t.key: t for t in towns}
    by_district = {d.key: d for d in districts}
    index = TownIndex(towns)
    areas = states + districts + towns

    # Offshore parks and seas.
    parks = defaultdict(list)
    for u in units:
        if u[0] == WIND_SEA and u[4] is not None:
            parks[u[7] or "Unnamed wind park"].append(u)
    sea_areas = {}
    for sea in ["Nordsee", "Ostsee"]:
        points = [(u[4], u[5]) for p in parks.values() for u in p if sea_of(u[4], u[5]) == sea]
        if not points:
            continue
        top = energy_areas.Area(0, "S" + sea, sea, [hull(points, 6000)])
        mid = energy_areas.Area(1, "S" + sea, sea, top.rings, top)
        for a in (top, mid):
            a.kind = 1
            ring = a.rings[0]
            a.label = (sum(p[0] for p in ring[:-1]) / (len(ring) - 1), sum(p[1] for p in ring[:-1]) / (len(ring) - 1))
        sea_areas[sea] = mid
        areas += [top, mid]
    park_area = {}
    for name, members in sorted(parks.items()):
        points = [(u[4], u[5]) for u in members]
        sea = sea_areas[sea_of(*points[0])]
        a = energy_areas.Area(2, "P" + name, name, [hull(points, 800)], sea)
        a.kind = 2
        a.label = (sum(p[0] for p in points) / len(points), sum(p[1] for p in points) / len(points))
        park_area[name] = a
        areas.append(a)
    for a in areas:
        a.kind = getattr(a, "kind", 0)
    for i, a in enumerate(areas):
        a.index = i

    unplaced = defaultdict(lambda: defaultdict(float))
    sites = {}
    counts = defaultdict(int)
    for source, start, end, kw, lat, lon, ags, park in units:
        if source == WIND_SEA:
            area = park_area.get(park or "Unnamed wind park") if lat is not None else None
            if area is None:
                counts["offshore without position"] += 1
                continue
        else:
            area = by_ags.get(ags)
            if area is None and lat is not None:
                area = index.find(lat, lon)
            if area is None:
                # The largest plants are published with only a district key and no position.
                area = by_district.get(ags[:5]) if len(ags) >= 5 else None
                if area is not None:
                    counts["district only"] += 1
                    lat = None
            if area is None:
                counts["no municipality"] += 1
                continue
            if lat is not None and area.level == 2 and not index.near(area, lat, lon):
                counts["position outside its municipality"] += 1
                lat = None
        if lat is None:
            series = unplaced[(area.index, source)]
            series[start] += kw
            if end != OPEN:
                series[end] -= kw
            counts["without position"] += 1
        else:
            key = (source, round(lat, 4), round(lon, 4))
            entry = sites.setdefault(key, (area.index, []))
            entry[1].append((start, end, kw))
            counts["at a site"] += 1
    print(dict(counts), f"{len(sites)} sites")
    size = write(target, date, areas, unplaced, sites)
    print(f"{len(areas)} areas, {len(unplaced)} series, {size} bytes")
    total = defaultdict(float)
    for source, start, end, kw, *_ in units:
        if end == OPEN:
            total[source] += kw
    print("running now, GW:", {k: round(v / 1e6, 2) for k, v in sorted(total.items())})


if __name__ == "__main__":
    main()
