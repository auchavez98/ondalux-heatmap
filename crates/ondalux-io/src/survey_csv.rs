//! Survey measurements as CSV. Format: `docs/survey-csv.md`.
//!
//! One row per BSS heard at one point. Rows are grouped into measurements
//! by the `point` column if present, otherwise by identical coordinates.

use crate::error::IoError;
use ondalux_core::channel::{Band, Channel, ChannelWidth};
use ondalux_core::geometry::Vec2;
use ondalux_core::project::FloorPlan;
use ondalux_core::survey::{normalize_bssid, BssReading, Measurement, Security, Survey};
use ondalux_core::units::Dbm;
use std::collections::HashMap;
use std::io::{Read, Write};

/// Columns written by [`write_survey_csv`].
pub const HEADER: [&str; 9] = [
    "point",
    "x_m",
    "y_m",
    "timestamp_ms",
    "ssid",
    "bssid",
    "rssi_dbm",
    "frequency_mhz",
    "security",
];

#[derive(Debug, Clone, PartialEq)]
pub struct CsvImport {
    pub survey: Survey,
    /// Rows that were skipped, with the reason (1-based line numbers).
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct Columns {
    point: Option<usize>,
    x_m: Option<usize>,
    y_m: Option<usize>,
    x_px: Option<usize>,
    y_px: Option<usize>,
    timestamp: Option<usize>,
    ssid: Option<usize>,
    bssid: Option<usize>,
    rssi: Option<usize>,
    frequency: Option<usize>,
    channel: Option<usize>,
    band: Option<usize>,
    security: Option<usize>,
}

fn normalize_header(h: &str) -> String {
    h.trim()
        .trim_start_matches('\u{feff}')
        .to_ascii_lowercase()
        .replace([' ', '-', '(', ')'], "_")
        .trim_matches('_')
        .to_owned()
}

impl Columns {
    fn from_header(header: &csv::StringRecord) -> Self {
        let mut c = Columns::default();
        for (i, h) in header.iter().enumerate() {
            let slot = match normalize_header(h).as_str() {
                "point" | "point_id" | "measurement" => &mut c.point,
                "x_m" | "x" => &mut c.x_m,
                "y_m" | "y" => &mut c.y_m,
                "x_px" => &mut c.x_px,
                "y_px" => &mut c.y_px,
                "timestamp_ms" | "timestamp" | "time_ms" => &mut c.timestamp,
                "ssid" | "network" => &mut c.ssid,
                "bssid" | "mac" => &mut c.bssid,
                "rssi_dbm" | "rssi" | "signal_dbm" | "signal" => &mut c.rssi,
                "frequency_mhz" | "frequency" | "freq_mhz" | "freq" => &mut c.frequency,
                "channel" | "ch" => &mut c.channel,
                "band" => &mut c.band,
                "security" | "auth" | "encryption" => &mut c.security,
                _ => continue,
            };
            slot.get_or_insert(i);
        }
        c
    }
}

fn parse_band(s: &str) -> Option<Band> {
    let k: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    match k.replace(',', ".").as_str() {
        "2.4" | "24" | "2" => Some(Band::G2_4),
        "5" => Some(Band::G5),
        "6" => Some(Band::G6),
        _ => None,
    }
}

/// Reads a survey CSV. Positions in pixels (`x_px`, `y_px`) are converted
/// with the plan calibration.
pub fn read_survey_csv<R: Read>(
    reader: R,
    name: &str,
    plan: &FloorPlan,
) -> Result<CsvImport, IoError> {
    let mut data = String::new();
    let mut reader = reader;
    reader.read_to_string(&mut data)?;
    // Spreadsheets in many locales export `;`-separated files with decimal
    // commas. Pick the separator that appears more often in the header.
    let first = data.lines().next().unwrap_or_default();
    let semicolon = first.matches(';').count() > first.matches(',').count();
    let delimiter = if semicolon { b';' } else { b',' };
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(data.as_bytes());

    let header = rdr
        .headers()
        .map_err(|e| IoError::Format(format!("CSV header: {e}")))?
        .clone();
    let cols = Columns::from_header(&header);
    let has_m = cols.x_m.is_some() && cols.y_m.is_some();
    let has_px = cols.x_px.is_some() && cols.y_px.is_some();
    let mut missing = Vec::new();
    if !has_m && !has_px {
        missing.push("x_m and y_m (or x_px and y_px)");
    }
    if cols.bssid.is_none() {
        missing.push("bssid");
    }
    if cols.rssi.is_none() {
        missing.push("rssi_dbm");
    }
    if cols.frequency.is_none() && cols.channel.is_none() {
        missing.push("frequency_mhz or channel");
    }
    if !missing.is_empty() {
        return Err(IoError::Format(format!(
            "CSV is missing required columns: {}",
            missing.join(", ")
        )));
    }

    let num = |s: &str| -> Option<f64> {
        let s = s.trim();
        let s = if semicolon {
            s.replace(',', ".")
        } else {
            s.to_owned()
        };
        s.parse::<f64>().ok().filter(|v| v.is_finite())
    };

    let mut survey = Survey::new(name, format!("CSV: {name}"));
    let mut warnings = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();

    for (row_i, rec) in rdr.records().enumerate() {
        let line = row_i + 2;
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                warnings.push(format!("line {line}: {e}"));
                continue;
            }
        };
        let get = |c: Option<usize>| c.and_then(|i| rec.get(i)).unwrap_or("").trim();
        if rec.iter().all(|f| f.trim().is_empty()) {
            continue;
        }

        let pos = if has_m {
            num(get(cols.x_m))
                .zip(num(get(cols.y_m)))
                .map(|(x, y)| Vec2::new(x as f32, y as f32))
        } else {
            num(get(cols.x_px))
                .zip(num(get(cols.y_px)))
                .map(|(x, y)| plan.px_to_m(Vec2::new(x as f32, y as f32)))
        };
        let Some(position) = pos else {
            warnings.push(format!("line {line}: invalid position"));
            continue;
        };
        let Some(bssid) = normalize_bssid(get(cols.bssid)) else {
            warnings.push(format!("line {line}: invalid BSSID {:?}", get(cols.bssid)));
            continue;
        };
        let Some(rssi) = num(get(cols.rssi)).filter(|v| (-130.0..=10.0).contains(v)) else {
            warnings.push(format!("line {line}: invalid RSSI {:?}", get(cols.rssi)));
            continue;
        };
        let frequency = num(get(cols.frequency)).map(|f| f as u32).or_else(|| {
            let ch = num(get(cols.channel))? as u16;
            let band =
                parse_band(get(cols.band)).unwrap_or(if ch <= 14 { Band::G2_4 } else { Band::G5 });
            let c = Channel::new(band, ch, ChannelWidth::Mhz20).ok()?;
            Some(c.primary_center_mhz() as u32)
        });
        let Some(frequency_mhz) = frequency else {
            warnings.push(format!("line {line}: invalid frequency/channel"));
            continue;
        };
        let reading = BssReading {
            bssid,
            ssid: get(cols.ssid).to_owned(),
            rssi: Dbm(rssi as f32),
            frequency_mhz,
            security: cols
                .security
                .map_or(Security::Unknown, |_| Security::parse(get(cols.security))),
        };

        let key = match cols.point {
            Some(_) if !get(cols.point).is_empty() => format!("p:{}", get(cols.point)),
            _ => format!("xy:{:.3}:{:.3}", position.x, position.y),
        };
        let idx = *by_key.entry(key).or_insert_with(|| {
            survey.measurements.push(Measurement {
                id: survey.measurements.len() as u32 + 1,
                position,
                timestamp_ms: num(get(cols.timestamp)).map_or(0, |t| t as i64),
                readings: Vec::new(),
            });
            survey.measurements.len() - 1
        });
        let m = &mut survey.measurements[idx];
        // Duplicate BSSID at one point: keep the strongest reading.
        match m.readings.iter_mut().find(|r| r.bssid == reading.bssid) {
            Some(r) if r.rssi.0 < reading.rssi.0 => *r = reading,
            Some(_) => {}
            None => m.readings.push(reading),
        }
    }
    if survey.measurements.is_empty() {
        return Err(IoError::Format(format!(
            "no valid rows in CSV ({} skipped)",
            warnings.len()
        )));
    }
    Ok(CsvImport { survey, warnings })
}

pub fn write_survey_csv<W: Write>(writer: W, survey: &Survey) -> Result<(), IoError> {
    let mut w = csv::Writer::from_writer(writer);
    let err = |e: csv::Error| IoError::Format(format!("CSV: {e}"));
    w.write_record(HEADER).map_err(err)?;
    for m in &survey.measurements {
        for r in &m.readings {
            w.write_record([
                m.id.to_string(),
                format!("{:.3}", m.position.x),
                format!("{:.3}", m.position.y),
                m.timestamp_ms.to_string(),
                r.ssid.clone(),
                r.bssid.clone(),
                format!("{:.0}", r.rssi.0),
                r.frequency_mhz.to_string(),
                r.security.label().to_owned(),
            ])
            .map_err(err)?;
        }
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ondalux_core::project::Calibration;

    fn plan() -> FloorPlan {
        FloorPlan {
            width_px: 1000,
            height_px: 500,
            calibration: Calibration::new(Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0), 1.0),
        }
    }

    #[test]
    fn reads_comma_separated_with_frequency() {
        let csv = "x_m,y_m,ssid,bssid,rssi_dbm,frequency_mhz,security\n\
                   1.5,2,Corp,AA-BB-CC-00-11-22,-61,5180,WPA2-Personal\n\
                   1.5,2,Guest,aa:bb:cc:00:11:23,-63,5180,open\n\
                   4,2,Corp,aa:bb:cc:00:11:22,-70,5180,WPA2-Personal\n";
        let r = read_survey_csv(csv.as_bytes(), "walk.csv", &plan()).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let s = r.survey;
        assert_eq!(s.measurements.len(), 2);
        assert_eq!(s.measurements[0].readings.len(), 2);
        assert_eq!(s.measurements[0].position, Vec2::new(1.5, 2.0));
        assert_eq!(s.measurements[0].readings[0].bssid, "aa:bb:cc:00:11:22");
        assert_eq!(s.measurements[0].readings[1].security, Security::Open);
        assert_eq!(s.source, "CSV: walk.csv");
    }

    #[test]
    fn reads_semicolons_decimal_commas_channels_and_pixels() {
        let csv = "Point;X_px;Y_px;BSSID;RSSI;Channel;Band\n\
                   p1;100;50;aabbcc001122;-55,5;36;5\n\
                   p1;100;50;aabbcc001133;-70;6;\n\
                   p2;150;50;aabbcc001122;-60;37;6 GHz\n";
        let r = read_survey_csv(csv.as_bytes(), "x", &plan()).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let s = r.survey;
        assert_eq!(s.measurements.len(), 2);
        // 50 px/m calibration.
        assert_eq!(s.measurements[0].position, Vec2::new(2.0, 1.0));
        let rs = &s.measurements[0].readings;
        assert_eq!(rs[0].rssi, Dbm(-55.5));
        assert_eq!(rs[0].frequency_mhz, 5180);
        assert_eq!(rs[1].frequency_mhz, 2437);
        assert_eq!(s.measurements[1].readings[0].frequency_mhz, 6135);
    }

    #[test]
    fn bad_rows_become_warnings() {
        let csv = "x_m,y_m,bssid,rssi_dbm,channel\n\
                   1,1,aa:bb:cc:00:11:22,-60,6\n\
                   1,1,not-a-mac,-60,6\n\
                   1,1,aa:bb:cc:00:11:23,loud,6\n\
                   ,1,aa:bb:cc:00:11:24,-60,6\n\
                   1,1,aa:bb:cc:00:11:25,-60,99\n";
        let r = read_survey_csv(csv.as_bytes(), "x", &plan()).unwrap();
        assert_eq!(r.survey.measurements[0].readings.len(), 1);
        assert_eq!(r.warnings.len(), 4);
        assert!(r.warnings[0].starts_with("line 3"), "{:?}", r.warnings);
    }

    #[test]
    fn missing_columns_are_reported() {
        let err = read_survey_csv("ssid,rssi\nx,-50\n".as_bytes(), "x", &plan()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bssid") && msg.contains("x_m"), "{msg}");
    }

    #[test]
    fn write_then_read_round_trip() {
        let csv = "x_m,y_m,ssid,bssid,rssi_dbm,frequency_mhz,security\n\
                   1.5,2,Corp,aa:bb:cc:00:11:22,-61,5180,WPA3-Personal\n\
                   4,2,Corp,aa:bb:cc:00:11:22,-70,2412,Enterprise\n";
        let original = read_survey_csv(csv.as_bytes(), "a", &plan())
            .unwrap()
            .survey;
        let mut out = Vec::new();
        write_survey_csv(&mut out, &original).unwrap();
        let back = read_survey_csv(out.as_slice(), "a", &plan())
            .unwrap()
            .survey;
        assert_eq!(back.measurements, original.measurements);
    }
}
