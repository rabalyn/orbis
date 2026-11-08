"""Areas for gen_energy.py: German states, districts and municipalities from BKG's VG250
(shapefiles in ETRS89 / UTM zone 32N), turned into simplified outlines in degrees, each with a
point inside it where its bar stands.

Needs only the standard library: a small shapefile and dBASE reader, the inverse transverse
Mercator projection (Krüger series) and Douglas-Peucker simplification.
"""
import math
import struct

# GRS80, which ETRS89 uses; within a metre of WGS84 for this purpose.
A = 6378137.0
F = 1 / 298.257222101
K0 = 0.9996
E0 = 500000.0
LON0 = math.radians(9.0)

_n = F / (2 - F)
_A1 = A / (1 + _n) * (1 + _n**2 / 4 + _n**4 / 64)
_beta = [
    _n / 2 - 2 * _n**2 / 3 + 37 * _n**3 / 96,
    _n**2 / 48 + _n**3 / 15,
    17 * _n**3 / 480,
]
_delta = [
    2 * _n - 2 * _n**2 / 3 - 2 * _n**3,
    7 * _n**2 / 3 - 8 * _n**3 / 5,
    56 * _n**3 / 15,
]


def utm_to_deg(e, n):
    """Latitude and longitude in degrees for UTM 32N easting and northing in metres."""
    xi = n / (K0 * _A1)
    eta = (e - E0) / (K0 * _A1)
    xi_p, eta_p = xi, eta
    for j, b in enumerate(_beta, 1):
        xi_p -= b * math.sin(2 * j * xi) * math.cosh(2 * j * eta)
        eta_p -= b * math.cos(2 * j * xi) * math.sinh(2 * j * eta)
    chi = math.asin(math.sin(xi_p) / math.cosh(eta_p))
    lat = chi
    for j, d in enumerate(_delta, 1):
        lat += d * math.sin(2 * j * chi)
    lon = LON0 + math.atan2(math.sinh(eta_p), math.cos(xi_p))
    return math.degrees(lat), math.degrees(lon)


def read_dbf(path):
    data = open(path, "rb").read()
    count, header_len, record_len = struct.unpack("<4xIHH", data[:12])
    fields, i = [], 32
    while data[i] != 0x0D:
        name = data[i : i + 11].split(b"\0")[0].decode("ascii")
        fields.append((name, data[i + 16]))
        i += 32
    rows = []
    for r in range(count):
        at = header_len + r * record_len + 1  # after the deletion flag
        row = {}
        for name, length in fields:
            row[name] = data[at : at + length].decode("utf-8", "replace").strip()
            at += length
        rows.append(row)
    return rows


def read_shp(path):
    """Polygons as lists of rings, each a list of (easting, northing)."""
    data = open(path, "rb").read()
    at, shapes = 100, []
    while at < len(data):
        _, length = struct.unpack(">II", data[at : at + 8])
        body = data[at + 8 : at + 8 + length * 2]
        at += 8 + length * 2
        kind = struct.unpack("<i", body[:4])[0]
        if kind == 0:
            shapes.append([])
            continue
        assert kind == 5, f"shape type {kind}"
        parts, points = struct.unpack("<ii", body[36:44])
        starts = list(struct.unpack(f"<{parts}i", body[44 : 44 + 4 * parts])) + [points]
        xy = struct.unpack(f"<{2 * points}d", body[44 + 4 * parts : 44 + 4 * parts + 16 * points])
        rings = []
        for a, b in zip(starts, starts[1:]):
            rings.append([(xy[2 * k], xy[2 * k + 1]) for k in range(a, b)])
        shapes.append(rings)
    return shapes


def signed_area(ring):
    s = 0.0
    for (x0, y0), (x1, y1) in zip(ring, ring[1:] + ring[:1]):
        s += x0 * y1 - x1 * y0
    return s / 2


def simplify(ring, tolerance):
    """Douglas-Peucker on a closed ring; keeps at least a triangle."""
    if len(ring) < 5:
        return ring
    pts = ring[:-1] if ring[0] == ring[-1] else ring[:]
    # Split at the point farthest from the first, so both halves are open polylines.
    far = max(range(len(pts)), key=lambda k: (pts[k][0] - pts[0][0]) ** 2 + (pts[k][1] - pts[0][1]) ** 2)
    out = _dp(pts[: far + 1], tolerance)[:-1] + _dp(pts[far:] + [pts[0]], tolerance)[:-1]
    if len(out) < 3:
        return []
    return out + [out[0]]


def _dp(points, tol):
    keep = [False] * len(points)
    keep[0] = keep[-1] = True
    stack = [(0, len(points) - 1)]
    while stack:
        a, b = stack.pop()
        (ax, ay), (bx, by) = points[a], points[b]
        dx, dy = bx - ax, by - ay
        norm = math.hypot(dx, dy)
        best, best_d = None, tol
        for k in range(a + 1, b):
            px, py = points[k]
            d = abs(dy * (px - ax) - dx * (py - ay)) / norm if norm else math.hypot(px - ax, py - ay)
            if d > best_d:
                best, best_d = k, d
        if best is not None:
            keep[best] = True
            stack += [(a, best), (best, b)]
    return [p for p, k in zip(points, keep) if k]


def inside(rings, x, y):
    """Even-odd test over all rings, so holes count."""
    hit = False
    for ring in rings:
        for (x0, y0), (x1, y1) in zip(ring, ring[1:]):
            if (y0 > y) != (y1 > y) and x < x0 + (y - y0) * (x1 - x0) / (y1 - y0):
                hit = not hit
    return hit


def label_point(rings):
    """A point well inside the largest part: the middle of the widest horizontal run through
    it, tried on a few rows around its centroid."""
    outer = max(rings, key=lambda r: abs(signed_area(r)))
    ys = [p[1] for p in outer]
    y_lo, y_hi = min(ys), max(ys)
    best, best_w = None, -1.0
    for t in [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8]:
        y = y_lo + (y_hi - y_lo) * t
        xs = []
        for ring in rings:
            for (x0, y0), (x1, y1) in zip(ring, ring[1:]):
                if (y0 > y) != (y1 > y):
                    xs.append(x0 + (y - y0) * (x1 - x0) / (y1 - y0))
        xs.sort()
        for a, b in zip(xs[0::2], xs[1::2]):
            # Rows near the middle are preferred over slightly wider ones near the edge.
            w = (b - a) * (1.0 - abs(t - 0.5))
            if w > best_w:
                best, best_w = ((a + b) / 2, y), w
    return best


class Area:
    def __init__(self, level, key, name, rings, parent=None):
        self.level = level
        self.key = key  # AGS: 2 digits state, 5 district, 8 municipality
        self.name = name
        self.rings = rings  # in degrees, (lat, lon)
        self.parent = parent
        self.label = None  # (lat, lon)
        self.index = None


# Simplification in metres for each level: about a pixel at the altitude where the level shows.
TOLERANCE = {0: 400.0, 1: 150.0, 2: 60.0}


def load_level(folder, layer, level, key_len):
    """Areas of one VG250 layer, merging the several records an area can have (VG250 splits
    land and water parts; only land parts, GF 4, are kept)."""
    rows = read_dbf(f"{folder}/VG250_{layer}.dbf")
    shapes = read_shp(f"{folder}/VG250_{layer}.shp")
    merged = {}
    for row, rings in zip(rows, shapes):
        if row["GF"] != "4" or not rings:
            continue
        key = row["AGS"][:key_len]
        entry = merged.setdefault(key, {"name": row["GEN"], "rings": []})
        entry["rings"] += rings
    areas = []
    for key, entry in sorted(merged.items()):
        utm = [simplify(r, TOLERANCE[level]) for r in entry["rings"]]
        utm = [r for r in utm if len(r) >= 4]
        if not utm:
            # Too small to outline at this tolerance: keep it as it is.
            utm = entry["rings"]
        lx, ly = label_point(utm)
        area = Area(level, key, entry["name"], [[utm_to_deg(e, n) for e, n in r] for r in utm])
        area.label = utm_to_deg(lx, ly)
        area.utm = utm  # kept for point-in-area tests on units without a municipality key
        areas.append(area)
    return areas


def load(folder):
    """States, districts and municipalities, each pointing at its parent."""
    states = load_level(folder, "LAN", 0, 2)
    districts = load_level(folder, "KRS", 1, 5)
    towns = load_level(folder, "GEM", 2, 8)
    by_key = {a.key: a for a in states + districts}
    for d in districts:
        d.parent = by_key[d.key[:2]]
    for t in towns:
        t.parent = by_key[t.key[:5]]
    return states, districts, towns
