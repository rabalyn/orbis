#!/usr/bin/env python3
"""Writes data/owid_energy.bin and data/countries.bin: electricity of every country by year and,
where Ember has it, by month, from Our World in Data; and country outlines from Natural Earth.

usage: gen_owid.py [--from DIR] [--only DEU,FRA,...] data/owid_energy.bin data/countries.bin

Without --from, the inputs are downloaded into a fresh temporary folder: OWID's energy dataset
(catalog.ourworldindata.org/energy/owid_energy/owid_energy.csv), the monthly series of the
`electricity-mix` chart (one CSV per source and metric, see MONTHLY) and Natural Earth's admin-0
countries at 1:50m (ne_50m_admin_0_countries.geojson). With --from, they are read from DIR under
those file names (owid_energy.csv, monthly_<key>.csv, ne_50m_admin_0_countries.geojson). --only
keeps the given codes, for test fixtures. Run with `python3 -I`.

Countries are keyed by ISO 3166 alpha-3, the World by OWID_WRL and Kosovo by OWID_KOS, as OWID
does. Regions and other aggregates are left out.

Integers are zigzag varints (as in gen_energy.py); strings are a u8 length and UTF-8.

owid_energy.bin: b"ORBO\\x01\\0\\0\\0", u32 date of retrieval (YYYYMMDD), then the series table:
u8 number of yearly series, u8 number of monthly series, and for each (yearly first) its key
(string) and u8 decimals d (values are stored as round(value * 10^d)). Then u16 number of
countries, and for each: code, name, u16 first year of Ember's figures (0 if none; earlier
electricity figures are the Energy Institute's), then for each yearly series a varint count n and
n pairs (varint step in year from the last, or from 0; varint step in value from the last, or
from 0); then the same for each monthly series, with months counted as year * 12 + month - 1.

countries.bin: b"ORBC\\x01\\0\\0\\0", u16 number of countries, and for each: code, name, Wikidata
item (string, such as Q183), label point as varint latitude and longitude in 1e-4 degrees, u16
number of rings, and for each ring a varint count of points (the first not repeated at the end)
and the points as varint steps in latitude and longitude, 1e-4 degrees, from the last point (the
first from the label point). Rings of a country are taken even-odd, so holes are rings too.
"""
import csv
import datetime
import json
import os
import struct
import sys
import tempfile
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from energy_areas import signed_area, simplify  # noqa: E402
from gen_energy import varint  # noqa: E402

ENERGY_URL = "https://catalog.ourworldindata.org/energy/owid_energy/owid_energy.csv"
MIX_URL = "https://ourworldindata.org/grapher/electricity-mix.csv?frequency=monthly&metric={metric}&source={source}&useColumnShortNames=true"
NE_URL = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_50m_admin_0_countries.geojson"

# Key in the file, column of owid_energy.csv, decimals kept.
YEARLY = [
    ("population", "population", 0),
    ("generation", "electricity_generation_twh", 3),
    ("demand", "electricity_demand_twh", 3),
    ("net_imports", "electricity_net_imports_twh", 3),
    ("carbon_intensity", "electricity_carbon_intensity_gco2eq_per_kwh", 2),
    ("renewable_share", "renewables_electricity_share_pct", 2),
    ("coal", "coal_electricity_twh", 3),
    ("gas", "gas_electricity_twh", 3),
    ("oil", "oil_electricity_twh", 3),
    ("nuclear", "nuclear_electricity_twh", 3),
    ("hydro", "hydro_electricity_twh", 3),
    ("wind", "wind_electricity_twh", 3),
    ("solar", "solar_electricity_twh", 3),
    ("bioenergy", "bioenergy_electricity_twh", 3),
    ("other_renewables", "other_renewables_excluding_bioenergy_electricity_twh", 3),
]
# Key in the file, metric and source of the electricity-mix chart, decimals kept.
MONTHLY = [
    ("generation", "generation", "total", 3),
    ("demand", "demand", "total", 3),
    ("net_imports", "net_imports", "total", 3),
    ("carbon_intensity", "carbon_intensity", "total", 2),
    ("coal", "generation", "coal", 3),
    ("gas", "generation", "gas", 3),
    ("oil", "generation", "oil", 3),
    ("nuclear", "generation", "nuclear", 3),
    ("hydro", "generation", "hydro", 3),
    ("wind", "generation", "wind", 3),
    ("solar", "generation", "solar", 3),
    ("bioenergy", "generation", "bioenergy", 3),
    ("other_renewables", "generation", "other_renewables", 3),
]
FIRST_YEAR = 1900
# Natural Earth features without an ISO code of their own that OWID has figures for.
NE_CODES = {"KOS": "OWID_KOS"}
# Douglas-Peucker tolerance and the smallest island kept apart from a country's largest ring.
TOLERANCE = 0.02
MIN_AREA = 0.002


def fetch(url, path):
    req = urllib.request.Request(url, headers={"User-Agent": "orbis gen_owid.py"})
    with urllib.request.urlopen(req, timeout=120) as r, open(path, "wb") as f:
        f.write(r.read())
    print("fetched", url, os.path.getsize(path), "bytes")


def string(s):
    b = s.encode("utf-8")[:255]
    return bytes([len(b)]) + b


def series(points):
    """points: sorted (time, int value)."""
    out = bytearray(varint(len(points)))
    t0, v0 = 0, 0
    for t, v in points:
        out += varint(t - t0) + varint(v - v0)
        t0, v0 = t, v
    return out


def read_yearly(path, only):
    countries = {}
    with open(path, newline="", encoding="utf-8") as f:
        for r in csv.DictReader(f):
            code = r["iso_code"]
            if r["country"] == "World":
                code = "OWID_WRL"
            elif r["country"] == "Kosovo" and not code:
                code = "OWID_KOS"
            if len(code) != 3 and code not in ("OWID_WRL", "OWID_KOS"):
                continue
            if only and code not in only:
                continue
            year = int(r["year"])
            if year < FIRST_YEAR:
                continue
            c = countries.setdefault(code, {"name": r["country"], "yearly": [[] for _ in YEARLY], "monthly": [[] for _ in MONTHLY]})
            for k, (_, col, dec) in enumerate(YEARLY):
                if r[col] != "":
                    c["yearly"][k].append((year, round(float(r[col]) * 10**dec)))
    return countries


def read_monthly(countries, k, path, dec):
    with open(path, newline="", encoding="utf-8") as f:
        rows = csv.reader(f)
        head = next(rows)
        assert head[:3] == ["entity", "code", "month"], f"{path}: unexpected columns {head}"
        for entity, code, month, value in rows:
            if code not in countries or value == "":
                continue
            y, m = month.split("-")
            countries[code]["monthly"][k].append((int(y) * 12 + int(m) - 1, round(float(value) * 10**dec)))


def write_energy(countries, target, date):
    out = bytearray(b"ORBO\x01\0\0\0") + struct.pack("<IBB", date, len(YEARLY), len(MONTHLY))
    for key, _, dec in YEARLY:
        out += string(key) + bytes([dec])
    for key, _, _, dec in MONTHLY:
        out += string(key) + bytes([dec])
    out += struct.pack("<H", len(countries))
    for code in sorted(countries):
        c = countries[code]
        # Demand is Ember's alone, so its first year is where Ember's figures begin.
        ember = c["yearly"][2][0][0] if c["yearly"][2] else 0
        out += string(code) + string(c["name"]) + struct.pack("<H", ember)
        for s in c["yearly"] + c["monthly"]:
            out += series(sorted(s))
    open(target, "wb").write(out)
    monthly = sum(1 for c in countries.values() if c["monthly"][0])
    print(len(countries), "countries,", monthly, "with monthly figures,", len(out), "bytes")


def ne_code(p):
    for key in ("ISO_A3", "ISO_A3_EH"):
        if p[key] != "-99":
            return p[key]
    return NE_CODES.get(p["ADM0_A3"])


def write_countries(path, target, only):
    features = json.load(open(path, encoding="utf-8"))["features"]
    countries = {}
    for f in features:
        p = f["properties"]
        code = ne_code(p)
        if code is None or (only and code not in only):
            continue
        g = f["geometry"]
        polys = g["coordinates"] if g["type"] == "MultiPolygon" else [g["coordinates"]]
        c = countries.setdefault(code, {"rings": [], "main": None})
        # The feature whose own code is the country's gives the name and the label point.
        if c["main"] is None or (p["ADM0_A3"] == code and c["main"]["ADM0_A3"] != code):
            c["main"] = p
        for poly in polys:
            for ring in poly:
                c["rings"].append([(lon, lat) for lon, lat in ring])
    out = bytearray(b"ORBC\x01\0\0\0") + struct.pack("<H", len(countries))
    points = 0
    for code in sorted(countries):
        c = countries[code]
        p = c["main"]
        rings = [r for r in (simplify(r, TOLERANCE) for r in c["rings"]) if r]
        biggest = max((abs(signed_area(r)) for r in rings), default=0)
        rings = [r for r in rings if abs(signed_area(r)) >= min(MIN_AREA, biggest)]
        lat0, lon0 = round(p["LABEL_Y"] * 1e4), round(p["LABEL_X"] * 1e4)
        out += string(code) + string(p["NAME_EN"] or p["NAME"]) + string(p["WIKIDATAID"] or "")
        out += varint(lat0) + varint(lon0) + struct.pack("<H", len(rings))
        for r in rings:
            r = r[:-1]
            out += varint(len(r))
            la, lo = lat0, lon0
            for lon, lat in r:
                a, o = round(lat * 1e4), round(lon * 1e4)
                out += varint(a - la) + varint(o - lo)
                la, lo = a, o
            points += len(r)
    open(target, "wb").write(out)
    print(len(countries), "outlines,", points, "points,", len(out), "bytes")


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
    if len(args) != 2:
        sys.exit(__doc__)
    energy_target, countries_target = args
    if src is None:
        src = tempfile.mkdtemp(prefix="owid-")
        print("inputs go to", src, "(give it to --from to build again from them)")
        fetch(ENERGY_URL, os.path.join(src, "owid_energy.csv"))
        for key, metric, source, _ in MONTHLY:
            fetch(MIX_URL.format(metric=metric, source=source), os.path.join(src, f"monthly_{key}.csv"))
        fetch(NE_URL, os.path.join(src, "ne_50m_admin_0_countries.geojson"))
        date = int(datetime.date.today().strftime("%Y%m%d"))
    else:
        date = int(datetime.date.fromtimestamp(os.path.getmtime(os.path.join(src, "owid_energy.csv"))).strftime("%Y%m%d"))
    countries = read_yearly(os.path.join(src, "owid_energy.csv"), only)
    for k, (key, _, _, dec) in enumerate(MONTHLY):
        read_monthly(countries, k, os.path.join(src, f"monthly_{key}.csv"), dec)
    write_energy(countries, energy_target, date)
    write_countries(os.path.join(src, "ne_50m_admin_0_countries.geojson"), countries_target, only)


if __name__ == "__main__":
    main()
