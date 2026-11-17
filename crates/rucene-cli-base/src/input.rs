//! The bulk input format of `add` and `update`: **JSON Lines** of flat
//! records.
//!
//! # Format
//!
//! * UTF-8 text, one JSON value (RFC 8259) per line, `\n` or `\r\n` line
//!   ends; blank lines are skipped.
//! * Each line is **one flat JSON object**: one document of the index the
//!   command names. Member names are the field names of that index's
//!   [`EntitySchema`], case-sensitive.
//! * A member value is a scalar (string, number, boolean, `null`) or, for a
//!   multi-valued field only, an array of scalars. A scalar on a
//!   multi-valued field is one value. Nested objects, and arrays inside
//!   arrays, are rejected: nested source lists are flattened by the ingestion
//!   script into parallel multi-valued fields.
//! * Values by field kind:
//!   * textual kinds take a string (a number or boolean is taken as its JSON
//!     text); the CLI trims it;
//!   * flags take `"Sim"`/`"Não"`/`"Nao"` (any case) or `true`/`false`, and
//!     are indexed as `sim`/`nao`;
//!   * `int` takes a JSON integer or a string of one, within 32 bits;
//!     `double` a JSON number or a string of one, finite;
//!   * dates take `YYYY-MM-DD` or `DD/MM/YYYY`, either optionally followed by
//!     ` HH:MM:SS` (see [`crate::date`]).
//! * **Absent:** a missing member, `null`, `""`, a whitespace-only string,
//!   `[]`, or a placeholder declared for the field
//!   ([`FieldSpec::absent`](crate::schema::FieldSpec::absent)). An absent
//!   value adds no field. Absent elements of an array are dropped.
//! * Derived hierarchy fields are computed by the CLI and must not be given.
//! * **Strict:** an unknown or repeated member, a list on a single-valued
//!   field, a value that does not fit its kind, or a record without its key
//!   fails the command; the writer is rolled back and nothing of the file is
//!   committed. Errors carry the file name and line number.
//!
//! `get` writes stored documents in this same format, so its output can be
//! fed back to `add` or `update`.

use std::io::BufRead;

use crate::error::{CliError, Result};
use crate::json::{self, JsonValue};
use crate::schema::{EntitySchema, Raw, RawField, Record};

/// Converts one parsed JSON line into a record of `schema`.
///
/// # Errors
///
/// Returns [`CliError::Input`] when the value is not an object, holds a
/// nested object or array, or fails [`Record::from_fields`].
pub fn record_from_json(schema: &'static EntitySchema, value: JsonValue) -> Result<Record> {
    let JsonValue::Object(members) = value else {
        return Err(CliError::Input(
            "a line must hold one JSON object".to_owned(),
        ));
    };
    let mut fields = Vec::with_capacity(members.len());
    for (name, value) in &members {
        let field = match value {
            JsonValue::Array(items) => RawField::List(
                items
                    .iter()
                    .map(|item| scalar(name, item))
                    .collect::<Result<_>>()?,
            ),
            other => RawField::One(scalar(name, other)?),
        };
        fields.push((name.as_str(), field));
    }
    Record::from_fields(schema, fields)
}

fn scalar(name: &str, value: &JsonValue) -> Result<Raw> {
    Ok(match value {
        JsonValue::Null => Raw::Null,
        JsonValue::Bool(value) => Raw::Bool(*value),
        JsonValue::Number(text) => Raw::Number(text.clone()),
        JsonValue::String(text) => Raw::Text(text.clone()),
        JsonValue::Array(_) | JsonValue::Object(_) => {
            return Err(CliError::Input(format!(
                "field {name:?}: nested arrays and objects are not accepted"
            )))
        }
    })
}

/// Parses one JSON Lines line (without its line end) into a record.
///
/// # Errors
///
/// As [`json::parse`] and [`record_from_json`].
pub fn parse_line(schema: &'static EntitySchema, line: &str) -> Result<Record> {
    record_from_json(schema, json::parse(line)?)
}

/// Streams the records of JSON Lines input, one line at a time.
///
/// Each non-blank line is parsed into a [`Record`] when it is reached, so
/// the input is never held in memory whole. Errors carry the 1-based line
/// number; the stream stops at its first error.
#[derive(Debug)]
pub struct RecordReader<R> {
    schema: &'static EntitySchema,
    input: R,
    line: usize,
    buffer: String,
    failed: bool,
}

impl<R: BufRead> RecordReader<R> {
    /// A reader of `input` for the index `schema`.
    pub fn new(schema: &'static EntitySchema, input: R) -> Self {
        Self {
            schema,
            input,
            line: 0,
            buffer: String::new(),
            failed: false,
        }
    }
}

impl<R: BufRead> Iterator for RecordReader<R> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            self.buffer.clear();
            self.line += 1;
            let read = match self.input.read_line(&mut self.buffer) {
                Ok(read) => read,
                Err(error) => {
                    self.failed = true;
                    let message = if error.kind() == std::io::ErrorKind::InvalidData {
                        format!("line {}: not valid UTF-8", self.line)
                    } else {
                        format!("line {}: {error}", self.line)
                    };
                    return Some(Err(CliError::Input(message)));
                }
            };
            if read == 0 {
                return None;
            }
            let line = self.buffer.trim_end_matches(['\n', '\r']);
            if line.trim().is_empty() {
                continue;
            }
            let record = parse_line(self.schema, line)
                .map_err(|error| error.context(&format!("line {}", self.line)));
            self.failed = record.is_err();
            return Some(record);
        }
    }
}

/// Builds one record from `--set field=value` pairs: a field repeated is a
/// list (multi-valued fields only).
///
/// # Errors
///
/// As [`Record::from_fields`].
pub fn record_from_pairs(schema: &'static EntitySchema, pairs: &[(&str, &str)]) -> Result<Record> {
    let mut grouped: Vec<(&str, Vec<Raw>)> = Vec::new();
    for (name, value) in pairs {
        let raw = Raw::Text((*value).to_owned());
        match grouped.iter_mut().find(|(existing, _)| existing == name) {
            Some((_, values)) => values.push(raw),
            None => grouped.push((name, vec![raw])),
        }
    }
    let mut fields = Vec::with_capacity(grouped.len());
    for (name, mut values) in grouped {
        let spec = schema.require_field(name)?;
        let field = if spec.multi {
            RawField::List(values)
        } else if values.len() == 1 {
            RawField::One(values.remove(0))
        } else {
            return Err(CliError::Input(format!(
                "field {name:?} is single-valued but was given {} times",
                values.len()
            )));
        };
        fields.push((name, field));
    }
    Record::from_fields(schema, fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{contratos, modificacoes};
    use crate::schema::Value;

    #[test]
    fn a_line_becomes_a_typed_record() {
        let record = parse_line(
            &contratos::SCHEMA,
            r#"{"idcontrato": "10855918", "precoContratual": 11500.0, "prazoExecucao": 99999999,
                "adjudicatarioNif": ["510763464", "-"], "dataPublicacao": "2024-08-06",
                "CritMateriais": "Não", "ocorrencias": false, "precoBaseProcedimento": 0.0,
                "nuts": [], "Observacoes": "n/a"}"#,
        )
        .unwrap();
        assert_eq!(record.key(), "10855918");
        assert_eq!(record.values("precoContratual"), &[Value::Double(11500.0)]);
        assert!(record.values("prazoExecucao").is_empty());
        assert!(record.values("precoBaseProcedimento").is_empty());
        assert!(record.values("Observacoes").is_empty());
        assert_eq!(
            record.values("adjudicatarioNif"),
            &[Value::String("510763464".into())]
        );
        assert_eq!(
            record.values("CritMateriais"),
            &[Value::String("nao".into())]
        );
        assert_eq!(record.values("ocorrencias"), &[Value::String("nao".into())]);
    }

    #[test]
    fn strictness_errors_name_the_problem() {
        let schema = &contratos::SCHEMA;
        for (line, expected) in [
            (r#"{"precoContratual": 1}"#, "key field"),
            (r#"{"idcontrato": "1", "bogus": 1}"#, "unknown field"),
            (r#"{"idcontrato": "1", "cpvDivisao": "45"}"#, "derived"),
            (r#"{"idcontrato": ["1", "2"]}"#, "single-valued"),
            (r#"{"idcontrato": "1", "cpv": [["x"]]}"#, "nested"),
            (r#"[1]"#, "one JSON object"),
            (r#"{"idcontrato": "1", "Ano": "dois"}"#, "32-bit integer"),
        ] {
            let error = parse_line(schema, line).unwrap_err().to_string();
            assert!(error.contains(expected), "{line}: {error}");
        }
    }

    #[test]
    fn composite_keys_join_with_a_slash() {
        let record = parse_line(
            &modificacoes::SCHEMA,
            r#"{"idcontrato": "11103352", "sequencia": "2", "modifContratoData": "15/04/2026 00:00:00"}"#,
        )
        .unwrap();
        assert_eq!(record.key(), "11103352/2");
        assert!(parse_line(
            &modificacoes::SCHEMA,
            r#"{"idcontrato": "1/2", "sequencia": 1}"#
        )
        .is_err());
        assert!(parse_line(&modificacoes::SCHEMA, r#"{"idcontrato": "1"}"#).is_err());
    }

    #[test]
    fn reader_skips_blank_lines_and_numbers_errors() {
        let input =
            "{\"idcontrato\": \"1\"}\n\n  \r\n{\"idcontrato\": \"2\"}\r\n{\"idcontrato\": 3\n";
        let mut reader = RecordReader::new(&contratos::SCHEMA, input.as_bytes());
        assert_eq!(reader.next().unwrap().unwrap().key(), "1");
        assert_eq!(reader.next().unwrap().unwrap().key(), "2");
        let error = reader.next().unwrap().unwrap_err().to_string();
        assert!(error.starts_with("line 5: "), "{error}");
        assert!(reader.next().is_none());
    }

    #[test]
    fn set_pairs_repeat_only_for_multi_valued_fields() {
        let schema = &contratos::SCHEMA;
        let record = record_from_pairs(
            schema,
            &[
                ("idcontrato", "9"),
                ("cpv", "45000000-7"),
                ("cpv", "71000000-8"),
            ],
        )
        .unwrap();
        assert_eq!(record.values("cpv").len(), 2);
        assert!(record_from_pairs(schema, &[("idcontrato", "9"), ("idcontrato", "8")]).is_err());
    }
}
