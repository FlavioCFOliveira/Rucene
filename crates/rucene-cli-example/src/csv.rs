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
    let mut reader = Reader::new(text.as_bytes());
    let mut records = Vec::new();
    while let Some(record) = reader.next_record()? {
        records.push(record);
    }
    Ok(records)
}

/// A streaming RFC 4180 reader: yields one record at a time from a buffered
/// input, holding only the current line and record in memory.
///
/// It applies exactly the grammar of [`parse`] (which is built on it), with
/// the same line numbers and the same error messages.
#[derive(Debug)]
pub struct Reader<R> {
    input: R,
    /// The physical line being read, through its line feed.
    buffer: String,
    /// The byte offset of the next character in `buffer`.
    position: usize,
    /// The 1-based number of the line the next character is on.
    line: usize,
    /// The 1-based line on which the next record starts.
    record_line: usize,
}

impl<R: std::io::BufRead> Reader<R> {
    /// Creates a reader positioned at the start of `input`.
    pub fn new(input: R) -> Self {
        Self {
            input,
            buffer: String::new(),
            position: 0,
            line: 1,
            record_line: 1,
        }
    }

    /// Makes `buffer` hold at least one unread character, reading the next
    /// line when it is exhausted; `false` at the end of the input.
    fn fill(&mut self) -> Result<bool> {
        if self.position < self.buffer.len() {
            return Ok(true);
        }
        self.buffer.clear();
        self.position = 0;
        Ok(self.input.read_line(&mut self.buffer)? > 0)
    }

    fn peek_char(&mut self) -> Result<Option<char>> {
        if !self.fill()? {
            return Ok(None);
        }
        Ok(self.buffer[self.position..].chars().next())
    }

    fn next_char(&mut self) -> Result<Option<char>> {
        let next = self.peek_char()?;
        if let Some(c) = next {
            self.position += c.len_utf8();
        }
        Ok(next)
    }

    /// Reads the next record; `None` at the end of the input.
    ///
    /// # Errors
    ///
    /// Returns [`CliError::Input`] with the line number when the text breaks
    /// the RFC 4180 grammar, and [`CliError::Io`] when the input cannot be
    /// read or is not UTF-8.
    pub fn next_record(&mut self) -> Result<Option<CsvRecord>> {
        let mut fields: Vec<String> = Vec::new();
        let mut field = String::new();
        // Whether the current record has any content yet (to skip empty lines).
        let mut record_started = false;

        while let Some(c) = self.next_char()? {
            match c {
                '"' => {
                    if !field.is_empty() {
                        return Err(CliError::Input(format!(
                            "CSV line {}: quote inside an unquoted field",
                            self.line
                        )));
                    }
                    record_started = true;
                    let opened_at = self.line;
                    loop {
                        match self.next_char()? {
                            None => {
                                return Err(CliError::Input(format!(
                                    "CSV line {opened_at}: unterminated quoted field"
                                )))
                            }
                            Some('"') => {
                                if self.peek_char()? == Some('"') {
                                    self.next_char()?;
                                    field.push('"');
                                } else {
                                    break;
                                }
                            }
                            Some(inner) => {
                                if inner == '\n' {
                                    self.line += 1;
                                }
                                field.push(inner);
                            }
                        }
                    }
                    match self.peek_char()? {
                        None | Some(',') | Some('\n') | Some('\r') => {}
                        Some(_) => {
                            return Err(CliError::Input(format!(
                                "CSV line {}: text after the closing quote of a field",
                                self.line
                            )))
                        }
                    }
                }
                ',' => {
                    record_started = true;
                    fields.push(std::mem::take(&mut field));
                }
                '\r' | '\n' => {
                    if c == '\r' && self.peek_char()? == Some('\n') {
                        self.next_char()?;
                    }
                    let record = record_started.then(|| {
                        fields.push(std::mem::take(&mut field));
                        CsvRecord {
                            line: self.record_line,
                            fields: std::mem::take(&mut fields),
                        }
                    });
                    record_started = false;
                    self.line += 1;
                    self.record_line = self.line;
                    if record.is_some() {
                        return Ok(record);
                    }
                }
                other => {
                    record_started = true;
                    field.push(other);
                }
            }
        }
        if record_started {
            fields.push(field);
            return Ok(Some(CsvRecord {
                line: self.record_line,
                fields,
            }));
        }
        Ok(None)
    }
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
    fn the_reader_streams_records_one_by_one_across_lines() {
        let text = "a,\"x\ny\"\r\n\nb,c\nd";
        let mut reader = Reader::new(std::io::BufReader::with_capacity(1, text.as_bytes()));
        let mut streamed = Vec::new();
        while let Some(record) = reader.next_record().unwrap() {
            streamed.push(record);
        }
        assert_eq!(streamed, parse(text).unwrap());
        assert_eq!(streamed[1].line, 4);
        assert_eq!(streamed[2].fields, vec!["d"]);
        assert!(reader.next_record().unwrap().is_none());
    }

    #[test]
    fn the_reader_reports_input_that_is_not_utf8() {
        let mut reader = Reader::new(&b"a,b\n\xff\n"[..]);
        assert_eq!(
            reader.next_record().unwrap().unwrap().fields,
            vec!["a", "b"]
        );
        assert!(matches!(reader.next_record(), Err(CliError::Io(_))));
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
