#!/usr/bin/env python3
"""Builds data/stars.bin from the HYG database v4.4 (CC BY-SA 4.0, David Nash, astronexus).

usage: gen_stars.py hyg_v44.csv.gz data/stars.bin

Keeps stars of visual magnitude 6.0 or brighter (the Sun is dropped), brightest first.
Each star takes 6 bytes, little endian: right ascension (J2000) as u16 over 0..2π,
declination as i16 over -π/2..π/2, magnitude as u8 = (mag + 1.5) * 20, and colour index
B-V as u8 = (ci + 0.5) * 50, clamped.
"""
import csv, gzip, math, struct, sys

rows = []
with gzip.open(sys.argv[1], "rt", newline="") as f:
    for r in csv.DictReader(f):
        if r["id"] == "0" or r["mag"] == "":
            continue
        mag = float(r["mag"])
        if mag > 6.0:
            continue
        ci = float(r["ci"]) if r["ci"] else 0.6
        rows.append((mag, float(r["rarad"]), float(r["decrad"]), ci))
rows.sort()
out = bytearray()
for mag, ra, dec, ci in rows:
    out += struct.pack(
        "<HhBB",
        round(ra / (2 * math.pi) * 65536) % 65536,
        round(dec / (math.pi / 2) * 32767),
        max(0, min(255, round((mag + 1.5) * 20))),
        max(0, min(255, round((ci + 0.5) * 50))),
    )
open(sys.argv[2], "wb").write(out)
print(len(rows), "stars,", len(out), "bytes")
