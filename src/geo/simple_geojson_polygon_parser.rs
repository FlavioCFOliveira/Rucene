//! Minimal GeoJSON polygon parsing, ported from
//! `org.apache.lucene.geo.SimpleGeoJSONPolygonParser`.
//!
//! The parser accepts either a whole `type: Feature`, the inner object with
//! `type: Polygon` or `type: MultiPolygon`, or a `type: FeatureCollection` that
//! holds exactly one polygon geometry. It is deliberately minimal: it does not
//! implement JSON, only as much of it as a GeoJSON polygon needs.
//!
//! # Divergences from Lucene 10.5.0
//!
//! * **The JSON value model is an enum.** Java parses into `Object` and
//!   recovers the type with `instanceof` (`String`, `Double`, `Boolean`,
//!   `List<Object>`, `null`). Rust has no universal supertype, so the parser
//!   builds a [`JsonValue`] instead. Every `instanceof` test in the Java source
//!   becomes a match on that enum, and the error messages it produces are the
//!   same ones Java produces for the corresponding branch.
//! * **Offsets count `char`s, not UTF-16 code units.** Java indexes the input
//!   with `String.charAt`, so `upto` is a UTF-16 code-unit offset; this port
//!   indexes a `Vec<char>`, so `upto` counts Unicode scalar values. The two
//!   agree for every character in the Basic Multilingual Plane and differ only
//!   in the reported offset of an error that follows an astral character. A
//!   faithful `u16` buffer would make every string operation in the parser
//!   surrogate-aware for no gain in what the parser accepts.
//! * **An invalid polygon becomes a [`ParseException`].** Java's `parsePolygon`
//!   calls the `Polygon` constructor, which throws the *unchecked*
//!   `IllegalArgumentException` — it escapes `fromGeoJSON` past its declared
//!   `throws ParseException`. Rust has no unchecked exceptions and
//!   [`crate::geo::Polygon::from_geojson`] returns `Result<_, ParseException>`,
//!   so the validation message is carried in a `ParseException` at the current
//!   offset rather than in a second, invisible error channel.
//!
//! One Lucene behaviour is reproduced deliberately even though it looks like a
//! defect: a `\uXXXX` escape appends the *decimal digits of the code point*
//! rather than the character it denotes, because Java's
//! `StringBuilder.append(int)` appends the number. See [`ParserState::parse_string`].

use std::fmt;

use crate::geo::geometry::Polygon;

/// The exception a GeoJSON parse failure raises.
///
/// Equivalent to the `java.text.ParseException` that
/// `org.apache.lucene.geo.SimpleGeoJSONPolygonParser` throws. It carries the
/// same message and the same error offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseException {
    message: String,
    error_offset: usize,
}

impl ParseException {
    /// Creates a parse exception with the given message and error offset.
    ///
    /// Equivalent to `new ParseException(String, int)`.
    pub fn new(message: impl Into<String>, error_offset: usize) -> Self {
        Self {
            message: message.into(),
            error_offset,
        }
    }

    /// Returns the position where the error was found.
    ///
    /// Equivalent to `ParseException.getErrorOffset()`.
    pub fn error_offset(&self) -> usize {
        self.error_offset
    }

    /// Returns the detail message.
    ///
    /// Equivalent to `Throwable.getMessage()`.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ParseException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseException {}

/// A parsed JSON value.
///
/// Stands in for the `Object` that Lucene's parser produces, so that the
/// `instanceof` tests of the Java source become exhaustive matches. `Null` is
/// the value Java represents with a `null` reference, which its parser also
/// yields for a nested object, whose contents it consumes into the parser state
/// rather than returning.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    /// A JSON string.
    String(String),
    /// A JSON number. Lucene only handles doubles.
    Number(f64),
    /// A JSON boolean.
    Boolean(bool),
    /// A JSON array.
    Array(Vec<JsonValue>),
    /// A JSON `null`, or a nested object consumed into the parser state.
    Null,
}

impl JsonValue {
    /// Renders the value the way `String.valueOf(Object)` renders the Java
    /// object it stands for, so that the parser's error messages match
    /// Lucene's.
    fn to_java_string(&self) -> String {
        match self {
            JsonValue::String(s) => s.clone(),
            // Java boxes every number as a Double, whose toString always shows
            // a decimal point.
            JsonValue::Number(n) => format_java_double(*n),
            JsonValue::Boolean(b) => b.to_string(),
            JsonValue::Array(items) => {
                let rendered: Vec<String> = items.iter().map(JsonValue::to_java_string).collect();
                format!("[{}]", rendered.join(", "))
            }
            JsonValue::Null => "null".to_string(),
        }
    }

    /// Names the value's type the way `Object.getClass()` names it, for the
    /// error messages that report an unexpected class.
    fn java_class(&self) -> &'static str {
        match self {
            JsonValue::String(_) => "class java.lang.String",
            JsonValue::Number(_) => "class java.lang.Double",
            JsonValue::Boolean(_) => "class java.lang.Boolean",
            JsonValue::Array(_) => "class java.util.ArrayList",
            JsonValue::Null => "null",
        }
    }
}

/// Formats a double the way `Double.toString` does for the values this parser
/// produces: always with a decimal point, and never in Rust's shortest form
/// when Java would print an exponent.
fn format_java_double(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    let magnitude = value.abs();
    if magnitude != 0.0 && !(1e-3..1e7).contains(&magnitude) {
        // Java switches to computerized scientific notation outside this range.
        let mut s = format!("{:E}", value);
        if let Some(pos) = s.find('E') {
            let (mantissa, exponent) = s.split_at(pos);
            if !mantissa.contains('.') {
                s = format!("{mantissa}.0{exponent}");
            }
        }
        return s;
    }
    if value == value.trunc() {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

/// Does minimal parsing of a GeoJSON object, to extract either a `Polygon` or a
/// `MultiPolygon`, either directly as the top-level type, or, if the top-level
/// type is `Feature`, as the geometry of that feature.
///
/// Port of `org.apache.lucene.geo.SimpleGeoJSONPolygonParser`. Lucene declares
/// the class package-private, so this port keeps it visible to the crate only
/// and reaches it through [`crate::geo::Polygon::from_geojson`].
pub(crate) struct SimpleGeoJSONPolygonParser {
    state: ParserState,
}

impl SimpleGeoJSONPolygonParser {
    /// Creates a parser over the given GeoJSON text.
    pub(crate) fn new(input: &str) -> Self {
        Self {
            state: ParserState {
                input: input.chars().collect(),
                upto: 0,
                poly_type: None,
                coordinates: None,
            },
        }
    }

    /// Parses the object and returns the polygons it holds.
    pub(crate) fn parse(mut self) -> Result<Vec<Polygon>, ParseException> {
        self.state.parse()
    }
}

/// The parser's mutable state: the input, the cursor, and the two pieces of the
/// object that must be collected before they can be put together.
///
/// Lucene keeps these as fields of the parser class. They live in their own
/// struct here so that the recursive descent can borrow them without borrowing
/// the public wrapper.
struct ParserState {
    input: Vec<char>,
    upto: usize,
    poly_type: Option<String>,
    coordinates: Option<Vec<JsonValue>>,
}

impl ParserState {
    fn parse(&mut self) -> Result<Vec<Polygon>, ParseException> {
        // parse entire object
        self.parse_object("")?;

        // make sure there's nothing left:
        self.read_end()?;

        // The order of JSON object keys (type, geometry, coordinates in our
        // case) can be arbitrary, so we wait until we are done parsing to put
        // the pieces together here:

        let coordinates = match self.coordinates.take() {
            Some(coordinates) => coordinates,
            None => return Err(self.new_parse_exception("did not see any polygon coordinates")),
        };

        let poly_type = match self.poly_type.clone() {
            Some(poly_type) => poly_type,
            None => {
                return Err(self.new_parse_exception("did not see type: Polygon or MultiPolygon"))
            }
        };

        if poly_type == "Polygon" {
            Ok(vec![self.parse_polygon(&coordinates)?])
        } else {
            let mut polygons = Vec::new();
            for o in &coordinates {
                match o {
                    JsonValue::Array(items) => polygons.push(self.parse_polygon(items)?),
                    other => {
                        return Err(self.new_parse_exception(format!(
                            "elements of coordinates array should be an array, but got: {}",
                            other.java_class()
                        )))
                    }
                }
            }
            Ok(polygons)
        }
    }

    /// `path` is the "address" by keys of where we are, e.g. `geometry.coordinates`.
    fn parse_object(&mut self, path: &str) -> Result<(), ParseException> {
        self.scan_char('{')?;
        let mut first = true;
        loop {
            let mut ch = self.peek()?;
            if ch == '}' {
                break;
            } else if !first {
                if ch == ',' {
                    // ok
                    self.upto += 1;
                    ch = self.peek()?;
                    if ch == '}' {
                        break;
                    }
                } else {
                    return Err(self.new_parse_exception(format!("expected , but got {ch}")));
                }
            }

            first = false;

            let mut upto_start = self.upto;
            let key = self.parse_string()?;

            if path == "crs.properties" && key == "href" {
                self.upto = upto_start;
                return Err(self.new_parse_exception("cannot handle linked crs"));
            }

            self.scan_char(':')?;

            ch = self.peek()?;

            upto_start = self.upto;

            let o: JsonValue = if ch == '[' {
                let new_path = join_path(path, &key);
                JsonValue::Array(self.parse_array(&new_path)?)
            } else if ch == '{' {
                let new_path = join_path(path, &key);
                self.parse_object(&new_path)?;
                JsonValue::Null
            } else if ch == '"' {
                JsonValue::String(self.parse_string()?)
            } else if ch == 't' {
                self.scan_str("true")?;
                JsonValue::Boolean(true)
            } else if ch == 'f' {
                self.scan_str("false")?;
                JsonValue::Boolean(false)
            } else if ch == 'n' {
                self.scan_str("null")?;
                JsonValue::Null
            } else if ch == '-' || ch == '.' || ch.is_ascii_digit() {
                JsonValue::Number(self.parse_number()?)
            } else if ch == '}' {
                break;
            } else {
                return Err(self.new_parse_exception(format!(
                    "expected array, object, string or literal value, but got: {ch}"
                )));
            };

            if path == "crs.properties" && key == "name" {
                let crs = match &o {
                    JsonValue::String(s) => s.clone(),
                    other => {
                        let rendered = other.to_java_string();
                        self.upto = upto_start;
                        return Err(self.new_parse_exception(format!(
                            "crs.properties.name should be a string, but saw: {rendered}"
                        )));
                    }
                };
                if !crs.starts_with("urn:ogc:def:crs:OGC") || !crs.ends_with(":CRS84") {
                    self.upto = upto_start;
                    return Err(self.new_parse_exception(format!(
                        "crs must be CRS84 from OGC, but saw: {crs}"
                    )));
                }
            }

            if key == "type" && !path.starts_with("crs") {
                let type_name = match &o {
                    JsonValue::String(s) => s.clone(),
                    other => {
                        let rendered = other.to_java_string();
                        self.upto = upto_start;
                        return Err(self.new_parse_exception(format!(
                            "type should be a string, but got: {rendered}"
                        )));
                    }
                };
                if type_name == "Polygon" && is_valid_geometry_path(path) {
                    self.poly_type = Some("Polygon".to_string());
                } else if type_name == "MultiPolygon" && is_valid_geometry_path(path) {
                    self.poly_type = Some("MultiPolygon".to_string());
                } else if (type_name == "FeatureCollection" || type_name == "Feature")
                    && (path == "features.[]" || path.is_empty())
                {
                    // OK, we recurse
                } else {
                    self.upto = upto_start;
                    return Err(self.new_parse_exception(format!(
                        "can only handle type FeatureCollection (if it has a single polygon \
                         geometry), Feature, Polygon or MultiPolygon, but got {type_name}"
                    )));
                }
            } else if key == "coordinates" && is_valid_geometry_path(path) {
                let items = match o {
                    JsonValue::Array(items) => items,
                    other => {
                        let class = other.java_class();
                        self.upto = upto_start;
                        return Err(self.new_parse_exception(format!(
                            "coordinates should be an array, but got: {class}"
                        )));
                    }
                };
                if self.coordinates.is_some() {
                    self.upto = upto_start;
                    return Err(
                        self.new_parse_exception("only one Polygon or MultiPolygon is supported")
                    );
                }
                self.coordinates = Some(items);
            }
        }

        self.scan_char('}')
    }

    fn parse_polygon(&self, coordinates: &[JsonValue]) -> Result<Polygon, ParseException> {
        let mut holes = Vec::new();
        let first = coordinates.first().unwrap_or(&JsonValue::Null);
        let poly_points = match first {
            JsonValue::Array(items) => self.parse_points(items)?,
            other => {
                return Err(self.new_parse_exception(format!(
                    "first element of polygon array must be an array [[lat, lon], [lat, lon] ...] \
                     but got: {}",
                    other.to_java_string()
                )))
            }
        };
        for o in coordinates.iter().skip(1) {
            let hole_points = match o {
                JsonValue::Array(items) => self.parse_points(items)?,
                other => {
                    return Err(self.new_parse_exception(format!(
                        "elements of coordinates array must be an array [[lat, lon], [lat, lon] \
                         ...] but got: {}",
                        other.to_java_string()
                    )))
                }
            };
            holes.push(self.build_polygon(hole_points.0, hole_points.1, Vec::new())?);
        }
        self.build_polygon(poly_points.0, poly_points.1, holes)
    }

    /// Builds a polygon, turning the validation failure that Java raises as the
    /// unchecked `IllegalArgumentException` into a [`ParseException`] — see the
    /// module documentation.
    fn build_polygon(
        &self,
        lats: Vec<f64>,
        lons: Vec<f64>,
        holes: Vec<Polygon>,
    ) -> Result<Polygon, ParseException> {
        Polygon::new(lats, lons, holes).map_err(|e| self.new_parse_exception(e.to_string()))
    }

    /// Parses `[[lat, lon], [lat, lon] ...]` into parallel latitude and
    /// longitude vectors.
    fn parse_points(&self, o: &[JsonValue]) -> Result<(Vec<f64>, Vec<f64>), ParseException> {
        let mut lats = Vec::with_capacity(o.len());
        let mut lons = Vec::with_capacity(o.len());
        for point in o {
            let point_list = match point {
                JsonValue::Array(items) => items,
                other => {
                    return Err(self.new_parse_exception(format!(
                        "elements of coordinates array must [lat, lon] array, but got: {}",
                        other.to_java_string()
                    )))
                }
            };
            if point_list.len() != 2 {
                let rendered: Vec<String> =
                    point_list.iter().map(JsonValue::to_java_string).collect();
                return Err(self.new_parse_exception(format!(
                    "elements of coordinates array must [lat, lon] array, but got wrong element \
                     count: [{}]",
                    rendered.join(", ")
                )));
            }
            let lon = match &point_list[0] {
                JsonValue::Number(n) => *n,
                other => {
                    return Err(self.new_parse_exception(format!(
                        "elements of coordinates array must [lat, lon] array, but first element is \
                         not a Double: {}",
                        other.to_java_string()
                    )))
                }
            };
            let lat = match &point_list[1] {
                JsonValue::Number(n) => *n,
                other => {
                    return Err(self.new_parse_exception(format!(
                        "elements of coordinates array must [lat, lon] array, but second element \
                         is not a Double: {}",
                        other.to_java_string()
                    )))
                }
            };

            // lon, lat ordering in GeoJSON!
            lons.push(lon);
            lats.push(lat);
        }

        Ok((lats, lons))
    }

    fn parse_array(&mut self, path: &str) -> Result<Vec<JsonValue>, ParseException> {
        let mut result: Vec<JsonValue> = Vec::new();
        self.scan_char('[')?;
        while self.upto < self.input.len() {
            let mut ch = self.peek()?;
            if ch == ']' {
                self.scan_char(']')?;
                return Ok(result);
            }

            if !result.is_empty() {
                if ch != ',' {
                    return Err(self.new_parse_exception(format!(
                        "expected ',' separating list items, but got '{ch}'"
                    )));
                }

                // skip the ,
                self.upto += 1;

                if self.upto == self.input.len() {
                    return Err(self.new_parse_exception("hit EOF while parsing array"));
                }
                ch = self.peek()?;
            }

            let o: JsonValue = if ch == '[' {
                JsonValue::Array(self.parse_array(&format!("{path}.[]"))?)
            } else if ch == '{' {
                // This is only used when parsing the "features" in type: FeatureCollection
                self.parse_object(&format!("{path}.[]"))?;
                JsonValue::Null
            } else if ch == '-' || ch == '.' || ch.is_ascii_digit() {
                JsonValue::Number(self.parse_number()?)
            } else if ch == '"' {
                JsonValue::String(self.parse_string()?)
            } else {
                return Err(self.new_parse_exception(format!(
                    "expected another array or number while parsing array, not '{ch}'"
                )));
            };

            result.push(o);
        }

        Err(self.new_parse_exception("hit EOF while reading array"))
    }

    fn parse_number(&mut self) -> Result<f64, ParseException> {
        let mut b = String::new();
        let upto_start = self.upto;
        while self.upto < self.input.len() {
            let ch = self.input[self.upto];
            if ch == '-' || ch == '.' || ch.is_ascii_digit() || ch == 'e' || ch == 'E' {
                self.upto += 1;
                b.push(ch);
            } else {
                break;
            }
        }

        // we only handle doubles
        match b.parse::<f64>() {
            Ok(value) => Ok(value),
            Err(_) => {
                self.upto = upto_start;
                Err(self.new_parse_exception("could not parse number as double"))
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, ParseException> {
        self.scan_char('"')?;
        let mut b = String::new();
        while self.upto < self.input.len() {
            let mut ch = self.input[self.upto];
            if ch == '"' {
                self.upto += 1;
                return Ok(b);
            }
            if ch == '\\' {
                // an escaped character
                self.upto += 1;
                if self.upto == self.input.len() {
                    return Err(self.new_parse_exception("hit EOF inside string literal"));
                }
                ch = self.input[self.upto];
                if ch == 'u' {
                    // 4 hex digit unicode BMP escape
                    self.upto += 1;
                    if self.upto + 4 > self.input.len() {
                        return Err(self.new_parse_exception("hit EOF inside string literal"));
                    }
                    let digits: String = self.input[self.upto..self.upto + 4]
                        .iter()
                        .collect::<String>();
                    let code_point = match u32::from_str_radix(&digits, 16) {
                        Ok(value) => value,
                        Err(_) => {
                            return Err(self.new_parse_exception(format!(
                                "For input string: \"{digits}\" under radix 16"
                            )))
                        }
                    };
                    // Faithful to Lucene 10.5.0: `StringBuilder.append(int)`
                    // appends the decimal digits of the code point, not the
                    // character it denotes, and the parser never advances past
                    // the four hex digits. Both are reproduced deliberately —
                    // correcting them here would change which documents a
                    // GeoJSON polygon parses to, relative to Java.
                    b.push_str(&code_point.to_string());
                } else if ch == '\\' {
                    b.push('\\');
                    self.upto += 1;
                } else {
                    // TODO: allow \n, \t, etc.???
                    return Err(self.new_parse_exception(format!(
                        "unsupported string escape character \\{ch}"
                    )));
                }
            } else {
                b.push(ch);
                self.upto += 1;
            }
        }

        Err(self.new_parse_exception("hit EOF inside string literal"))
    }

    fn peek(&mut self) -> Result<char, ParseException> {
        while self.upto < self.input.len() {
            let ch = self.input[self.upto];
            if is_json_whitespace(ch) {
                self.upto += 1;
                continue;
            }
            return Ok(ch);
        }

        Err(self.new_parse_exception("unexpected EOF"))
    }

    /// Scans across whitespace and consumes the expected character, or returns a
    /// [`ParseException`] if the character is wrong.
    fn scan_char(&mut self, expected: char) -> Result<(), ParseException> {
        while self.upto < self.input.len() {
            let ch = self.input[self.upto];
            if is_json_whitespace(ch) {
                self.upto += 1;
                continue;
            }
            if ch != expected {
                return Err(
                    self.new_parse_exception(format!("expected '{expected}' but got '{ch}'"))
                );
            }
            self.upto += 1;
            return Ok(());
        }
        Err(self.new_parse_exception(format!("expected '{expected}' but got EOF")))
    }

    fn read_end(&mut self) -> Result<(), ParseException> {
        while self.upto < self.input.len() {
            let ch = self.input[self.upto];
            if !is_json_whitespace(ch) {
                return Err(self.new_parse_exception(format!(
                    "unexpected character '{ch}' after end of GeoJSON object"
                )));
            }
            self.upto += 1;
        }
        Ok(())
    }

    /// Scans the expected string, or returns a [`ParseException`].
    fn scan_str(&mut self, expected: &str) -> Result<(), ParseException> {
        let expected_len = expected.chars().count();
        if self.upto + expected_len > self.input.len() {
            return Err(self.new_parse_exception(format!("expected \"{expected}\" but hit EOF")));
        }
        let sub_string: String = self.input[self.upto..self.upto + expected_len]
            .iter()
            .collect();
        if sub_string != expected {
            return Err(self
                .new_parse_exception(format!("expected \"{expected}\" but got \"{sub_string}\"")));
        }
        self.upto += expected_len;
        Ok(())
    }

    /// When calling this, `upto` should be at the position of the incorrect
    /// character.
    fn new_parse_exception(&self, details: impl Into<String>) -> ParseException {
        let details = details.into();
        let end = self.input.len().min(self.upto + 1);
        let fragment: String = if self.upto < 50 {
            self.input[0..end].iter().collect()
        } else {
            let mut s = String::from("...");
            s.extend(self.input[self.upto - 50..end].iter());
            s
        };
        ParseException::new(
            format!(
                "{details} at character offset {}; fragment leading to this:\n{fragment}",
                self.upto
            ),
            self.upto,
        )
    }
}

/// Returns true if the object path is a valid location to see a Multi/Polygon
/// geometry.
fn is_valid_geometry_path(path: &str) -> bool {
    path.is_empty() || path == "geometry" || path == "features.[].geometry"
}

/// Appends `key` to `path`, matching how the Java parser builds a nested path.
fn join_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// JSON does not accept Unicode whitespace, so only these four characters skip.
fn is_json_whitespace(ch: char) -> bool {
    ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r'
}
