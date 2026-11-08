"""Writes data/population_de.bin: the population of every state, district and municipality of
data/energy_de.bin, in that file's order, from BKG's VG250-EW (municipalities with their
population at 31 December).

The energy layer shares Germany's measured consumption out by population, so that each bar has an
estimated demand beside it.

usage: gen_population.py vg250_ebenen_0101/ vg250-ew_ebenen_1231/ data/energy_de.bin data/population_de.bin

The first folder is the one gen_energy.py was given: it fixes the areas and their order. VG250-EW
is a year older in its boundaries, so the few municipalities merged at New Year are found by
position instead of by key.

File: b"ORBP\\x01\\0\\0\\0", u32 date of the population (YYYYMMDD), u32 number of areas N, then N
zigzag varints: the population of areas 0..N of energy_de.bin, which are its states, districts and
municipalities.
"""
import os
import struct
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import energy_areas  # noqa: E402
from gen_energy import TownIndex, varint  # noqa: E402


def energy_area_names(path):
    """Level and name of each area of energy_de.bin, in its order."""
    data = open(path, "rb").read()
    assert data[:8] == b"ORBE\x01\0\0\0", "not an energy file"
    at = 12
    count = struct.unpack_from("<I", data, at)[0]
    at += 4
    out = []
    for _ in range(count):
        level, kind, _parent, n = struct.unpack_from("<BBIB", data, at)
        at += 7
        name = data[at : at + n].decode("utf-8", "replace")
        at += n + 8
        rings = struct.unpack_from("<H", data, at)[0]
        at += 2
        for _ in range(rings):
            points = struct.unpack_from("<I", data, at)[0]
            at += 12
            for _ in range(2 * (points - 1)):
                while data[at] & 0x80:
                    at += 1
                at += 1
        out.append((level, kind, name))
    return out


def main():
    vg250, ew, energy, target = sys.argv[1:5]
    states, districts, towns = energy_areas.load(vg250)
    areas = states + districts + towns
    names = energy_area_names(energy)
    for i, a in enumerate(areas):
        assert names[i][1] == 0 and names[i][0] == a.level and names[i][2] == a.name.encode("utf-8")[:255].decode("utf-8", "replace"), (
            f"area {i}: {names[i]} is not {a.level} {a.name}; was energy_de.bin made from this VG250?"
        )
    by_key = {t.key: t for t in towns}
    index = TownIndex(towns)
    rows = energy_areas.read_dbf(f"{ew}/VG250_GEM.dbf")
    shapes = energy_areas.read_shp(f"{ew}/VG250_GEM.shp")
    people = defaultdict(int)
    moved, lost = 0, 0
    for row, rings in zip(rows, shapes):
        if row["GF"] != "4":
            continue
        n = int(row["EWZ"] or 0)
        town = by_key.get(row["AGS"])
        if town is None and rings:
            lat, lon = energy_areas.utm_to_deg(*energy_areas.label_point(rings))
            town = index.find(lat, lon)
            moved += 1
        if town is None:
            lost += n
            continue
        people[town.key] += n
    for t in towns:
        people[t.parent.key] += people[t.key]
    for d in districts:
        people[d.parent.key] += people[d.key]
    stamp = open(os.path.join(os.path.dirname(os.path.normpath(ew)), "dokumentation", "aktualitaet.txt")).read().split()[0]
    d, m, y = stamp.split(".")
    date = int(y + m + d)
    out = bytearray(b"ORBP\x01\0\0\0") + struct.pack("<II", date, len(areas))
    for a in areas:
        out += varint(people[a.key])
    open(target, "wb").write(out)
    total = sum(people[s.key] for s in states)
    print(f"{total} people in {len(towns)} municipalities ({moved} found by position, {lost} not placed); {len(out)} bytes")


if __name__ == "__main__":
    main()
