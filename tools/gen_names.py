"""Builds data/names.tsv: the IAU names of features on the Moon, Mars, Mercury, Ceres and Vesta,
from the USGS Gazetteer of Planetary Nomenclature (public domain).

usage: gen_names.py DOWNLOADS data/names.tsv

DOWNLOADS holds the unpacked center-point shapefiles of the Gazetteer's GIS downloads,
https://planetarynames.wr.usgs.gov/GIS_Downloads, as
DOWNLOADS/nom_MOON/MOON_nomenclature_center_pts.dbf and so on. Only the .dbf is read.

Each line is: body, name, latitude, east longitude in -180..180 (degrees, planetocentric),
diameter in km, and R for a wide region (maria, plains, lands, albedo features) or F for any
other feature. Albedo features without a size are given 800 km, so they are named from afar
like the regions they are. Lines are sorted by body, then largest first.
"""
import os
import struct
import sys

BODIES = [("MOON", "moon"), ("MARS", "mars"), ("MERCURY", "mercury"), ("CERES", "ceres"), ("VESTA", "vesta")]
REGIONS = {"ME", "OC", "LC", "SI", "PA", "PL", "TA", "VS", "AL", "RE", "PM"}


def read_dbf(path):
    f = open(path, "rb").read()
    n, hl, rl = struct.unpack("<xxxxIHH", f[:12])
    fields, p = [], 32
    while f[p] != 0x0D:
        fields.append((f[p:p + 11].split(b"\0")[0].decode(), f[p + 16]))
        p += 32
    for i in range(n):
        r = f[hl + i * rl:hl + (i + 1) * rl]
        if r[:1] == b"*":
            continue
        o, d = 1, {}
        for name, ln in fields:
            d[name] = r[o:o + ln].decode("utf-8", "replace").strip()
            o += ln
        yield d


def main():
    src, out = sys.argv[1], sys.argv[2]
    lines = []
    for upper, body in BODIES:
        rows = []
        for r in read_dbf(os.path.join(src, f"nom_{upper}", f"{upper}_nomenclature_center_pts.dbf")):
            if r["approval"] != "Adopted by IAU":
                continue
            name = r["clean_name"] or r["name"]
            if not name or "\t" in name:
                continue
            lat, lon = float(r["center_lat"]), float(r["center_lon"])
            if lon > 180:
                lon -= 360
            diameter = float(r["diameter"] or 0)
            if diameter <= 0:
                diameter = 800.0 if r["code"] == "AL" else 1.0
            kind = "R" if r["code"] in REGIONS else "F"
            rows.append((diameter, name, lat, lon, kind))
        rows.sort(key=lambda x: (-x[0], x[1]))
        for diameter, name, lat, lon, kind in rows:
            lines.append(f"{body}\t{name}\t{lat:.4f}\t{lon:.4f}\t{diameter:.3g}\t{kind}")
        print(f"{body}: {len(rows)} names", file=sys.stderr)
    with open(out, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
