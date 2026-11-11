//! Brings SMARD's quarter hours to Ember's monthly level for Germany. SMARD's public net
//! generation leaves out what plants make for their owners' own use (rooftop solar, industry);
//! Ember's monthly figures, from Energy-Charts and Agora Energiewende, include it. Each series is
//! multiplied by a factor per month, so that its month adds up to Ember's while every quarter
//! hour keeps SMARD's shape. The carbon intensity from SMARD's mix is calibrated the same way to
//! Ember's monthly average. For a month Ember has not published yet, the same month of the latest
//! year it has stands in.

use crate::owid::{Country, Series};
use crate::power_feed::{mix_intensity, owid_series, Year, FILTERS};

/// How many years back a month of the same season is looked for.
const SEASONS_BACK: i32 = 3;

/// The factors in effect for a month.
#[derive(Clone, Debug, PartialEq)]
pub struct Factors {
    /// The month shown, (year, 1 to 12).
    pub month: (i32, u32),
    /// The month whose factors these are: `month`, or the same month of an earlier year.
    pub from: (i32, u32),
    /// Per series of `FILTERS`, Ember's month against SMARD's; 1 where Ember has no such figure.
    pub output: [f32; FILTERS.len()],
    /// Ember's monthly carbon intensity against SMARD's mix by `power_feed::EMISSIONS`.
    pub intensity: Option<f32>,
}

impl Factors {
    /// Whether the factors are those of an earlier year's month.
    pub fn borrowed(&self) -> bool {
        self.from != self.month
    }
}

/// The month whose factors stand for `month`: itself if Ember has it, else the same month of the
/// latest earlier year that Ember has.
pub fn source_month(ember: &Country, month: (i32, u32)) -> Option<(i32, u32)> {
    (0..=SEASONS_BACK)
        .map(|back| (month.0 - back, month.1))
        .find(|&(y, m)| ember.month(Series::Generation, y, m).is_some())
}

/// The factors from Ember's figures for a month and SMARD's sums for it, MWh per series of
/// `FILTERS`. Series sharing one of Ember's sources share its factor; consumption is brought to
/// Ember's demand.
pub fn factors_for(ember: &Country, from: (i32, u32), smard_mwh: &[f64]) -> [f32; FILTERS.len()] {
    let mut out = [1.0f32; FILTERS.len()];
    let (y, m) = from;
    for (i, factor) in out.iter_mut().enumerate() {
        let target = match owid_series(i) {
            Some(s) => Some(s),
            None if i == FILTERS.len() - 1 => Some(Series::Demand),
            None => None,
        };
        let Some(target) = target else { continue };
        let Some(twh) = ember.month(target, y, m) else {
            continue;
        };
        let smard: f64 = (0..FILTERS.len())
            .filter(|&j| {
                if target == Series::Demand {
                    j == i
                } else {
                    owid_series(j) == Some(target)
                }
            })
            .map(|j| smard_mwh[j])
            .sum();
        if smard > 0.0 && twh > 0.0 {
            *factor = (twh * 1e6 / smard) as f32;
        }
    }
    out
}

/// Ember's monthly carbon intensity against that of SMARD's month by `EMISSIONS`.
pub fn intensity_factor(ember: &Country, from: (i32, u32), smard_mwh: &[f64]) -> Option<f32> {
    let target = ember.month(Series::CarbonIntensity, from.0, from.1)?;
    let own = mix_intensity(smard_mwh)?;
    (own > 0.0).then(|| (target / own) as f32)
}

/// The factors for `month`, if Ember has it or the same month of an earlier year, and `year`
/// holds SMARD's figures for the month they come from.
pub fn factors(ember: &Country, month: (i32, u32), year: &Year) -> Option<Factors> {
    let from = source_month(ember, month)?;
    if year.year != from.0 {
        return None;
    }
    let smard = year.month(from.1)?;
    Some(Factors {
        month,
        from,
        output: factors_for(ember, from, &smard),
        intensity: intensity_factor(ember, from, &smard),
    })
}

/// A month in words, such as "November 2025".
pub fn month_words((year, month): (i32, u32)) -> String {
    crate::energy::month_name(((year - 1900) * 12 + month as i32 - 1).max(0) as u16)
}

/// The renewable share of a year in percent, and how many months it covers: Ember's yearly figure
/// where it has one, else the sum of its months so far (`None` for months if it is the whole
/// year). With the change in points against the year before, or against the same months of it.
#[derive(Clone, Debug, PartialEq)]
pub struct YearShare {
    pub year: i32,
    /// The last month counted, if the year is not yet complete.
    pub through: Option<u32>,
    pub share: f64,
    pub change: Option<f64>,
}

const RENEWABLES: [Series; 5] = [
    Series::Hydro,
    Series::Wind,
    Series::Solar,
    Series::Bioenergy,
    Series::OtherRenewables,
];

/// Ember's renewable share over the first `months` months of a year, if it has all of them.
fn months_share(ember: &Country, year: i32, months: u32) -> Option<f64> {
    let mut renewable = 0.0;
    let mut total = 0.0;
    for m in 1..=months {
        total += ember.month(Series::Generation, year, m)?;
        for s in RENEWABLES {
            renewable += ember.month(s, year, m).unwrap_or(0.0);
        }
    }
    (total > 0.0).then(|| 100.0 * renewable / total)
}

/// The year at a glance for the clock's year.
pub fn year_share(ember: &Country, year: i32) -> Option<YearShare> {
    if let Some(share) = ember.year(Series::RenewableShare, year) {
        let change = ember
            .year(Series::RenewableShare, year - 1)
            .map(|before| share - before);
        return Some(YearShare {
            year,
            through: None,
            share,
            change,
        });
    }
    let ((y, through), _) = ember.latest_month(Series::Generation, year, 12)?;
    if y != year {
        // Nothing of this year yet: the latest year there is.
        let (latest, _) = ember.latest_year(Series::RenewableShare, year)?;
        return (latest < year).then(|| year_share(ember, latest)).flatten();
    }
    let share = months_share(ember, year, through)?;
    let change = months_share(ember, year - 1, through).map(|before| share - before);
    Some(YearShare {
        year,
        through: (through < 12).then_some(through),
        share,
        change,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owid::Owid;
    use crate::power_feed::{Moment, EMISSIONS};

    fn germany() -> &'static Country {
        Owid::bundled().unwrap().country("DEU").unwrap()
    }

    /// SMARD's month: solar 5 TWh, wind 10 TWh in two series, lignite 6 TWh, the rest 1 TWh
    /// each, nuclear none.
    fn smard_month() -> Vec<f64> {
        let mut mwh = vec![1e6; FILTERS.len()];
        mwh[0] = 5e6;
        mwh[1] = 8e6;
        mwh[2] = 2e6;
        mwh[5] = 6e6;
        mwh[9] = 0.0;
        mwh
    }

    #[test]
    fn scaling_keeps_the_shape_and_hits_the_monthly_sum() {
        let ember = germany();
        let from = (2025, 6);
        let smard = smard_month();
        let f = factors_for(ember, from, &smard);
        // A month of quarter hours whose sums are `smard`, with a different shape per series.
        let quarters = 30 * 96;
        let shape = |i: usize, q: usize| 1.0 + ((q * (i + 3)) % 17) as f64;
        let norm: Vec<f64> = (0..FILTERS.len())
            .map(|i| (0..quarters).map(|q| shape(i, q)).sum())
            .collect();
        let mut sums = vec![0.0f64; FILTERS.len()];
        for q in 0..quarters {
            let mw: Vec<f32> = (0..FILTERS.len())
                .map(|i| (smard[i] * shape(i, q) / norm[i] * 4.0) as f32)
                .collect();
            let raw = Moment { time: 0.0, mw };
            let scaled = raw.scaled(&f);
            for i in 0..FILTERS.len() {
                sums[i] += scaled.mw[i] as f64 / 4.0;
                if raw.mw[i] > 0.0 {
                    assert!(
                        (scaled.mw[i] / raw.mw[i] - f[i]).abs() < 1e-4,
                        "the shape stays"
                    );
                }
            }
        }
        let twh = |s: Series| ember.month(s, from.0, from.1).unwrap();
        let close = |a: f64, b: f64| (a / b - 1.0).abs() < 1e-4;
        assert!(close(sums[0] / 1e6, twh(Series::Solar)));
        assert!(
            close((sums[1] + sums[2]) / 1e6, twh(Series::Wind)),
            "wind's two series share the factor"
        );
        assert_eq!(f[1], f[2]);
        assert!(close((sums[5] + sums[6]) / 1e6, twh(Series::Coal)));
        assert!(close(sums[12] / 1e6, twh(Series::Demand)));
        assert_eq!(f[9], 1.0, "nothing to scale without nuclear");
        assert_eq!(f[11], 1.0, "pumped storage is not Ember's");
    }

    #[test]
    fn months_ember_has_not_published_borrow_the_same_month_a_year_earlier() {
        let ember = germany();
        let ((ly, lm), _) = ember.latest_month(Series::Generation, 2100, 12).unwrap();
        assert!((ly, lm) >= (2026, 6), "Ember's months reach mid-2026");
        assert_eq!(source_month(ember, (2025, 11)), Some((2025, 11)));
        let later = if lm == 12 { (ly + 1, 1) } else { (ly, lm + 1) };
        assert_eq!(source_month(ember, later), Some((later.0 - 1, later.1)));
        assert_eq!(source_month(ember, (1990, 1)), None);
        let mut year = Year {
            year: later.0 - 1,
            mwh: smard_month().iter().map(|&v| [Some(v); 12]).collect(),
        };
        let f = factors(ember, later, &year).unwrap();
        assert!(f.borrowed());
        assert_eq!(f.from, (later.0 - 1, later.1));
        assert_eq!(f.output, factors_for(ember, f.from, &smard_month()));
        year.year = later.0;
        assert!(
            factors(ember, later, &year).is_none(),
            "the factors need SMARD's months of the year they come from"
        );
    }

    #[test]
    fn intensity_of_an_all_wind_and_an_all_lignite_quarter_hour() {
        let alone = |i: usize| {
            let mut mw = vec![0.0; FILTERS.len()];
            mw[i] = 10_000.0;
            mw[FILTERS.len() - 1] = 50_000.0;
            Moment { time: 0.0, mw }.intensity()
        };
        assert_eq!(alone(1), Some(11.0));
        assert_eq!(alone(2), Some(12.0));
        assert_eq!(alone(5), Some(820.0));
        assert_eq!(alone(11), None, "pumped storage alone is no mix");
        let mut mw = vec![0.0; FILTERS.len()];
        mw[1] = 3000.0;
        mw[5] = 1000.0;
        mw[11] = 5000.0;
        let m = Moment { time: 0.0, mw };
        assert_eq!(
            m.intensity().unwrap(),
            (3.0 * EMISSIONS[1] + EMISSIONS[5]) / 4.0,
            "pumped storage is left out"
        );
    }

    #[test]
    fn calibration_brings_the_monthly_average_to_embers() {
        let ember = germany();
        let from = (2025, 3);
        let smard = smard_month();
        let k = intensity_factor(ember, from, &smard).unwrap();
        let calibrated = k as f64 * mix_intensity(&smard).unwrap();
        let target = ember
            .month(Series::CarbonIntensity, from.0, from.1)
            .unwrap();
        assert!((calibrated / target - 1.0).abs() < 1e-5);
        // The month's average over its quarter hours, weighted by generation, is the month's mix.
        let a = [1.0f64, 3.0];
        let quarters: Vec<Vec<f64>> = a
            .iter()
            .map(|&w| {
                smard
                    .iter()
                    .enumerate()
                    .map(|(i, v)| v * if i == 5 { w } else { 1.0 })
                    .collect()
            })
            .collect();
        let gen = |q: &Vec<f64>| {
            (0..FILTERS.len())
                .filter(|&i| owid_series(i).is_some())
                .map(|i| q[i])
                .sum::<f64>()
        };
        let avg = quarters
            .iter()
            .map(|q| mix_intensity(q).unwrap() * gen(q))
            .sum::<f64>()
            / quarters.iter().map(gen).sum::<f64>();
        let summed: Vec<f64> = (0..FILTERS.len())
            .map(|i| quarters.iter().map(|q| q[i]).sum())
            .collect();
        assert!((avg / mix_intensity(&summed).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_year_at_a_glance() {
        let ember = germany();
        let y = year_share(ember, 2025).unwrap();
        assert_eq!((y.year, y.through), (2025, None));
        assert!((y.share - 59.1).abs() < 0.1, "{}", y.share);
        assert!(y.change.is_some());
        let y = year_share(ember, 2026).unwrap();
        assert_eq!(y.year, 2026);
        assert!(y.through.is_some_and(|m| m >= 6));
        assert!(y.change.is_some());
        assert!(year_share(ember, 1900).is_none());
    }
}
