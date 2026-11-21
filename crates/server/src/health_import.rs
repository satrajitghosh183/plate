//! Past days from Apple Health's own export (Health app › your picture › Export All Health
//! Data). The file is `export.zip`, holding `apple_health_export/export.xml`, often hundreds
//! of megabytes, so it is read as a stream and never held in memory.
//!
//! Each `<Record>` of the five kinds below adds to its day. The day is the calendar date the
//! sample started on, in the phone's own time zone, which the export writes with every date.
//! When two sources recorded the same thing (the iPhone and the Watch both count active
//! energy), only the larger source's total counts, so nothing is counted twice.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::{Context, bail};
use chrono::NaiveDate;
use gym_core::api::HealthDay;
use quick_xml::Reader;
use quick_xml::events::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Kind {
    Eaten,
    Burned,
    Protein,
    Carbs,
    Fat,
}

fn kind_of(t: &[u8]) -> Option<Kind> {
    Some(match t {
        b"HKQuantityTypeIdentifierDietaryEnergyConsumed" => Kind::Eaten,
        b"HKQuantityTypeIdentifierActiveEnergyBurned" => Kind::Burned,
        b"HKQuantityTypeIdentifierDietaryProtein" => Kind::Protein,
        b"HKQuantityTypeIdentifierDietaryCarbohydrates" => Kind::Carbs,
        b"HKQuantityTypeIdentifierDietaryFatTotal" => Kind::Fat,
        _ => return None,
    })
}

/// Energy may be in kcal ("kcal", "Cal") or kJ; mass in g, mg or kg.
fn to_base(value: f64, unit: &str) -> Option<f64> {
    let v = match unit {
        "kcal" | "Cal" | "g" => value,
        "kJ" => value / 4.184,
        "mg" => value / 1000.0,
        "kg" => value * 1000.0,
        _ => return None,
    };
    v.is_finite().then_some(v)
}

/// What one export holds, day by day, plus how many samples were read.
#[derive(Debug, Default)]
pub struct Summary {
    pub days: BTreeMap<NaiveDate, HealthDay>,
    pub samples: u64,
}

/// Read an export.xml stream into day totals.
pub fn summarize<R: BufRead>(input: R) -> anyhow::Result<Summary> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    // (day, kind, source) → total
    let mut sums: HashMap<(NaiveDate, Kind, String), f64> = HashMap::new();
    let mut samples = 0u64;
    loop {
        match reader.read_event_into(&mut buf).context("The export is not valid XML")? {
            Event::Eof => break,
            Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"Record" => {
                let mut kind = None;
                let (mut unit, mut value, mut start, mut source) = (String::new(), None, None, String::new());
                for a in e.attributes().flatten() {
                    match a.key.as_ref() {
                        b"type" => kind = kind_of(&a.value),
                        b"unit" => unit = String::from_utf8_lossy(&a.value).into_owned(),
                        b"value" => value = std::str::from_utf8(&a.value).ok().and_then(|v| v.parse::<f64>().ok()),
                        // "2026-11-20 08:15:00 -0500": the first ten characters are the local date.
                        b"startDate" => start = std::str::from_utf8(&a.value).ok().and_then(|v| NaiveDate::parse_from_str(v.get(..10)?, "%Y-%m-%d").ok()),
                        b"sourceName" => source = String::from_utf8_lossy(&a.value).into_owned(),
                        _ => {}
                    }
                }
                let (Some(kind), Some(value), Some(day)) = (kind, value, start) else { continue };
                let Some(v) = to_base(value, &unit).filter(|v| *v >= 0.0) else { continue };
                *sums.entry((day, kind, source)).or_default() += v;
                samples += 1;
            }
            _ => {}
        }
        buf.clear();
    }
    // The larger source wins for each day and kind.
    let mut best: HashMap<(NaiveDate, Kind), f64> = HashMap::new();
    for ((day, kind, _), v) in sums {
        let e = best.entry((day, kind)).or_default();
        if v > *e {
            *e = v;
        }
    }
    let mut days: BTreeMap<NaiveDate, HealthDay> = BTreeMap::new();
    for ((day, kind), v) in best {
        let d = days.entry(day).or_insert_with(|| HealthDay { date: day, ..Default::default() });
        let v = Some((v * 10.0).round() / 10.0);
        match kind {
            Kind::Eaten => d.energy_eaten_kcal = v,
            Kind::Burned => d.energy_burned_kcal = v,
            Kind::Protein => d.protein_g = v,
            Kind::Carbs => d.carbs_g = v,
            Kind::Fat => d.fat_g = v,
        }
    }
    Ok(Summary { days, samples })
}

/// Open `export.zip` (or a bare `export.xml`) and summarize it.
pub fn read_file(path: &Path) -> anyhow::Result<Summary> {
    let file = std::fs::File::open(path).with_context(|| format!("Could not open {}", path.display()))?;
    let is_zip = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if !is_zip {
        return summarize(BufReader::with_capacity(1 << 20, file));
    }
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).context("That zip file could not be read")?;
    let name = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .find(|n| n.ends_with("export.xml") && !n.contains("cda"))
        .context("No export.xml inside. Use the file from Health › your picture › Export All Health Data")?;
    let entry = zip.by_name(&name)?;
    summarize(BufReader::with_capacity(1 << 20, entry.take(u64::MAX)))
}

/// Keep only days in [from, to] that hold something.
pub fn keep(summary: Summary, from: Option<NaiveDate>, to: NaiveDate) -> anyhow::Result<Vec<HealthDay>> {
    let days: Vec<HealthDay> = summary
        .days
        .into_values()
        .filter(|d| d.date <= to && from.is_none_or(|f| d.date >= f))
        .filter(|d| [d.energy_eaten_kcal, d.energy_burned_kcal, d.protein_g, d.carbs_g, d.fat_g].iter().any(|v| v.is_some_and(|v| v > 0.0)))
        .collect();
    if days.is_empty() {
        bail!("The export has no food or active energy in that range");
    }
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE HealthData [
<!ELEMENT HealthData (ExportDate,Me,(Record|Workout)*)>
]>
<HealthData locale="en_US">
 <ExportDate value="2026-11-20 21:00:00 -0500"/>
 <Record type="HKQuantityTypeIdentifierDietaryEnergyConsumed" sourceName="Lifesum" unit="kcal" startDate="2026-11-18 08:00:00 -0500" endDate="2026-11-18 08:00:00 -0500" value="600"/>
 <Record type="HKQuantityTypeIdentifierDietaryEnergyConsumed" sourceName="Lifesum" unit="kcal" startDate="2026-11-18 13:00:00 -0500" endDate="2026-11-18 13:00:00 -0500" value="900.5"/>
 <Record type="HKQuantityTypeIdentifierDietaryProtein" sourceName="Lifesum" unit="g" startDate="2026-11-18 13:00:00 -0500" endDate="2026-11-18 13:00:00 -0500" value="45"/>
 <Record type="HKQuantityTypeIdentifierDietaryCarbohydrates" sourceName="Lifesum" unit="g" startDate="2026-11-18 13:00:00 -0500" endDate="2026-11-18 13:00:00 -0500" value="120"/>
 <Record type="HKQuantityTypeIdentifierDietaryFatTotal" sourceName="Lifesum" unit="g" startDate="2026-11-18 13:00:00 -0500" endDate="2026-11-18 13:00:00 -0500" value="30"/>
 <Record type="HKQuantityTypeIdentifierActiveEnergyBurned" sourceName="iPhone" unit="kcal" startDate="2026-11-18 10:00:00 -0500" endDate="2026-11-18 10:05:00 -0500" value="100"/>
 <Record type="HKQuantityTypeIdentifierActiveEnergyBurned" sourceName="Apple Watch" unit="kcal" startDate="2026-11-18 10:00:00 -0500" endDate="2026-11-18 10:05:00 -0500" value="140">
  <MetadataEntry key="HKMetadataKeyHeartRateMotionContext" value="0"/>
 </Record>
 <Record type="HKQuantityTypeIdentifierActiveEnergyBurned" sourceName="Apple Watch" unit="kJ" startDate="2026-11-19 23:30:00 -0500" endDate="2026-11-19 23:40:00 -0500" value="418.4"/>
 <Record type="HKQuantityTypeIdentifierStepCount" sourceName="iPhone" unit="count" startDate="2026-11-18 10:00:00 -0500" endDate="2026-11-18 10:05:00 -0500" value="500"/>
 <Workout workoutActivityType="HKWorkoutActivityTypeTraditionalStrengthTraining" duration="54" durationUnit="min" startDate="2026-11-18 18:00:00 -0500" endDate="2026-11-18 18:54:00 -0500"/>
</HealthData>"#;

    #[test]
    fn days_from_an_apple_health_export() {
        let s = summarize(SAMPLE.as_bytes()).unwrap();
        let d = &s.days[&NaiveDate::from_ymd_opt(2026, 11, 18).unwrap()];
        assert_eq!(d.energy_eaten_kcal, Some(1500.5));
        assert_eq!((d.protein_g, d.carbs_g, d.fat_g), (Some(45.0), Some(120.0), Some(30.0)));
        assert_eq!(d.energy_burned_kcal, Some(140.0), "the watch and the phone both counted: the larger wins, never the sum");
        // kJ become kcal; a late-evening sample stays on its own local day.
        let late = &s.days[&NaiveDate::from_ymd_opt(2026, 11, 19).unwrap()];
        assert_eq!(late.energy_burned_kcal, Some(100.0));
        assert_eq!(s.samples, 8, "steps and workouts are not counted");
    }

    #[test]
    fn keeps_only_the_range_and_refuses_an_empty_one() {
        let s = summarize(SAMPLE.as_bytes()).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let days = keep(s, None, to).unwrap();
        assert_eq!(days.len(), 1);
        let s = summarize(SAMPLE.as_bytes()).unwrap();
        assert!(keep(s, Some(NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()), NaiveDate::from_ymd_opt(2027, 2, 1).unwrap()).is_err());
    }
}
