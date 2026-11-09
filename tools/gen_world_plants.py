#!/usr/bin/env python3
"""Writes data/plants_world.bin: the world's power plants outside Germany, from Global Energy
Monitor's Global Integrated Power Tracker (GIPT, CC BY 4.0), for the Plants level of the energy
layer (`src/world_plants.rs`). Germany's plants come from the Marktstammdatenregister
(gen_energy.py), so GEM's German units are left out.

usage: gen_world_plants.py [--min-mw MW] [--far-min-mw MW] PATH [OUT]

PATH is GEM's spreadsheet (.xlsx, sheets "Power facilities" and "Regions, area, and countries";
needs openpyxl, `pip install openpyxl` in a virtual environment), or a CSV of the same columns
for test fixtures, with the countries' codes in a `regions.csv` beside it (columns "GEM Standard
Country Name/Area" and "ISO-alpha3 Code"). OUT defaults to data/plants_world.bin. Run with
`python3 -I`. Columns are found by their header names, so added or moved columns do no harm.

What is kept:
- Units with status operating, mothballed or retired; announced, pre-construction, construction,
  shelved and cancelled ones are dropped. A retired unit needs a retirement year, every unit a
  start year. Operating and mothballed units without a start year are kept, marked unknown
  (the program counts them from the release on).
- Units of at least --min-mw (1 MW); solar and wind outside Europe and the United States of at
  least --far-min-mw (10 MW, to keep the web build small: at 1 MW the web build grew by 3 MB).
- Not units partly or wholly from TransitionZero's Solar Asset Mapper (a "TZ:" ID in "Other IDs
  (unit/phase)"): those are CC BY-NC 4.0, not CC BY 4.0, and are left out.
- Nationally aggregated distributed (<1 MW) solar, which GEM gives per country without a
  position (rows of solar with no coordinates, or with "distributed" and "aggregated" in their
  name), is not made into plants but summed per country into the file's own table.

Units of one source at one GEM location (its "GEM location ID", else the plant's name and
position) make one site. Years become months: a unit counts from January of its start year (1900 at the earliest), a
retired one through December of its retirement year.

Sources (as `energy::Source`): utility-scale solar: Solar; wind: Wind at sea where the technology
says offshore, else on land; hydropower (pumped storage too): Hydro; bioenergy and geothermal:
Biomass (as Ember's other renewables are drawn); nuclear: Nuclear; coal: Lignite where the fuel
says lignite, else Hard coal; oil/gas: by the first fuel listed, fossil gas Gas, fossil liquids
Oil, coal Hard coal, anything else (blast furnace gas and the like) Other fossil, none Gas.

Integers are zigzag varints (as in gen_energy.py); strings are a u8 length and UTF-8 (cut at 255
bytes).

plants_world.bin: b"ORBW\\x01\\0\\0\\0", u32 release date YYYYMMDD,
  u16 number of countries and their codes (strings; as in countries.bin),
  u16 number of distributed solar entries, each a varint country index and varint MW times 10,
  u16 number of GEM types and their names, u16 number of details (technology and fuel) and their
  texts, u32 number of owners and their names,
  u32 number of sites, sorted by country, then latitude, then longitude, each:
    varint country index, varint steps from the last site's latitude and longitude in 1e-4
    degrees, u8 source (index in energy::Source::ALL), u8 GEM type index, name (string),
    varint owner index plus one (0: none), varint detail index plus one (0: none), the gem.wiki
    page (string, after "https://www.gem.wiki/"; empty for none), varint number of units and for
    each: varint start month (months from January 1900) plus one (0: unknown), varint months it
    ran plus one (0: still there; always 0 for an unknown start), varint MW times 10.
"""

import collections
import csv
import os
import re
import struct
import sys

MAGIC = b"ORBW\x01\0\0\0"
RELEASE = 20260901
KEEP = {"operating", "mothballed", "retired"}
WIKI = "https://www.gem.wiki/"
OPEN = None

# energy::Source::ALL
SOLAR, WIND_ON, WIND_OFF, HYDRO, BIOMASS, LIGNITE, HARD_COAL, GAS, OIL, OTHER_FOSSIL, NUCLEAR = range(11)

# Fallback codes where GEM's own table has none; keyed as in countries.bin.
CODE_FIXES = {"Kosovo": "OWID_KOS"}
# Areas GEM lists apart that countries.bin (Natural Earth's admin-0) counts to their country.
PART_OF = {"GLP": "FRA", "GUF": "FRA", "MTQ": "FRA", "REU": "FRA", "MYT": "FRA", "BES": "NLD"}
GERMANY = "DEU"


def header_index(header, *names, required=True):
    """The column of the first of `names` found, by case-insensitive header name."""
    norm = [str(h or "").strip().lower() for h in header]
    for name in names:
        if name.lower() in norm:
            return norm.index(name.lower())
    for name in names:
        for i, h in enumerate(norm):
            if h.startswith(name.lower()):
                return i
    if required:
        sys.exit(f"no column {names[0]!r} in {header}")
    return None


def read_rows(path):
    """(header, rows of the facilities, {GEM country name: code})."""
    if path.lower().endswith(".csv"):
        with open(path, newline="", encoding="utf-8") as f:
            rows = list(csv.reader(f))
        regions = os.path.join(os.path.dirname(path), "regions.csv")
        with open(regions, newline="", encoding="utf-8") as f:
            reg = list(csv.reader(f))
        return rows[0], rows[1:], codes_of(reg[0], reg[1:])
    import openpyxl

    wb = openpyxl.load_workbook(path, read_only=True, data_only=True)
    sheet = next(n for n in wb.sheetnames if n.lower().startswith("power facilities"))
    it = wb[sheet].iter_rows(values_only=True)
    header = next(it)
    rows = [r for r in it]
    reg_sheet = next((n for n in wb.sheetnames if n.lower().startswith("regions")), None)
    codes = {}
    if reg_sheet:
        r = wb[reg_sheet].iter_rows(values_only=True)
        codes = codes_of(next(r), list(r))
    return header, rows, codes


def codes_of(header, rows):
    name = header_index(header, "GEM Standard Country Name/Area", "Country/area")
    code = header_index(header, "ISO-alpha3 Code", "ISO3")
    out = dict(CODE_FIXES)
    for r in rows:
        if len(r) > max(name, code) and r[name] and r[code]:
            out.setdefault(str(r[name]).strip(), str(r[code]).strip())
    return out


def number(v):
    if v is None or v == "":
        return None
    try:
        return float(v)
    except (TypeError, ValueError):
        m = re.match(r"\s*(-?\d+(?:\.\d+)?)", str(v))
        return float(m.group(1)) if m else None


def text(v):
    s = "" if v is None else str(v).strip()
    return "" if s in ("--", "-", "None", "nan") else s


def source_of(kind, tech, fuel):
    kind, tech, fuel = kind.lower(), tech.lower(), fuel.lower()
    if "solar" in kind:
        return SOLAR
    if "wind" in kind:
        return WIND_OFF if "offshore" in tech else WIND_ON
    if "hydro" in kind:
        return HYDRO
    if "bio" in kind or "geothermal" in kind:
        return BIOMASS
    if "nuclear" in kind:
        return NUCLEAR
    if "coal" in kind:
        return LIGNITE if "lignite" in fuel else HARD_COAL
    if "oil" in kind or "gas" in kind:
        first = fuel.split(",")[0].strip()
        if not first or first.startswith("fossil gas"):
            return GAS
        if first.startswith("fossil liquids"):
            return OIL
        if first.startswith("coal"):
            return HARD_COAL
        return OTHER_FOSSIL
    return None


def varint(v):
    v = (v << 1) ^ (v >> 63)
    out = bytearray()
    while True:
        b = v & 0x7F
        v >>= 7
        if v:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def string(s):
    b = s.encode("utf-8")[:255]
    while b:
        try:
            b.decode("utf-8")
            break
        except UnicodeDecodeError:
            b = b[:-1]
    return bytes([len(b)]) + b


def owner_of(s):
    """GEM's owners without their shares: "A [60%]; B [40%]" becomes "A; B"."""
    return re.sub(r"\s*\[[^\]]*\]", "", s).strip("; ")


def main(argv):
    args = list(argv)
    min_mw, far_min_mw = 1.0, 10.0
    while args and args[0].startswith("--"):
        flag = args.pop(0)
        if flag == "--min-mw":
            min_mw = float(args.pop(0))
        elif flag == "--far-min-mw":
            far_min_mw = float(args.pop(0))
        else:
            sys.exit(__doc__)
    if not args:
        sys.exit(__doc__)
    path = args[0]
    out_path = args[1] if len(args) > 1 else os.path.join(os.path.dirname(__file__), "..", "data", "plants_world.bin")
    header, rows, codes = read_rows(path)
    col = {
        "type": header_index(header, "Type"),
        "country": header_index(header, "Country/area"),
        "country1": header_index(header, "Country/area 1 (hydropower only)", required=False),
        "region": header_index(header, "Region", required=False),
        "name": header_index(header, "Plant / Project name"),
        "mw": header_index(header, "Capacity (MW)"),
        "status": header_index(header, "Status"),
        "start": header_index(header, "Start year"),
        "retired": header_index(header, "Retired year"),
        "tech": header_index(header, "Technology", required=False),
        "fuel": header_index(header, "Fuel (combustion only)", "Fuel", required=False),
        "owner": header_index(header, "Owner(s)", "Owner", required=False),
        "lat": header_index(header, "Latitude"),
        "lon": header_index(header, "Longitude"),
        "other_ids": header_index(header, "Other IDs (unit/phase)", required=False),
        "location": header_index(header, "GEM location ID", required=False),
        "wiki": header_index(header, "GEM.Wiki URL", "Wiki URL", required=False),
    }

    def get(r, k):
        i = col[k]
        return r[i] if i is not None and i < len(r) else None

    skipped = collections.Counter()
    counts = collections.Counter()
    unknown = collections.Counter()
    kinds, details, owners = [], {}, {}
    kind_index = {}
    distributed = collections.Counter()
    sites = {}
    for r in rows:
        status = text(get(r, "status")).lower()
        if status not in KEEP:
            skipped["planned, shelved or cancelled"] += 1
            continue
        country_name = text(get(r, "country"))
        code = codes.get(country_name) or codes.get(text(get(r, "country1")))
        code = PART_OF.get(code, code)
        if not code:
            unknown[country_name] += 1
            skipped["unknown country"] += 1
            continue
        if code == GERMANY:
            skipped["Germany (from MaStR)"] += 1
            continue
        if re.search(r"(^|[\s,;])TZ:", text(get(r, "other_ids"))):
            skipped["TransitionZero (CC BY-NC)"] += 1
            continue
        kind = text(get(r, "type"))
        tech, fuel = text(get(r, "tech")), text(get(r, "fuel"))
        source = source_of(kind, tech, fuel)
        if source is None:
            skipped[f"type {kind!r}"] += 1
            continue
        mw = number(get(r, "mw"))
        if not mw or mw <= 0:
            skipped["no capacity"] += 1
            continue
        lat, lon = number(get(r, "lat")), number(get(r, "lon"))
        name = text(get(r, "name"))
        if source == SOLAR and (lat is None or lon is None or ("distributed" in name.lower() and "aggregated" in name.lower())):
            if status != "retired":
                distributed[code] += mw
            skipped["distributed solar (per country)"] += 1
            continue
        if lat is None or lon is None or not (-90 <= lat <= 90 and -180 <= lon <= 180):
            skipped["no position"] += 1
            continue
        far = text(get(r, "region")).lower() != "europe" and code != "USA"
        least = far_min_mw if far and source in (SOLAR, WIND_ON, WIND_OFF) else min_mw
        if mw < least:
            skipped[f"below {least:g} MW"] += 1
            continue
        start_year = number(get(r, "start"))
        start = None
        if start_year is not None and 1880 <= start_year <= 2100:
            start = max(0, (int(start_year) - 1900) * 12)  # months count from 1900
        elif status == "retired":
            skipped["retired without a start year"] += 1
            continue
        else:
            counts["start year unknown"] += 1
        end = OPEN
        if status == "retired":
            ry = number(get(r, "retired"))
            if ry is None or not 1880 <= ry <= 2100:
                skipped["retired without a year"] += 1
                continue
            end = max(start + 1, (int(ry) - 1900) * 12 + 12)
        if kind not in kind_index:
            kind_index[kind] = len(kinds)
            kinds.append(kind)
        detail = "; ".join(x for x in (tech, fuel) if x and x.lower() != "unknown")
        location = text(get(r, "location"))
        key = (code, location or (name, round(lat, 3), round(lon, 3)), source)
        site = sites.get(key)
        if site is None:
            wiki = text(get(r, "wiki"))
            site = sites[key] = {
                "code": code,
                "lat": round(lat * 1e4),
                "lon": round(lon * 1e4),
                "source": source,
                "kind": kind_index[kind],
                "name": name,
                "owners": collections.Counter(),
                "details": collections.Counter(),
                "wiki": wiki[len(WIKI):] if wiki.startswith(WIKI) else "",
                "units": [],
            }
        owner = owner_of(text(get(r, "owner")))
        if owner:
            site["owners"][owner] += mw
        if detail:
            site["details"][detail] += mw
        site["units"].append((start, end, max(1, round(mw * 10))))

    countries = sorted({s["code"] for s in sites.values()} | set(distributed))
    cidx = {c: i for i, c in enumerate(countries)}
    order = sorted(sites.values(), key=lambda s: (s["code"], s["lat"], s["lon"], s["source"], s["name"]))
    owner_list, detail_list = [], []
    for s in order:
        for field, table, lst in (("owners", owners, owner_list), ("details", details, detail_list)):
            top = s[field].most_common(1)
            s[field] = None
            if top:
                v = top[0][0]
                if v not in table:
                    table[v] = len(lst)
                    lst.append(v)
                s[field] = table[v] + 1
    out = bytearray(MAGIC)
    out += struct.pack("<I", RELEASE)
    out += struct.pack("<H", len(countries)) + b"".join(string(c) for c in countries)
    out += struct.pack("<H", len(distributed))
    for c, mw in sorted(distributed.items()):
        out += varint(cidx[c]) + varint(round(mw * 10))
    out += struct.pack("<H", len(kinds)) + b"".join(string(k) for k in kinds)
    out += struct.pack("<H", len(detail_list)) + b"".join(string(d) for d in detail_list)
    out += struct.pack("<I", len(owner_list)) + b"".join(string(o) for o in owner_list)
    out += struct.pack("<I", len(order))
    lat = lon = 0
    for s in order:
        out += varint(cidx[s["code"]]) + varint(s["lat"] - lat) + varint(s["lon"] - lon)
        lat, lon = s["lat"], s["lon"]
        out += bytes([s["source"], s["kind"]]) + string(s["name"])
        out += varint(s["owners"] or 0) + varint(s["details"] or 0) + string(s["wiki"])
        units = sorted(s["units"], key=lambda u: (u[0] is not None, u[0] or 0, u[1] is None, u[1] or 0, u[2]))
        out += varint(len(units))
        for start, end, mw10 in units:
            ran = 0 if end is OPEN else end - start + 1
            out += varint(0 if start is None else start + 1) + varint(ran) + varint(mw10)
    with open(out_path, "wb") as f:
        f.write(out)
    units = sum(len(s["units"]) for s in order)
    print(f"{out_path}: {len(out) / 1e6:.2f} MB, {len(order)} sites of {units} units in {len(countries)} countries")
    print("skipped:", dict(skipped), dict(counts))
    if unknown:
        print("unknown countries:", dict(unknown.most_common(20)))
    if distributed:
        print("distributed solar, MW:", {c: round(v, 1) for c, v in sorted(distributed.items())})


if __name__ == "__main__":
    main(sys.argv[1:])
