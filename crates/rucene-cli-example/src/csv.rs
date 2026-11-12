//! A strict RFC 4180 CSV reader.
//!
//! The USGS catalog is RFC 4180 CSV: comma-separated fields, records ended by
//! `CRLF` (or `LF`), fields that hold a comma, a quote or a line break wrapped
//! in double quotes, and a quote inside a quoted field doubled. This reader
//! implements exactly that grammar and rejects anything outside it (a stray
//! quote in an unquoted field, text after a closing quote, an unterminated
//! quoted field), reporting the line where the problem starts.

use crate::error::{CliError, Result};

/// One parsed record: its fields, and the 1-based line it starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvRecord {
    /// The 1-based line number of the first character of the record.
    pub line: usize,
    /// The unquoted field values, in order.
    pub fields: Vec<String>,
}

/// Parses `text` into records.
///
/// A final line break is optional. Empty lines are not records and are
/// skipped (RFC 4180 does not define them; the USGS output has none).
///
/// # Errors
///
/// Returns [`CliError::Input`] with the line number when the text breaks the
/// RFC 4180 grammar.
pub fn parse(text: &str) -> Result<Vec<CsvRecord>> {
    let mut records = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut line = 1usize;
    let mut record_line = 1usize;
    // Whether the current record has any content yet (to skip empty lines).
    let mut record_started = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if !field.is_empty() {
                    return Err(CliError::Input(format!(
                        "CSV line {line}: quote inside an unquoted field"
                    )));
                }
                record_started = true;
                let opened_at = line;
                loop {
                    match chars.next() {
                        None => {
                            return Err(CliError::Input(format!(
                                "CSV line {opened_at}: unterminated quoted field"
                            )))
                        }
                        Some('"') => {
                            if chars.peek() == Some(&'"') {
                                chars.next();
                                field.push('"');
                            } else {
                                break;
                            }
                        }
                        Some(inner) => {
                            if inner == '\n' {
                                line += 1;
                            }
                            field.push(inner);
                        }
                    }
                }
                match chars.peek() {
                    None | Some(',') | Some('\n') | Some('\r') => {}
                    Some(_) => {
                        return Err(CliError::Input(format!(
                            "CSV line {line}: text after the closing quote of a field"
                        )))
                    }
                }
            }
            ',' => {
                record_started = true;
                fields.push(std::mem::take(&mut field));
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                if record_started {
                    fields.push(std::mem::take(&mut field));
                    records.push(CsvRecord {
                        line: record_line,
                        fields: std::mem::take(&mut fields),
                    });
                }
                record_started = false;
                line += 1;
                record_line = line;
            }
            other => {
                record_started = true;
                field.push(other);
            }
        }
    }
    if record_started {
        fields.push(field);
        records.push(CsvRecord {
            line: record_line,
            fields,
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(text: &str) -> Vec<Vec<String>> {
        parse(text)
            .unwrap()
            .into_iter()
            .map(|record| record.fields)
            .collect()
    }

    #[test]
    fn plain_quoted_and_empty_fields() {
        assert_eq!(
            fields("a,b,c\r\n1,\"x, y\",\n"),
            vec![vec!["a", "b", "c"], vec!["1", "x, y", ""]]
        );
    }

    #[test]
    fn doubled_quotes_and_embedded_line_breaks() {
        let records = parse("\"he said \"\"hi\"\"\",\"two\nlines\"\nnext,row").unwrap();
        assert_eq!(records[0].fields, vec!["he said \"hi\"", "two\nlines"]);
        assert_eq!(records[1].line, 3);
        assert_eq!(records[1].fields, vec!["next", "row"]);
    }

    #[test]
    fn empty_lines_are_skipped_and_the_final_break_is_optional() {
        assert_eq!(fields("a\n\nb"), vec![vec!["a"], vec!["b"]]);
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn grammar_violations_are_rejected_with_their_line() {
        let message = |text: &str| parse(text).unwrap_err().to_string();
        assert_eq!(
            message("ok\nab\"c"),
            "CSV line 2: quote inside an unquoted field"
        );
        assert_eq!(
            message("\"abc\"d"),
            "CSV line 1: text after the closing quote of a field"
        );
        assert_eq!(
            message("a\n\"open"),
            "CSV line 2: unterminated quoted field"
        );
    }
}
