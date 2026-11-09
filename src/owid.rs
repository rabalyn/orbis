//! Electricity of the world's countries by year and, where Ember has it, by month, from Our World
//! in Data. `tools/gen_owid.py` turns OWID's energy dataset and the monthly series of its
//! `electricity-mix` chart into `data/owid_energy.bin`; this module reads it. Nothing is requested
//! at runtime. Country outlines are in `countries.rs`.
//!
//! Countries are keyed by ISO 3166 alpha-3, the World by `OWID_WRL` and Kosovo by `OWID_KOS`.
//! Electricity is in TWh, population in people, carbon intensity in g CO₂-eq per kWh and the
//! renewable share in percent.

// Nothing draws this yet; the countries level of the energy layer will.
#![cfg_attr(not(test), allow(dead_code))]

/// The credit while OWID's figures are in view.
pub const CREDIT: &str = "Electricity of countries: Our World in Data (CC BY 4.0), from Ember (CC BY 4.0) and the Energy Institute Statistical Review of World Energy";

/// A figure of the file. Population and the renewable share are yearly only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Series {
    Population,
    Generation,
    Demand,
    NetImports,
    CarbonIntensity,
    RenewableShare,
    Coal,
    Gas,
    Oil,
    Nuclear,
    Hydro,
    Wind,
    Solar,
    Bioenergy,
    OtherRenewables,
}

impl Series {
    pub const ALL: [Series; 15] = [
        Series::Population,
        Series::Generation,
        Series::Demand,
        Series::NetImports,
        Series::CarbonIntensity,
        Series::RenewableShare,
        Series::Coal,
        Series::Gas,
        Series::Oil,
        Series::Nuclear,
        Series::Hydro,
        Series::Wind,
        Series::Solar,
        Series::Bioenergy,
        Series::OtherRenewables,
    ];

    /// Its key in the file.
    pub fn key(self) -> &'static str {
        match self {
            Series::Population => "population",
            Series::Generation => "generation",
            Series::Demand => "demand",
            Series::NetImports => "net_imports",
            Series::CarbonIntensity => "carbon_intensity",
            Series::RenewableShare => "renewable_share",
            Series::Coal => "coal",
            Series::Gas => "gas",
            Series::Oil => "oil",
            Series::Nuclear => "nuclear",
            Series::Hydro => "hydro",
            Series::Wind => "wind",
            Series::Solar => "solar",
            Series::Bioenergy => "bioenergy",
            Series::OtherRenewables => "other_renewables",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Series::Population => "people",
            Series::CarbonIntensity => "g/kWh",
            Series::RenewableShare => "%",
            _ => "TWh",
        }
    }

    fn from_key(key: &str) -> Option<Series> {
        Series::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// Where a figure comes from, as OWID names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Ember's yearly or monthly electricity data.
    Ember,
    /// The Energy Institute's Statistical Review of World Energy, for years before Ember's.
    EnergyInstitute,
    /// OWID's population series, mostly the UN's World Population Prospects.
    Population,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Ember => "Ember",
            Source::EnergyInstitute => "Energy Institute",
            Source::Population => "Our World in Data (UN and others)",
        }
    }
}

/// Values of one series at increasing times (years, or months as year * 12 + month - 1).
#[derive(Default, Debug)]
struct Points {
    times: Vec<i32>,
    values: Vec<f64>,
}

impl Points {
    fn at(&self, t: i32) -> Option<f64> {
        self.times.binary_search(&t).ok().map(|i| self.values[i])
    }
    fn latest(&self, t: i32) -> Option<(i32, f64)> {
        let i = self.times.partition_point(|&x| x <= t);
        (i > 0).then(|| (self.times[i - 1], self.values[i - 1]))
    }
}

pub struct Country {
    pub code: String,
    /// OWID's English name.
    pub name: String,
    /// First year of Ember's figures; 0 if it has none.
    ember_from: i32,
    yearly: Vec<Points>,
    monthly: Vec<Points>,
}

impl Country {
    fn series(list: &[Points], s: Series) -> Option<&Points> {
        list.get(s as usize).filter(|p| !p.times.is_empty())
    }

    /// The figure for a year.
    pub fn year(&self, s: Series, year: i32) -> Option<f64> {
        Self::series(&self.yearly, s)?.at(year)
    }

    /// The latest yearly figure at or before a year, with its year.
    pub fn latest_year(&self, s: Series, year: i32) -> Option<(i32, f64)> {
        Self::series(&self.yearly, s)?.latest(year)
    }

    /// The figure for a month (1 to 12).
    pub fn month(&self, s: Series, year: i32, month: u32) -> Option<f64> {
        Self::series(&self.monthly, s)?.at(month_index(year, month))
    }

    /// The latest monthly figure at or before a month, with its year and month.
    pub fn latest_month(&self, s: Series, year: i32, month: u32) -> Option<((i32, u32), f64)> {
        let (t, v) = Self::series(&self.monthly, s)?.latest(month_index(year, month))?;
        Some(((t.div_euclid(12), t.rem_euclid(12) as u32 + 1), v))
    }

    /// The first and last year of a yearly series.
    pub fn years(&self, s: Series) -> Option<(i32, i32)> {
        let p = Self::series(&self.yearly, s)?;
        Some((p.times[0], *p.times.last()?))
    }

    /// Whether Ember publishes monthly figures for the country.
    pub fn has_monthly(&self) -> bool {
        Self::series(&self.monthly, Series::Generation).is_some()
    }

    /// Where a yearly figure comes from. OWID takes Ember's where it has them and the Energy
    /// Institute's before; demand and carbon intensity are Ember's alone.
    pub fn year_source(&self, s: Series, year: i32) -> Source {
        match s {
            Series::Population => Source::Population,
            _ if self.ember_from > 0 && year >= self.ember_from => Source::Ember,
            Series::Demand | Series::CarbonIntensity | Series::NetImports => Source::Ember,
            _ => Source::EnergyInstitute,
        }
    }

    /// Where a monthly figure comes from.
    pub fn month_source(&self, _s: Series) -> Source {
        Source::Ember
    }
}

fn month_index(year: i32, month: u32) -> i32 {
    year * 12 + month.clamp(1, 12) as i32 - 1
}

/// Reads the files `gen_owid.py` writes.
pub(crate) struct Reader<'a> {
    pub data: &'a [u8],
    pub at: usize,
}

impl<'a> Reader<'a> {
    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.data.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(b)
    }
    pub fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    pub fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.bytes(2)?.try_into().ok()?))
    }
    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    pub fn varint(&mut self) -> Option<i64> {
        let mut v: u64 = 0;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Some(((v >> 1) as i64) ^ -((v & 1) as i64));
            }
        }
        None
    }
    pub fn string(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        String::from_utf8(self.bytes(n)?.to_vec()).ok()
    }
    fn points(&mut self, scale: f64) -> Option<Points> {
        let n = usize::try_from(self.varint()?).ok()?;
        let mut p = Points {
            times: Vec::with_capacity(n.min(1 << 12)),
            values: Vec::with_capacity(n.min(1 << 12)),
        };
        let (mut t, mut v) = (0i64, 0i64);
        for _ in 0..n {
            t += self.varint()?;
            v += self.varint()?;
            p.times.push(i32::try_from(t).ok()?);
            p.values.push(v as f64 / scale);
        }
        Some(p)
    }
}

const MAGIC: &[u8; 8] = b"ORBO\x01\0\0\0";

pub struct Owid {
    /// Date the data was retrieved, YYYYMMDD.
    pub retrieved: u32,
    /// Sorted by code.
    countries: Vec<Country>,
}

impl Owid {
    /// The data built into the program.
    pub fn bundled() -> Option<&'static Owid> {
        static DATA: std::sync::OnceLock<Option<Owid>> = std::sync::OnceLock::new();
        DATA.get_or_init(|| Owid::parse(include_bytes!("../data/owid_energy.bin"))).as_ref()
    }

    pub fn parse(data: &[u8]) -> Option<Owid> {
        let mut r = Reader { data, at: 0 };
        if r.bytes(8)? != MAGIC {
            return None;
        }
        let retrieved = r.u32()?;
        let (ny, nm) = (r.u8()? as usize, r.u8()? as usize);
        // Where each series of the file goes (None for keys this program does not know), and its scale.
        let mut table = Vec::with_capacity(ny + nm);
        for _ in 0..ny + nm {
            let key = r.string()?;
            let decimals = r.u8()?;
            table.push((Series::from_key(&key), 10f64.powi(decimals.into())));
        }
        let n = r.u16()? as usize;
        let mut countries = Vec::with_capacity(n);
        for _ in 0..n {
            let code = r.string()?;
            let name = r.string()?;
            let ember_from = r.u16()?.into();
            let mut yearly: Vec<Points> = Series::ALL.iter().map(|_| Points::default()).collect();
            let mut monthly: Vec<Points> = Series::ALL.iter().map(|_| Points::default()).collect();
            for (k, &(series, scale)) in table.iter().enumerate() {
                let p = r.points(scale)?;
                let list = if k < ny { &mut yearly } else { &mut monthly };
                if let Some(s) = series {
                    list[s as usize] = p;
                }
            }
            countries.push(Country { code, name, ember_from, yearly, monthly });
        }
        countries.sort_by(|a, b| a.code.cmp(&b.code));
        Some(Owid { retrieved, countries })
    }

    pub fn country(&self, code: &str) -> Option<&Country> {
        let i = self.countries.binary_search_by(|c| c.code.as_str().cmp(code)).ok()?;
        Some(&self.countries[i])
    }

    pub fn countries(&self) -> &[Country] {
        &self.countries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/owid/owid_energy.bin");

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-6 * b.abs().max(1.0))
    }

    fn germany_2025(o: &Owid) {
        let de = o.country("DEU").unwrap();
        assert_eq!(de.name, "Germany");
        assert!(close(de.year(Series::Generation, 2025), 500.47));
        assert!(close(de.year(Series::Demand, 2025), 520.27));
        assert!((de.year(Series::RenewableShare, 2025).unwrap() - 59.09).abs() < 0.005);
        assert!(close(de.year(Series::CarbonIntensity, 2025), 329.65));
        assert!(close(de.year(Series::NetImports, 2025), 19.8));
        assert!(close(de.year(Series::Population, 2025), 84_075_075.0));
        // The sources sum to the total, and the renewable ones to the share.
        let sum = |list: &[Series]| list.iter().map(|&s| de.year(s, 2025).unwrap()).sum::<f64>();
        let renewable = sum(&[Series::Hydro, Series::Wind, Series::Solar, Series::Bioenergy, Series::OtherRenewables]);
        let all = renewable + sum(&[Series::Coal, Series::Gas, Series::Oil, Series::Nuclear]);
        assert!((all - 500.47).abs() < 0.05, "{all}");
        assert!((renewable / all * 100.0 - 59.09).abs() < 0.05);
        assert_eq!(de.year_source(Series::Generation, 2025), Source::Ember);
        assert!(de.has_monthly());
    }

    #[test]
    fn germany_2025_in_fixture_and_bundled() {
        germany_2025(&Owid::parse(FIXTURE).unwrap());
        germany_2025(Owid::bundled().unwrap());
    }

    #[test]
    fn bundled_has_the_countries_and_the_world() {
        let o = Owid::bundled().unwrap();
        assert!(o.countries().len() > 200);
        assert!(o.countries().iter().all(|c| c.code.len() == 3 || c.code.starts_with("OWID_")));
        assert!(o.country("OWID_WRL").unwrap().year(Series::Generation, 2025).unwrap() > 30_000.0);
        assert!(o.country("OWID_EUR").is_none());
        assert!(o.countries().iter().filter(|c| c.has_monthly()).count() > 80);
        assert!(o.retrieved >= 20261109);
    }

    #[test]
    fn months() {
        let o = Owid::parse(FIXTURE).unwrap();
        let de = o.country("DEU").unwrap();
        // Ember's monthly figures for Germany run from January 2015.
        assert!(de.month(Series::Generation, 2015, 1).is_some());
        assert_eq!(de.latest_month(Series::Generation, 2014, 12), None);
        assert_eq!(de.month(Series::Generation, 2014, 12), None);
        let ((y, m), v) = de.latest_month(Series::Generation, 2030, 1).unwrap();
        assert!(y > 2025 && (1..=12).contains(&m) && v > 20.0);
        assert_eq!(de.latest_month(Series::Generation, y, m).unwrap().0, (y, m));
        // Ember's monthly series for Germany sums to about 6% less than its yearly one (470.6
        // against 500.5 TWh in 2025); milestone 3 looks into why.
        let year: f64 = (1..=12).map(|m| de.month(Series::Generation, 2025, m).unwrap()).sum();
        assert!((year - 470.58).abs() < 0.5, "{year}");
        assert!(de.month(Series::Demand, 2025, 1).unwrap() > de.month(Series::Demand, 2025, 7).unwrap());
        assert!(de.month(Series::Solar, 2025, 6).unwrap() > de.month(Series::Solar, 2025, 12).unwrap());
        assert_eq!(de.month(Series::Population, 2025, 1), None);
        assert_eq!(de.month_source(Series::Solar), Source::Ember);
    }

    #[test]
    fn annual_without_monthly() {
        let o = Owid::parse(FIXTURE).unwrap();
        let af = o.country("AFG").unwrap();
        assert!(!af.has_monthly());
        assert!(af.year(Series::Generation, 2024).is_some());
        assert_eq!(af.latest_month(Series::Generation, 2025, 6), None);
        // Kosovo, which has no ISO code of its own, is there by OWID's code.
        assert!(o.country("OWID_KOS").unwrap().year(Series::Demand, 2025).is_some());
    }

    #[test]
    fn years_before_the_first() {
        let o = Owid::parse(FIXTURE).unwrap();
        let de = o.country("DEU").unwrap();
        let (first, last) = de.years(Series::Generation).unwrap();
        assert_eq!(first, 1985);
        assert!(last >= 2025);
        assert_eq!(de.year(Series::Generation, 1984), None);
        assert_eq!(de.latest_year(Series::Generation, 1984), None);
        assert_eq!(de.latest_year(Series::Generation, 1985).unwrap().0, 1985);
        assert_eq!(de.latest_year(Series::Generation, 2100).unwrap().0, last);
        assert_eq!(de.latest_year(Series::Demand, 1989), None);
        // Before Ember's figures begin, OWID's generation is the Energy Institute's.
        assert_eq!(de.year_source(Series::Generation, 1985), Source::EnergyInstitute);
        assert_eq!(de.year_source(Series::Generation, 1990), Source::Ember);
        assert_eq!(de.year_source(Series::Population, 2025), Source::Population);
    }

    #[test]
    fn units_sources_and_credit() {
        assert_eq!(Series::Solar.unit(), "TWh");
        assert_eq!(Series::CarbonIntensity.unit(), "g/kWh");
        assert_eq!(Source::EnergyInstitute.name(), "Energy Institute");
        assert!(CREDIT.contains("Our World in Data") && CREDIT.contains("Ember"));
        assert!(Series::ALL.iter().all(|&s| Series::from_key(s.key()) == Some(s)));
    }

    #[test]
    fn rejects_other_files() {
        assert!(Owid::parse(b"ORBE\x01\0\0\0").is_none());
        assert!(Owid::parse(&FIXTURE[..FIXTURE.len() - 1]).is_none());
    }
}
