//! Outlines and label points of the world's countries, from Natural Earth's admin-0 countries at
//! 1:50m (public domain), simplified by `tools/gen_owid.py` into `data/countries.bin` and keyed
//! by the same codes as `owid.rs`. Positions are in degrees.

// A few readers are used by the tests only.
#![cfg_attr(not(test), allow(dead_code))]

use crate::owid::Reader;

pub struct Outline {
    /// ISO 3166 alpha-3, or OWID's code where there is none (`OWID_KOS`).
    pub code: String,
    /// Natural Earth's English name.
    pub name: String,
    /// The Wikidata item, such as `Q183`; empty if Natural Earth has none.
    pub wikidata: String,
    /// Where the name goes: latitude and longitude.
    pub label: [f64; 2],
    /// Rings of [latitude, longitude], not closed; taken even-odd, so holes are rings too.
    pub rings: Vec<Vec<[f64; 2]>>,
    /// South, west, north, east.
    pub bounds: [f64; 4],
}

impl Outline {
    /// Whether a point lies inside. Natural Earth cuts outlines at the antimeridian, so plain
    /// longitudes do.
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        let [s, w, n, e] = self.bounds;
        if lat < s || lat > n || lon < w || lon > e {
            return false;
        }
        let mut hit = false;
        for ring in &self.rings {
            let mut prev = ring[ring.len() - 1];
            for &p in ring {
                let ([y0, x0], [y1, x1]) = (prev, p);
                if (y0 > lat) != (y1 > lat) && lon < x0 + (lat - y0) * (x1 - x0) / (y1 - y0) {
                    hit = !hit;
                }
                prev = p;
            }
        }
        hit
    }
}

const MAGIC: &[u8; 8] = b"ORBC\x01\0\0\0";

pub struct Countries {
    /// Sorted by code.
    list: Vec<Outline>,
}

impl Countries {
    /// The outlines built into the program.
    pub fn bundled() -> Option<&'static Countries> {
        static DATA: std::sync::OnceLock<Option<Countries>> = std::sync::OnceLock::new();
        DATA.get_or_init(|| Countries::parse(include_bytes!("../data/countries.bin"))).as_ref()
    }

    pub fn parse(data: &[u8]) -> Option<Countries> {
        let mut r = Reader { data, at: 0 };
        if r.bytes(8)? != MAGIC {
            return None;
        }
        let n = r.u16()? as usize;
        let mut list = Vec::with_capacity(n);
        let deg = |v: i64| v as f64 * 1e-4;
        for _ in 0..n {
            let code = r.string()?;
            let name = r.string()?;
            let wikidata = r.string()?;
            let (lat0, lon0) = (r.varint()?, r.varint()?);
            let mut rings = Vec::new();
            let mut bounds = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
            for _ in 0..r.u16()? {
                let points = usize::try_from(r.varint()?).ok()?;
                if points < 3 {
                    return None;
                }
                let (mut lat, mut lon) = (lat0, lon0);
                let mut ring = Vec::with_capacity(points.min(1 << 16));
                for _ in 0..points {
                    lat += r.varint()?;
                    lon += r.varint()?;
                    let p = [deg(lat), deg(lon)];
                    bounds = [bounds[0].min(p[0]), bounds[1].min(p[1]), bounds[2].max(p[0]), bounds[3].max(p[1])];
                    ring.push(p);
                }
                rings.push(ring);
            }
            list.push(Outline { code, name, wikidata, label: [deg(lat0), deg(lon0)], rings, bounds });
        }
        list.sort_by(|a, b| a.code.cmp(&b.code));
        Some(Countries { list })
    }

    pub fn get(&self, code: &str) -> Option<&Outline> {
        let i = self.list.binary_search_by(|c| c.code.as_str().cmp(code)).ok()?;
        Some(&self.list[i])
    }

    pub fn by_wikidata(&self, item: &str) -> Option<&Outline> {
        self.list.iter().find(|c| !c.wikidata.is_empty() && c.wikidata == item)
    }

    /// The country a point lies in.
    pub fn at(&self, lat: f64, lon: f64) -> Option<&Outline> {
        self.list.iter().find(|c| c.contains(lat, lon))
    }

    pub fn all(&self) -> &[Outline] {
        &self.list
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owid::Owid;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/owid/countries.bin");

    fn code(c: &Countries, lat: f64, lon: f64) -> Option<&str> {
        c.at(lat, lon).map(|o| o.code.as_str())
    }

    #[test]
    fn cities_in_fixture() {
        let c = Countries::parse(FIXTURE).unwrap();
        assert_eq!(code(&c, 52.52, 13.405), Some("DEU")); // Berlin
        assert_eq!(code(&c, 48.137, 11.575), Some("DEU")); // Munich
        assert_eq!(code(&c, 48.857, 2.352), Some("FRA")); // Paris
        assert_eq!(code(&c, 59.913, 10.752), Some("NOR")); // Oslo, Norway being -99 in ISO_A3
        assert_eq!(code(&c, 34.53, 69.17), Some("AFG")); // Kabul
        assert_eq!(code(&c, -26.2, 28.05), Some("ZAF")); // Johannesburg
        assert_eq!(code(&c, -29.31, 27.48), Some("LSO")); // Maseru, in a hole of South Africa
        assert_eq!(code(&c, 42.66, 21.16), Some("OWID_KOS")); // Pristina
        assert_eq!(code(&c, 54.5, 3.0), None); // North Sea
        assert_eq!(code(&c, 40.71, -74.0), None); // New York, not in the fixture
    }

    #[test]
    fn labels_and_items() {
        let c = Countries::parse(FIXTURE).unwrap();
        let de = c.get("DEU").unwrap();
        assert_eq!(de.name, "Germany");
        assert_eq!(de.wikidata, "Q183");
        assert!(de.contains(de.label[0], de.label[1]));
        // France's label is in Europe, not in French Guiana.
        let fr = c.get("FRA").unwrap();
        assert!(fr.label[0] > 42.0 && fr.label[1] > -5.0);
        assert!(fr.contains(4.93, -52.33)); // Cayenne
        assert_eq!(c.by_wikidata("Q183").unwrap().code, "DEU");
        assert!(c.get("OWID_WRL").is_none());
    }

    #[test]
    fn bundled_outlines_fit_the_figures() {
        let c = Countries::bundled().unwrap();
        assert!(c.all().len() > 200);
        assert_eq!(code(c, 35.68, 139.69), Some("JPN")); // Tokyo
        assert_eq!(code(c, -35.28, 149.13), Some("AUS")); // Canberra
        assert_eq!(code(c, -15.79, -47.88), Some("BRA")); // Brasília
        assert_eq!(code(c, 64.84, -147.72), Some("USA")); // Fairbanks
        assert_eq!(code(c, 0.0, -30.0), None); // Atlantic
        // Every country with electricity figures has an outline, bar a few without one at 1:50m.
        let o = Owid::bundled().unwrap();
        let missing: Vec<&str> = o
            .countries()
            .iter()
            .filter(|k| k.code != "OWID_WRL" && k.year(crate::owid::Series::Generation, 2024).is_some() && c.get(&k.code).is_none())
            .map(|k| k.code.as_str())
            .collect();
        assert!(missing.len() <= 6, "{missing:?}");
    }

    #[test]
    fn rejects_other_files() {
        assert!(Countries::parse(b"ORBO\x01\0\0\0").is_none());
        assert!(Countries::parse(&FIXTURE[..FIXTURE.len() - 3]).is_none());
    }
}
