"""Builds data/missions.bin: the tracks of rovers, a helicopter and astronauts on the Moon and
Mars, each vertex with the moment it was passed.

usage: gen_missions.py DOWNLOADS data/missions.bin

DOWNLOADS holds the published files under these names (see docs/PROVENANCE.md for where each
comes from):

    M20_traverse.json, m20_heli_flight_path.json      Perseverance and Ingenuity (MMGIS)
    MSL_traverse.json, MSL_waypoints.json             Curiosity (MMGIS)
    MERA_traverse.csv, MERB_traverse_5130.csv         Spirit and Opportunity (PDS Geosciences)
    apollo12/, apollo14/                              EVA paths and timed points (LROC RDR)

Needs only the standard library.

Times
-----
Perseverance's drives and Ingenuity's flights carry spacecraft clock readings at their start and
end; the vertices in between are spread over that span by distance. Curiosity, Spirit and
Opportunity are known only by the sol of each drive: their drives are placed from 11:00 local
mean solar time on that sol, one after the other, at an assumed 0.02 m/s. Apollo walks carry
ground elapsed times at mapped points; the path between two points is spread over that span.

Format (little endian)
----------------------
    b"ORBM", u32 version (1), u32 number of tracks, then for each track:
    u8 body id length, body id; u8 name length, name (UTF-8); u8 kind (0 rover, 1 helicopter,
    2 astronaut); u8 red, green, blue; u8 flags (1: stays in view after its last piece);
    f64 Unix time of the mission's sol 0 at local midnight (NaN where sols do not apply);
    u32 number of pieces, then each piece: u32 number of vertices, then each vertex
    f64 Unix time, i32 latitude and i32 longitude in units of 1e-7 degree.
"""
import calendar
import csv
import json
import math
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

MARS_R = 3_396_190.0
MOON_R = 1_737_400.0
SOL = 88_775.244

# ---------------------------------------------------------------------------------------------
# Time


def unix(y, mo, d, h=0, mi=0, s=0.0):
    return calendar.timegm((y, mo, d, h, mi, 0, 0, 0, 0)) + s


def tt_minus_utc(t):
    """Seconds, from the leap second table; the years before 1972 are not needed for Mars."""
    steps = [
        (unix(1999, 1, 1), 64.184), (unix(2006, 1, 1), 65.184), (unix(2009, 1, 1), 66.184),
        (unix(2012, 7, 1), 67.184), (unix(2015, 7, 1), 68.184), (unix(2017, 1, 1), 69.184),
    ]
    out = 64.184
    for at, v in steps:
        if t >= at:
            out = v
    return out


def msd(t):
    """Mars Sol Date (Allison and McEwen 2000, as in Mars24)."""
    jd_tt = t / 86400.0 + 2440587.5 + tt_minus_utc(t) / 86400.0
    return (jd_tt - 2405522.0028779) / 1.0274912517


def unix_of_msd(m):
    jd_tt = m * 1.0274912517 + 2405522.0028779
    t = (jd_tt - 2440587.5) * 86400.0
    return t - tt_minus_utc(t)


def sol_zero(landing, landing_sol, lon_e):
    """The Unix time of local mean midnight that starts sol 0 at a landing site."""
    local = msd(landing) + lon_e / 360.0
    first = math.floor(local) - landing_sol
    return unix_of_msd(first - lon_e / 360.0)


def sclk(s):
    """MSL and Mars 2020 spacecraft clock seconds; clock drift of a few seconds is ignored."""
    return unix(2000, 1, 1, 11, 58, 55.816) + s


# ---------------------------------------------------------------------------------------------
# Geometry


def dist(a, b, radius):
    lat = math.radians((a[0] + b[0]) / 2)
    dx = math.radians(b[1] - a[1]) * math.cos(lat) * radius
    dy = math.radians(b[0] - a[0]) * radius
    return math.hypot(dx, dy)


def simplify(points, tol, radius):
    """Douglas-Peucker over (time, lat, lon), in local metres; keeps the first and last."""
    if len(points) < 3:
        return points
    lat0 = math.radians(points[0][1])
    xy = [(math.radians(p[2]) * math.cos(lat0) * radius, math.radians(p[1]) * radius) for p in points]
    keep = [False] * len(points)
    keep[0] = keep[-1] = True
    stack = [(0, len(points) - 1)]
    while stack:
        i, j = stack.pop()
        if j <= i + 1:
            continue
        (x0, y0), (x1, y1) = xy[i], xy[j]
        dx, dy = x1 - x0, y1 - y0
        n = math.hypot(dx, dy)
        best, at = -1.0, -1
        for k in range(i + 1, j):
            x, y = xy[k]
            d = abs(dy * (x - x0) - dx * (y - y0)) / n if n > 0 else math.hypot(x - x0, y - y0)
            if d > best:
                best, at = d, k
        if best > tol:
            keep[at] = True
            stack.append((i, at))
            stack.append((at, j))
    return [p for p, k in zip(points, keep) if k]


def spread(coords, t0, t1, radius):
    """Vertices (time, lat, lon), passed at times spread over [t0, t1] by distance."""
    total = sum(dist(coords[i], coords[i + 1], radius) for i in range(len(coords) - 1))
    out, run = [], 0.0
    for i, c in enumerate(coords):
        if i:
            run += dist(coords[i - 1], c, radius)
        f = run / total if total > 0 else i / max(1, len(coords) - 1)
        out.append((t0 + (t1 - t0) * f, c[0], c[1]))
    return out


def dedupe(points):
    out = []
    for p in points:
        if out and p[1] == out[-1][1] and p[2] == out[-1][2]:
            continue
        if out and p[0] < out[-1][0]:
            p = (out[-1][0], p[1], p[2])
        out.append(p)
    return out


# ---------------------------------------------------------------------------------------------
# Missions


PERSEVERANCE_LANDING = unix(2021, 2, 18, 20, 55)


def perseverance(d):
    feats = json.load(open(os.path.join(d, "M20_traverse.json")))["features"]
    feats.sort(key=lambda f: f["properties"]["SCLK_START"])
    pts = []
    for f in feats:
        p = f["properties"]
        g = f["geometry"]
        lines = g["coordinates"] if g["type"] == "MultiLineString" else [g["coordinates"]]
        coords = [(c[1], c[0]) for line in lines for c in line]
        pts += spread(coords, sclk(p["SCLK_START"]), sclk(p["SCLK_END"]), MARS_R)
    pts.insert(0, (PERSEVERANCE_LANDING, pts[0][1], pts[0][2]))
    return [simplify(dedupe(pts), 0.25, MARS_R)]


def ingenuity(d):
    feats = json.load(open(os.path.join(d, "m20_heli_flight_path.json")))["features"]
    feats.sort(key=lambda f: f["properties"]["SCLK_START"])
    pieces = []
    for f in feats:
        p = f["properties"]
        coords = [(c[1], c[0]) for c in f["geometry"]["coordinates"]]
        pieces.append(simplify(dedupe(spread(coords, sclk(p["SCLK_START"]), sclk(p["SCLK_END"]), MARS_R)), 0.25, MARS_R))
    # Between flights it rests where it landed: one piece from the first flight to the last.
    return [[v for piece in pieces for v in piece]]


class SolDrives:
    """Places drives known only by their sol: from 11:00 local time, in order, at 0.02 m/s."""

    def __init__(self, zero):
        self.zero = zero
        self.sol = None
        self.clock = 0.0

    def drive(self, sol, coords, radius):
        if sol != self.sol or self.clock < self.zero + sol * SOL:
            self.sol = sol
            self.clock = self.zero + (sol + 11 / 24) * SOL
        length = sum(dist(coords[i], coords[i + 1], radius) for i in range(len(coords) - 1))
        t0 = self.clock
        t1 = t0 + min(max(length / 0.02, 60.0), 5 * 3600.0)
        self.clock = t1
        return spread(coords, t0, t1, radius)


CURIOSITY_LANDING = unix(2012, 8, 6, 5, 17, 57)
CURIOSITY_ZERO = sol_zero(CURIOSITY_LANDING, 0, 137.4417)


def curiosity(d):
    waypoints = json.load(open(os.path.join(d, "MSL_waypoints.json")))["features"]
    sol_at = {}
    for w in waypoints:
        c = w["geometry"]["coordinates"]
        sol_at.setdefault((round(c[0], 6), round(c[1], 6)), w["properties"]["sol"])
    feats = json.load(open(os.path.join(d, "MSL_traverse.json")))["features"]
    drives = SolDrives(CURIOSITY_ZERO)
    pts, last_sol, missed, n = [], 0, 0, 0
    for f in feats:
        g = f["geometry"]
        # The early drives are merged into one feature; each of its parts is a drive.
        lines = g["coordinates"] if g["type"] == "MultiLineString" else [g["coordinates"]]
        for line in lines:
            if len(line) < 2:
                continue
            n += 1
            coords = [(c[1], c[0]) for c in line]
            end = line[-1]
            sol = sol_at.get((round(end[0], 6), round(end[1], 6)))
            if sol is None:
                best = min(waypoints, key=lambda w: (w["geometry"]["coordinates"][0] - end[0]) ** 2 + (w["geometry"]["coordinates"][1] - end[1]) ** 2)
                sol = best["properties"]["sol"]
                missed += 1
            sol = max(sol, last_sol)
            last_sol = sol
            pts += drives.drive(sol, coords, MARS_R)
    landing = (CURIOSITY_LANDING, pts[0][1], pts[0][2])
    pts.insert(0, landing)
    print(f"curiosity: {n} drives, {missed} matched to the nearest waypoint", file=sys.stderr)
    return [simplify(dedupe(pts), 0.25, MARS_R)]


def mer(path, to_lat_lon, zero, landing):
    rows = list(csv.DictReader(open(path)))
    drives = SolDrives(zero)
    pts = []
    prev = None
    for r in rows:
        here = to_lat_lon(float(r["CORRECTED_X"]), float(r["CORRECTED_Y"]))
        sol = int(r["START_SOL"])
        if prev is None:
            pts.append((landing, here[0], here[1]))
        elif here != prev:
            pts += drives.drive(sol, [prev, here], MARS_R)[1:]
        prev = here
    return [simplify(dedupe(pts), 0.25, MARS_R)]


def spirit(d):
    # Mars equirectangular projection centred on 175.5 E, 0 N (MERA_traverse.lbl).
    def ll(x, y):
        return (math.degrees(y / MARS_R), 175.5 + math.degrees(x / MARS_R))

    landing = unix(2004, 1, 4, 4, 35)
    return mer(os.path.join(d, "MERA_traverse.csv"), ll, sol_zero(landing, 1, 175.4785), landing)


# Opportunity's corrected positions are metres east and north of its landing site.
OPPORTUNITY_SITE = (-1.9462, -5.5266)


def opportunity(d):
    lat0, lon0 = OPPORTUNITY_SITE

    def ll(x, y):
        return (lat0 + math.degrees(y / MARS_R), lon0 + math.degrees(x / (MARS_R * math.cos(math.radians(lat0)))))

    landing = unix(2004, 1, 25, 5, 5)
    return mer(os.path.join(d, "MERB_traverse_5130.csv"), ll, sol_zero(landing, 1, 354.4734), landing)


# --- Apollo -----------------------------------------------------------------------------------

def read_dbf(path):
    f = open(path, "rb").read()
    n, hl, rl = struct.unpack("<xxxxIHH", f[:12])
    fields, p = [], 32
    while f[p] != 0x0D:
        fields.append((f[p:p + 11].split(b"\0")[0].decode(), f[p + 16]))
        p += 32
    rows = []
    for i in range(n):
        r = f[hl + i * rl:hl + (i + 1) * rl]
        if r[:1] == b"*":
            continue
        o, d = 1, {}
        for name, ln in fields:
            d[name] = r[o:o + ln].decode("utf-8", "replace").strip()
            o += ln
        rows.append(d)
    return rows


def read_shp(path):
    """Points as (x, y) or polylines as lists of parts, in file order."""
    f = open(path, "rb").read()
    out, p = [], 100
    while p < len(f):
        _, length = struct.unpack(">ii", f[p:p + 8])
        rec = f[p + 8:p + 8 + length * 2]
        p += 8 + length * 2
        kind = struct.unpack("<i", rec[:4])[0]
        if kind in (1, 11, 21):
            out.append(struct.unpack("<dd", rec[4:20]))
        elif kind in (3, 13, 23):
            nparts, npts = struct.unpack("<ii", rec[36:44])
            parts = list(struct.unpack(f"<{nparts}i", rec[44:44 + 4 * nparts])) + [npts]
            xy = struct.unpack(f"<{2 * npts}d", rec[44 + 4 * nparts:44 + 4 * nparts + 16 * npts])
            pts = [(xy[2 * i], xy[2 * i + 1]) for i in range(npts)]
            out.append([pts[parts[k]:parts[k + 1]] for k in range(nparts)])
        else:
            out.append(None)
    return out


def moon_ll(x, y):
    # Equirectangular Moon, 180 E and standard parallel 3 S (the files' .PRJ).
    lon = 180.0 + math.degrees(x / (MOON_R * math.cos(math.radians(-3.0))))
    if lon > 180:
        lon -= 360
    return (math.degrees(y / MOON_R), lon)


def get_seconds(s):
    h, m, sec = s.split(":")
    return int(h) * 3600 + int(m) * 60 + float(sec)


def eva(folder, stem, launch):
    """One astronaut's walk: the mapped paths, each passed between its timed end points."""
    pts_rows = read_dbf(os.path.join(folder, stem + "_PTS.DBF"))
    pts_xy = read_shp(os.path.join(folder, stem + "_PTS.SHP"))
    times = {}
    order = []
    for row, xy in zip(pts_rows, pts_xy):
        if not row.get("G_E_Time"):
            continue
        i = int(row["ID"])
        times[i] = launch + get_seconds(row["G_E_Time"])
        order.append((i, xy))
    ids = sorted(times)
    paths = {}
    path_rows = read_dbf(os.path.join(folder, stem + "_PATHS.DBF"))
    for row, parts in zip(path_rows, read_shp(os.path.join(folder, stem + "_PATHS.SHP"))):
        if parts:
            paths[int(row["ID"])] = [moon_ll(*p) for part in parts for p in part]
    xy_of = dict(order)
    out = []
    for a, b in zip(ids, ids[1:]):
        t0, t1 = times[a], max(times[b], times[a])
        coords = paths.get(a) or [moon_ll(*xy_of[a]), moon_ll(*xy_of[b])]
        out += spread(coords, t0, t1, MOON_R)
    return simplify(dedupe(out), 0.05, MOON_R)


def apollo(d, mission, launch, crew):
    folder = os.path.join(d, f"apollo{mission}")
    tracks = []
    for role, name in crew:
        pieces = [eva(folder, f"APOLLO_{mission}-{n}_{role}", launch) for n in (1, 2)]
        tracks.append((f"Apollo {mission}: {name}", pieces))
    return tracks


# ---------------------------------------------------------------------------------------------


def write(out_path, tracks):
    buf = bytearray(b"ORBM")
    buf += struct.pack("<II", 1, len(tracks))
    for body, name, kind, colour, flags, zero, pieces in tracks:
        b, n = body.encode(), name.encode()
        buf += struct.pack("<B", len(b)) + b + struct.pack("<B", len(n)) + n
        buf += struct.pack("<BBBBBd", kind, *colour, flags, zero)
        buf += struct.pack("<I", len(pieces))
        for piece in pieces:
            buf += struct.pack("<I", len(piece))
            for t, lat, lon in piece:
                buf += struct.pack("<dii", t, round(lat * 1e7), round(lon * 1e7))
    open(out_path, "wb").write(buf)
    print(f"{out_path}: {len(tracks)} tracks, {len(buf)} bytes", file=sys.stderr)


def main():
    d, out = sys.argv[1], sys.argv[2]
    nan = float("nan")
    m20_zero = sol_zero(PERSEVERANCE_LANDING, 0, 77.4509)
    tracks = [
        ("mars", "Perseverance", 0, (80, 200, 255), 1, m20_zero, perseverance(d)),
        ("mars", "Ingenuity", 1, (255, 214, 80), 1, m20_zero, ingenuity(d)),
        ("mars", "Curiosity", 0, (255, 150, 60), 1, CURIOSITY_ZERO, curiosity(d)),
        ("mars", "Spirit", 0, (130, 230, 140), 1, sol_zero(unix(2004, 1, 4, 4, 35), 1, 175.4785), spirit(d)),
        ("mars", "Opportunity", 0, (240, 120, 220), 1, sol_zero(unix(2004, 1, 25, 5, 5), 1, 354.4734), opportunity(d)),
    ]
    colours = [(255, 235, 120), (150, 210, 255)]
    for mission, launch, crew in [
        (12, unix(1969, 11, 14, 16, 22, 0), [("CDR", "Conrad"), ("LMP", "Bean")]),
        (14, unix(1971, 1, 31, 21, 3, 2), [("CDR", "Shepard"), ("LMP", "Mitchell")]),
    ]:
        for (name, pieces), colour in zip(apollo(d, mission, launch, crew), colours):
            tracks.append(("moon", name, 2, colour, 0, nan, pieces))
    for t in tracks:
        n = sum(len(p) for p in t[6])
        first, last = t[6][0][0][0], t[6][-1][-1][0]
        import time
        fmt = lambda s: time.strftime("%Y-%m-%d %H:%M", time.gmtime(s))
        print(f"  {t[1]}: {n} vertices, {fmt(first)} to {fmt(last)}", file=sys.stderr)
    write(out, tracks)


if __name__ == "__main__":
    main()
