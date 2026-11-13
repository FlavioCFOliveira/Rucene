//! The committed fixture parses completely and faithfully.

mod common;

use rucene_cli_example::cli::read_csv_file;
use rucene_cli_example::schema::Value;

use common::{fixture_path, fixture_rows};

#[test]
fn fixture_has_no_multiline_fields() {
    let text = std::fs::read_to_string(fixture_path()).unwrap();
    for (number, line) in text.lines().enumerate() {
        assert_eq!(
            line.matches('"').count() % 2,
            0,
            "line {} has an unbalanced quote, so a field spans lines",
            number + 1
        );
    }
}

#[test]
fn every_data_line_becomes_one_record() {
    let text = std::fs::read_to_string(fixture_path()).unwrap();
    let data_lines = text.lines().count() - 1;
    let records = read_csv_file(&fixture_path()).unwrap();
    assert_eq!(records.len(), data_lines);
    assert!(
        (100..=300).contains(&data_lines),
        "fixture size {data_lines}"
    );

    let rows = fixture_rows();
    assert_eq!(rows.len(), data_lines);
    for (record, row) in records.iter().zip(&rows) {
        assert_eq!(record.id(), row.id());
        // Every non-empty raw value is present, every empty one absent.
        for (column, raw) in &row.columns {
            match record.get(column) {
                None => assert!(raw.is_empty(), "{} {column} lost {raw:?}", row.id()),
                Some(Value::String(value)) => assert_eq!(value, raw),
                Some(Value::Double(value)) => assert_eq!(*value, raw.parse::<f64>().unwrap()),
                Some(Value::Int(value)) => {
                    assert_eq!(i64::from(*value), raw.parse::<i64>().unwrap())
                }
                Some(Value::Date(_)) => {
                    assert_eq!(record.get(column).unwrap().display(), *raw);
                }
            }
        }
    }
}
