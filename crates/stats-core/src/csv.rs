//! History and events as CSV, for Numbers and Excel (#45).
//!
//! The files follow the German format of 3D Stats, so Numbers and Excel on
//! German devices open them in columns right away: a byte order mark,
//! semicolons, a decimal comma and times as `YYYY-MM-DD HH:MM` in the time
//! zone of the device. Values stay in their base units (bytes, bytes per
//! second, percent, degrees Celsius), as in the archive.
//!
//! Text from the NAS (keys, subjects, texts) can start with `=`, `+`, `-`
//! or `@`; such a field gets a leading `'`, so a spreadsheet does not run
//! it as a formula. A field with a semicolon, quote, line break or `'` is
//! quoted, with inner quotes doubled.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use nas_model::{Health, Metric};

use crate::history::{EventRecord, Series};

/// Byte order mark, so Excel reads the file as UTF-8.
const BOM: &str = "\u{FEFF}";

const HISTORY_HEADER: &str = "Zeit;Metrik;Schlüssel;Einheit;Wert;Min;Max";
const EVENTS_HEADER: &str = "Beginn;Ende;Art;Betreff;Stufe;Text";

/// The history as CSV: one line per point, in the order of the input
/// (metric, then key, then time). `Wert` is the mean of the point, `Min`
/// and `Max` its extremes; for a single sample all three are the value.
/// Times are written in `zone` (an IANA name such as `Europe/Berlin`); an
/// unknown zone counts as UTC.
pub fn history_csv(series: &[(Metric, Vec<Series>)], zone: &str) -> String {
    let tz = time_zone(zone);
    let mut out = format!("{BOM}{HISTORY_HEADER}\n");
    for (metric, list) in series {
        let (name, unit) = metric_label(*metric);
        let decimals = if unit.starts_with("Byte") { 0 } else { 1 };
        for one in list {
            let key = field(one.key.as_deref().unwrap_or(""));
            for point in &one.points {
                out.push_str(&format!(
                    "{};{name};{key};{unit};{};{};{}\n",
                    time(point.at, &tz),
                    number(point.mean, decimals),
                    number(point.min, decimals),
                    number(point.max, decimals),
                ));
            }
        }
    }
    out
}

/// The events as CSV: one line per event, in the order of the input. An
/// open event has an empty `Ende`, an event of the whole NAS an empty
/// `Betreff`. Times are written in `zone`; an unknown zone counts as UTC.
pub fn events_csv(events: &[EventRecord], zone: &str) -> String {
    let tz = time_zone(zone);
    let mut out = format!("{BOM}{EVENTS_HEADER}\n");
    for event in events {
        let end = event.ended_at.map(|at| time(at, &tz)).unwrap_or_default();
        out.push_str(&format!(
            "{};{end};{};{};{};{}\n",
            time(event.started_at, &tz),
            kind_label(&event.kind),
            field(event.subject.as_deref().unwrap_or("")),
            level_label(event.level),
            field(&event.text),
        ));
    }
    out
}

fn time_zone(zone: &str) -> TimeZone {
    TimeZone::get(zone).unwrap_or(TimeZone::UTC)
}

fn time(at: i64, tz: &TimeZone) -> String {
    match Timestamp::from_second(at) {
        Ok(ts) => ts
            .to_zoned(tz.clone())
            .strftime("%Y-%m-%d %H:%M")
            .to_string(),
        // Outside the range jiff knows; not reachable for real samples.
        Err(_) => String::new(),
    }
}

/// German name and unit of a metric.
fn metric_label(metric: Metric) -> (&'static str, &'static str) {
    match metric {
        Metric::VolumeUsed => ("Belegung", "Byte"),
        Metric::VolumeTotal => ("Größe", "Byte"),
        Metric::DiskTemp => ("Plattentemperatur", "°C"),
        Metric::Cpu => ("CPU", "%"),
        Metric::Mem => ("RAM", "%"),
        Metric::NetRx => ("Empfangen", "Byte/s"),
        Metric::NetTx => ("Gesendet", "Byte/s"),
        Metric::SysTemp => ("Systemtemperatur", "°C"),
    }
}

/// German name of an event kind; a kind this build does not know stays as
/// it is stored.
fn kind_label(kind: &str) -> String {
    let label = match kind {
        "alert" => "Meldung des NAS",
        "disk_health" => "Plattenzustand",
        "disk_temperature" => "Plattentemperatur",
        "volume_health" => "Volume-Zustand",
        "volume_fill" => "Volume-Belegung",
        "fan_health" => "Lüfter",
        "ups_on_battery" => "USV im Akkubetrieb",
        "backup_failed" => "Backup fehlgeschlagen",
        "backup_overdue" => "Backup überfällig",
        "unreachable" => "Nicht erreichbar",
        other => return field(other),
    };
    label.to_owned()
}

fn level_label(level: Health) -> &'static str {
    match level {
        Health::Ok => "OK",
        Health::Warning => "Warnung",
        Health::Critical => "Kritisch",
    }
}

/// A number with `decimals` places and a decimal comma; `-0` is `0`.
fn number(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    let text = match text.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => rest.to_owned(),
        _ => text,
    };
    text.replace('.', ",")
}

/// A text field: defused against formulas, quoted when needed.
fn field(text: &str) -> String {
    let mut value = String::with_capacity(text.len() + 2);
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        value.push('\'');
    }
    value.push_str(text);
    if value.contains([';', '"', '\n', '\r', '\'']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Point;

    /// 2026-01-15 12:00 UTC, winter.
    const WINTER: i64 = 1_768_478_400;
    /// 2026-07-15 12:00 UTC, summer.
    const SUMMER: i64 = 1_784_116_800;

    fn point(at: i64, min: f64, mean: f64, max: f64) -> Point {
        Point {
            at,
            min,
            mean,
            max,
            count: 1,
        }
    }

    fn event(kind: &str, subject: Option<&str>, text: &str, ended_at: Option<i64>) -> EventRecord {
        EventRecord {
            nas_id: "nas-1".into(),
            kind: kind.into(),
            subject: subject.map(Into::into),
            level: Health::Warning,
            text: text.into(),
            started_at: WINTER,
            ended_at,
        }
    }

    fn lines(csv: &str) -> Vec<&str> {
        csv.lines().collect()
    }

    #[test]
    fn empty_history_is_bom_and_header() {
        assert_eq!(
            history_csv(&[], "UTC"),
            "\u{FEFF}Zeit;Metrik;Schlüssel;Einheit;Wert;Min;Max\n"
        );
    }

    #[test]
    fn empty_events_are_bom_and_header() {
        assert_eq!(
            events_csv(&[], "UTC"),
            "\u{FEFF}Beginn;Ende;Art;Betreff;Stufe;Text\n"
        );
    }

    #[test]
    fn history_lines_with_decimal_comma() {
        let series = vec![
            (
                Metric::VolumeUsed,
                vec![Series {
                    key: Some("DataVol1".into()),
                    points: vec![point(
                        WINTER,
                        1_234_000_000.4,
                        1_234_567_890.6,
                        1_235_000_000.0,
                    )],
                }],
            ),
            (
                Metric::Cpu,
                vec![Series {
                    key: None,
                    points: vec![point(WINTER, 3.0, 12.44, 55.07)],
                }],
            ),
            (
                Metric::NetRx,
                vec![Series {
                    key: Some("eth0".into()),
                    points: vec![point(WINTER, -0.2, -0.0, 1024.4)],
                }],
            ),
        ];
        let csv = history_csv(&series, "UTC");
        assert!(csv.starts_with('\u{FEFF}'));
        assert_eq!(
            lines(&csv)[1..],
            [
                "2026-01-15 12:00;Belegung;DataVol1;Byte;1234567891;1234000000;1235000000",
                "2026-01-15 12:00;CPU;;%;12,4;3,0;55,1",
                "2026-01-15 12:00;Empfangen;eth0;Byte/s;0;0;1024",
            ]
        );
    }

    #[test]
    fn every_metric_has_a_label() {
        for metric in Metric::ALL {
            let (name, unit) = metric_label(metric);
            assert!(!name.is_empty() && !unit.is_empty());
        }
    }

    #[test]
    fn berlin_is_one_hour_ahead_in_winter_and_two_in_summer() {
        let series = vec![(
            Metric::Mem,
            vec![Series {
                key: None,
                points: vec![point(WINTER, 1.0, 1.0, 1.0), point(SUMMER, 2.0, 2.0, 2.0)],
            }],
        )];
        let csv = history_csv(&series, "Europe/Berlin");
        let rows = lines(&csv);
        assert!(rows[1].starts_with("2026-01-15 13:00;"), "{}", rows[1]);
        assert!(rows[2].starts_with("2026-07-15 14:00;"), "{}", rows[2]);
    }

    #[test]
    fn unknown_zone_is_utc() {
        let csv = events_csv(&[event("alert", None, "x", None)], "Mars/Olympus");
        assert!(lines(&csv)[1].starts_with("2026-01-15 12:00;"));
    }

    #[test]
    fn open_event_has_empty_end() {
        let csv = events_csv(
            &[
                event("disk_temperature", Some("3"), "Platte 3 bei 52 °C", None),
                event("unreachable", None, "weg", Some(WINTER + 3600)),
            ],
            "UTC",
        );
        assert_eq!(
            lines(&csv)[1..],
            [
                "2026-01-15 12:00;;Plattentemperatur;3;Warnung;Platte 3 bei 52 °C",
                "2026-01-15 12:00;2026-01-15 13:00;Nicht erreichbar;;Warnung;weg",
            ]
        );
    }

    #[test]
    fn every_known_kind_is_german() {
        for kind in nas_model::EventKind::ALL {
            assert_ne!(kind_label(kind.as_str()), kind.as_str());
        }
        assert_eq!(kind_label("new_kind"), "new_kind");
    }

    #[test]
    fn fields_are_quoted_and_defused() {
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("a;b"), "\"a;b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(field("two\nlines"), "\"two\nlines\"");
        assert_eq!(field("=SUM(A1)"), "\"'=SUM(A1)\"");
        assert_eq!(field("+1"), "\"'+1\"");
        assert_eq!(field("-1"), "\"'-1\"");
        assert_eq!(field("@cmd"), "\"'@cmd\"");
        assert_eq!(field("\tx"), "\"'\tx\"");
        assert_eq!(field("it's"), "\"it's\"");
    }

    #[test]
    fn event_text_with_semicolon_keeps_its_column() {
        let csv = events_csv(&[event("=evil", Some("=A1;B1"), "a;\"b\"", None)], "UTC");
        assert_eq!(
            lines(&csv)[1],
            "2026-01-15 12:00;;\"'=evil\";\"'=A1;B1\";Warnung;\"a;\"\"b\"\"\""
        );
    }
}
