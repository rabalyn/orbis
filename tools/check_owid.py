#!/usr/bin/env python3
"""Checks SMARD's quarter-hourly generation in Germany against Ember's monthly figures.

Usage: python3 -I tools/check_owid.py [--cache DIR] [--fetch DIR] [--from YEAR] [--to YEAR]

Sums SMARD's quarter hours (MWh each) into calendar months, German local time, per source,
and lists them against Ember's monthly generation for Germany in data/owid_energy.bin, with the
ratio Ember / SMARD. Weekly files are read from the program's disk cache (--cache, by default
~/.cache/orbis/tiles/power) where it has them, else from --fetch DIR, else fetched from SMARD
into --fetch DIR, at most three requests a second. Without --fetch nothing is fetched and
missing weeks are reported.

MAPPING is the program's mapping of SMARD series to OWID's sources; a test in
src/power_feed.rs keeps it in step with `power_feed::owid_series`.
"""
import datetime
import json
import os
import struct
import sys
import time
import urllib.request
import zoneinfo

BASE = "https://www.smard.de/app/chart_data"
HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, "..", "data", "owid_energy.bin")

# SMARD filter: OWID source (key in owid_energy.bin), or None where Ember has no such figure.
MAPPING = {
    4068: "solar",             # Photovoltaik
    4067: "wind",              # Wind an Land
    1225: "wind",              # Wind auf See
    1226: "hydro",             # Wasserkraft
    4066: "bioenergy",         # Biomasse
    1223: "coal",              # Braunkohle
    4069: "coal",              # Steinkohle
    4071: "gas",               # Erdgas
    1227: "oil",               # Sonstige Konventionelle (Ember's "other fossil")
    1224: "nuclear",           # Kernenergie
    1228: "other_renewables",  # Sonstige Erneuerbare
    4070: None,                # Pumpspeicher: stored energy, not counted by Ember
}
# Netzlast, against Ember's demand.
CONSUMPTION = 410
SOURCES = ["solar", "wind", "hydro", "bioenergy", "other_renewables", "coal", "gas", "oil", "nuclear"]
BERLIN = zoneinfo.ZoneInfo("Europe/Berlin")
last_request = [0.0]


def varint(data, i):
    shift = n = 0
    while True:
        b = data[i]
        i += 1
        n |= (b & 0x7F) << shift
        shift += 7
        if b < 0x80:
            return (n >> 1) ^ -(n & 1), i


def read_owid(code="DEU"):
    """Ember's monthly series for one country: {key: {(year, month): value}}."""
    data = open(BIN, "rb").read()
    assert data[:8] == b"ORBO\x01\0\0\0"
    i = 12
    ny, nm = data[i], data[i + 1]
    i += 2
    keys = []
    for _ in range(ny + nm):
        n = data[i]
        keys.append((data[i + 1:i + 1 + n].decode(), data[i + 1 + n]))
        i += 2 + n
    (count,) = struct.unpack_from("<H", data, i)
    i += 2
    for _ in range(count):
        strings = []
        for _ in range(2):
            n = data[i]
            strings.append(data[i + 1:i + 1 + n].decode())
            i += 1 + n
        i += 2
        found = {"yearly": {}, "monthly": {}}
        for k, (key, dec) in enumerate(keys):
            n, i = varint(data, i)
            t = v = 0
            points = {}
            for _ in range(n):
                dt, i = varint(data, i)
                dv, i = varint(data, i)
                t, v = t + dt, v + dv
                points[t] = v / 10**dec
            if k < ny:
                found["yearly"][key] = points
            else:
                found["monthly"][key] = {(t // 12, t % 12 + 1): v for t, v in points.items()}
        if strings[0] == code:
            return found
    sys.exit(f"{code} is not in {BIN}")


def unpack(body):
    if body.startswith("orbis-feed "):
        body = body.split("\n", 1)[1]
    return body


def get(name, url, cache, fetch):
    for d in (cache, fetch):
        if d and os.path.exists(os.path.join(d, name)):
            return unpack(open(os.path.join(d, name), encoding="utf-8").read())
    if not fetch:
        return None
    wait = last_request[0] + 0.34 - time.time()
    if wait > 0:
        time.sleep(wait)
    last_request[0] = time.time()
    req = urllib.request.Request(url, headers={"User-Agent": "orbis check_owid.py"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            body = r.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        if e.code != 404:
            raise
        body = "missing"
    open(os.path.join(fetch, name), "w", encoding="utf-8").write(body)
    return body


def main():
    args = sys.argv[1:]
    cache = os.path.expanduser("~/.cache/orbis/tiles/power")
    fetch, first, last = None, 2024, 2026
    while args:
        flag, value, args = args[0], args[1], args[2:]
        if flag == "--cache":
            cache = value
        elif flag == "--fetch":
            fetch = value
            os.makedirs(fetch, exist_ok=True)
        elif flag == "--from":
            first = int(value)
        elif flag == "--to":
            last = int(value)
        else:
            sys.exit(__doc__)
    index = get("smard-index.json", f"{BASE}/410/DE/index_quarterhour.json", None, fetch) or get(
        "smard-index.json", "", cache, None)
    if index is None:
        sys.exit("no SMARD index: give --fetch DIR")
    weeks = json.loads(index)["timestamps"]
    lo = datetime.datetime(first, 1, 1, tzinfo=BERLIN).timestamp() * 1000 - 7 * 86_400_000
    hi = datetime.datetime(last + 1, 1, 1, tzinfo=BERLIN).timestamp() * 1000
    weeks = [w for w in weeks if lo <= w < hi]
    sums, quarters, missing = {}, {}, 0
    for filt, key in list(MAPPING.items()) + [(CONSUMPTION, "demand")]:
        for w in weeks:
            body = get(f"smard-{filt}-{w}.json", f"{BASE}/{filt}/DE/{filt}_DE_quarterhour_{w}.json", cache, fetch)
            if body is None:
                missing += 1
                continue
            if body == "missing":
                continue
            for t, v in json.loads(body)["series"]:
                if v is None:
                    continue
                d = datetime.datetime.fromtimestamp(t / 1000, BERLIN)
                if not first <= d.year <= last:
                    continue
                m = (d.year, d.month)
                k = key or "pumped_storage"
                sums.setdefault(m, {}).setdefault(k, 0.0)
                sums[m][k] += v / 1e6  # MWh to TWh
                if filt == 4068:
                    quarters[m] = quarters.get(m, 0) + 1
    if missing:
        print(f"{missing} weekly files are neither cached nor fetched; give --fetch DIR", file=sys.stderr)
    ember = read_owid()["monthly"]
    report(sums, quarters, ember, first, last)


def ratio(e, s):
    return f"{e / s:6.3f}" if e is not None and s else "     -"


def report(sums, quarters, ember, first, last):
    cols = SOURCES + ["total", "demand"]
    print("Ember / SMARD; total generation leaves out pumped storage; demand against SMARD's Netzlast")
    print("SMARD, Ember: total generation in TWh; pumped: SMARD's pumped storage in TWh\n")
    print("month   " + "".join(f"{c[:9]:>10}" for c in cols) + "   SMARD   Ember  pumped")
    for year in range(first, last + 1):
        ysm, yem = {}, {}
        for month in range(1, 13):
            m = (year, month)
            if m not in sums:
                continue
            full = quarters.get(m, 0) >= 4 * 24 * 28 - 8
            s = dict(sums[m])
            s["total"] = sum(s.get(k, 0.0) for k in SOURCES)
            e = {k: ember.get(k, {}).get(m) for k in SOURCES}
            e["total"] = ember.get("generation", {}).get(m)
            e["demand"] = ember.get("demand", {}).get(m)
            line = f"{year}-{month:02} " + "".join(f"    {ratio(e[c], s.get(c))}" for c in cols)
            et = f"{e['total']:7.2f}" if e["total"] is not None else "      -"
            print(line + f" {s['total']:7.2f} {et} {s.get('pumped_storage', 0):7.2f}" + ("" if full else "  (part)"))
            if full and e["total"] is not None:
                for c in cols:
                    ysm[c] = ysm.get(c, 0.0) + s.get(c, 0.0)
                    yem[c] = yem.get(c, 0.0) + (e[c] or 0.0)
        if ysm:
            n = sum(1 for mo in range(1, 13) if (year, mo) in sums and ember.get("generation", {}).get((year, mo)))
            print(f"{year} {n:2}m " + "".join(f"    {ratio(yem[c], ysm[c])}" for c in cols)
                  + f" {ysm['total']:7.2f} {yem['total']:7.2f}")
            print()


if __name__ == "__main__":
    main()
