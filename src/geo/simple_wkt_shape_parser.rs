//! WKT shape parsing, ported from `org.apache.lucene.geo.SimpleWKTShapeParser`.
//!
//! Parses shape geometry represented in WKT format.
//!
//! Complies with the OGC® document 12-063r5 and the ISO/IEC 13249-3:2016
//! standard located at <http://docs.opengeospatial.org/is/12-063r5/12-063r5.html>.
//!
//! # Divergences from Lucene 10.5.0
//!
//! * **The returned geometry is an enum.** Java's `parse` returns `Object`, and
//!   the caller recovers the shape with `instanceof`: `double[]`,
//!   `double[][]`, `Line`, `Line[]`, `Polygon`, `Polygon[]`, `Rectangle`, or
//!   `Object[]` for a collection. Rust has no universal supertype, so the
//!   parser returns [`WktGeometry`]. Java's `null` — which every branch returns
//!   for an `EMPTY` shape, and which a collection can therefore contain —
//!   becomes `Option::None`, so `MULTILINESTRING (EMPTY)` still parses to a
//!   one-element collection holding nothing, as it does in Java.
//! * **`java.io.StreamTokenizer` is reimplemented here.** It has no Rust
//!   analogue. [`StreamTokenizer`] reproduces the tokenizer *as Lucene
//!   configures it* — `resetSyntax`, word characters `a-z`, `A-Z`, `0-9`, `-`,
//!   `+`, `.` and `160..=255`, whitespace `0..=32`, `#` as the comment
//!   character, and every character at or above 256 treated as alphabetic,
//!   which is what `nextToken` does for a code unit outside its 256-entry
//!   character-type table. It is deliberately not a general port of the class:
//!   only the configuration this parser uses is reproduced, and the module
//!   documents that boundary rather than pretending to more.
//! * **The tokenizer counts `char`s, not UTF-16 code units.** The line number a
//!   `ParseException` carries is unaffected; only the treatment of an astral
//!   character differs, and both implementations classify it as alphabetic.
//! * **`ShapeType::for_name` returns a `Result`.** Java throws the unchecked
//!   `IllegalArgumentException`; Rust has no unchecked exceptions, so the same
//!   message travels in [`crate::error::LuceneError::IllegalArgument`]. Inside
//!   the parser it is converted to a [`ParseException`] at the current line, so
//!   a malformed WKT string is still one error channel for the caller.
//! * **There is no `IOException`.** Java reads through a `StringReader`, whose
//!   `read` is declared to throw; reading a `Vec<char>` cannot fail.

use crate::error::LuceneError;
use crate::geo::geometry::{Line, Polygon, Rectangle};
use crate::geo::simple_geojson_polygon_parser::ParseException;

/// The literal `EMPTY` keyword.
pub const EMPTY: &str = "EMPTY";
/// A single space.
pub const SPACE: &str = " ";
/// The left parenthesis token.
pub const LPAREN: &str = "(";
/// The right parenthesis token.
pub const RPAREN: &str = ")";
/// The comma token.
pub const COMMA: &str = ",";
/// The literal `NaN` keyword.
pub const NAN: &str = "NaN";

/// How `tokenString` names a number token.
const NUMBER: &str = "<NUMBER>";
/// How `tokenString` names the end of the stream.
const EOF: &str = "END-OF-STREAM";
/// How `tokenString` names the end of a line.
const EOL: &str = "END-OF-LINE";

/// A geometry parsed from a WKT string.
///
/// Stands in for the `Object` that `SimpleWKTShapeParser.parse` returns; see
/// the module documentation. Every variant that Java represents with `null`
/// is an `Option::None` in the enclosing position rather than a variant here.
#[derive(Clone, Debug, PartialEq)]
pub enum WktGeometry {
    /// A `POINT`, as the `{lon, lat}` pair Java returns in a `double[]`.
    Point([f64; 2]),
    /// A `MULTIPOINT`, as the `{lon, lat}` pairs Java returns in a `double[][]`.
    MultiPoint(Vec<[f64; 2]>),
    /// A `LINESTRING`.
    Line(Line),
    /// A `MULTILINESTRING`. An element is `None` where Java holds a `null`,
    /// which it stores for a nested `EMPTY`.
    MultiLine(Vec<Option<Line>>),
    /// A `POLYGON`.
    Polygon(Polygon),
    /// A `MULTIPOLYGON`. An element is `None` where Java holds a `null`.
    MultiPolygon(Vec<Option<Polygon>>),
    /// An `ENVELOPE` (also spelled `BBOX`).
    Envelope(Rectangle),
    /// A `GEOMETRYCOLLECTION`. An element is `None` where Java holds a `null`.
    GeometryCollection(Vec<Option<WktGeometry>>),
}

/// Enumerated type for shapes.
///
/// Port of `org.apache.lucene.geo.SimpleWKTShapeParser.ShapeType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeType {
    /// A single point.
    Point,
    /// A set of points.
    MultiPoint,
    /// A single line string.
    LineString,
    /// A set of line strings.
    MultiLineString,
    /// A single polygon.
    Polygon,
    /// A set of polygons.
    MultiPolygon,
    /// A heterogeneous collection of geometries.
    GeometryCollection,
    /// An axis-aligned bounding box. Not part of the actual WKB spec.
    Envelope,
}

impl ShapeType {
    /// How the WKB spec spells `ENVELOPE`.
    const BBOX: &'static str = "BBOX";

    /// Returns the lower-case shape name.
    ///
    /// Equivalent to the package-private `ShapeType.typename()`.
    pub(crate) fn typename(self) -> &'static str {
        match self {
            ShapeType::Point => "point",
            ShapeType::MultiPoint => "multipoint",
            ShapeType::LineString => "linestring",
            ShapeType::MultiLineString => "multilinestring",
            ShapeType::Polygon => "polygon",
            ShapeType::MultiPolygon => "multipolygon",
            ShapeType::GeometryCollection => "geometrycollection",
            ShapeType::Envelope => "envelope",
        }
    }

    /// The WKT shape name.
    ///
    /// Equivalent to `ShapeType.wktName()`.
    pub fn wkt_name(self) -> &'static str {
        if self == ShapeType::Envelope {
            Self::BBOX
        } else {
            self.typename()
        }
    }

    /// Resolves a shape name, case-insensitively.
    ///
    /// Equivalent to `ShapeType.forName(String)`. Java throws the unchecked
    /// `IllegalArgumentException` for an unknown name; see the module
    /// documentation for why this port returns a `Result` instead.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the name matches no shape.
    pub fn for_name(shapename: &str) -> Result<ShapeType, LuceneError> {
        let typename = shapename.to_lowercase();
        const ALL: [ShapeType; 8] = [
            ShapeType::Point,
            ShapeType::MultiPoint,
            ShapeType::LineString,
            ShapeType::MultiLineString,
            ShapeType::Polygon,
            ShapeType::MultiPolygon,
            ShapeType::GeometryCollection,
            ShapeType::Envelope,
        ];
        for candidate in ALL {
            if candidate.typename() == typename {
                return Ok(candidate);
            }
        }
        // Lucene also registers ENVELOPE under its WKT name, lower-cased.
        if typename == ShapeType::Envelope.wkt_name().to_lowercase() {
            return Ok(ShapeType::Envelope);
        }
        Err(LuceneError::IllegalArgument(format!(
            "unknown geo_shape [{shapename}]"
        )))
    }
}

impl std::fmt::Display for ShapeType {
    /// Renders the variant the way Java renders an enum constant, so that the
    /// parser's error messages match Lucene's.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            ShapeType::Point => "POINT",
            ShapeType::MultiPoint => "MULTIPOINT",
            ShapeType::LineString => "LINESTRING",
            ShapeType::MultiLineString => "MULTILINESTRING",
            ShapeType::Polygon => "POLYGON",
            ShapeType::MultiPolygon => "MULTIPOLYGON",
            ShapeType::GeometryCollection => "GEOMETRYCOLLECTION",
            ShapeType::Envelope => "ENVELOPE",
        };
        f.write_str(name)
    }
}

/// Parses shape geometry represented in WKT format.
///
/// Port of `org.apache.lucene.geo.SimpleWKTShapeParser`. Java forbids
/// instantiation with a private constructor; this port is a namespace type with
/// associated functions only.
pub struct SimpleWKTShapeParser;

impl SimpleWKTShapeParser {
    /// Parses any WKT geometry.
    ///
    /// Equivalent to `SimpleWKTShapeParser.parse(String)`. Returns `None` for an
    /// `EMPTY` geometry, which Java signals with a `null` return.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseException`] naming the offending token and the line it
    /// was found on.
    pub fn parse(wkt: &str) -> Result<Option<WktGeometry>, ParseException> {
        Self::parse_expected_type(wkt, None)
    }

    /// Parses a WKT geometry, requiring it to be of the expected type.
    ///
    /// Equivalent to `SimpleWKTShapeParser.parseExpectedType(String, ShapeType)`.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseException`] when the geometry is malformed, or when it
    /// is not of `shape_type`.
    pub fn parse_expected_type(
        wkt: &str,
        shape_type: Option<ShapeType>,
    ) -> Result<Option<WktGeometry>, ParseException> {
        // set up the tokenizer; configured to read words w/o numbers
        let mut tokenizer = StreamTokenizer::new(wkt);
        let geometry = parse_geometry(&mut tokenizer, shape_type)?;
        check_eof(&mut tokenizer)?;
        Ok(geometry)
    }
}

/// Parses a geometry from the stream tokenizer.
fn parse_geometry(
    stream: &mut StreamTokenizer,
    shape_type: Option<ShapeType>,
) -> Result<Option<WktGeometry>, ParseException> {
    let word = next_word(stream)?;
    let type_ = ShapeType::for_name(&word)
        .map_err(|e| ParseException::new(e.to_string(), stream.lineno()))?;
    if let Some(expected) = shape_type {
        if expected != ShapeType::GeometryCollection && type_.wkt_name() != expected.wkt_name() {
            return Err(ParseException::new(
                format!("Expected geometry type: [{expected}], but found: [{type_}]"),
                stream.lineno(),
            ));
        }
    }
    let geometry = match type_ {
        ShapeType::Point => parse_point(stream)?.map(WktGeometry::Point),
        ShapeType::MultiPoint => parse_multi_point(stream)?.map(WktGeometry::MultiPoint),
        ShapeType::LineString => parse_line(stream)?.map(WktGeometry::Line),
        ShapeType::MultiLineString => parse_multi_line(stream)?.map(WktGeometry::MultiLine),
        ShapeType::Polygon => parse_polygon(stream)?.map(WktGeometry::Polygon),
        ShapeType::MultiPolygon => parse_multi_polygon(stream)?.map(WktGeometry::MultiPolygon),
        ShapeType::Envelope => parse_bbox(stream)?.map(WktGeometry::Envelope),
        ShapeType::GeometryCollection => {
            parse_geometry_collection(stream)?.map(WktGeometry::GeometryCollection)
        }
    };
    Ok(geometry)
}

/// Parses a point as a `{lon, lat}` pair.
fn parse_point(stream: &mut StreamTokenizer) -> Result<Option<[f64; 2]>, ParseException> {
    if next_empty_or_open(stream)? == EMPTY {
        return Ok(None);
    }
    let pt = [next_number(stream)?, next_number(stream)?];
    if is_number_next(stream) {
        next_number(stream)?;
    }
    next_closer(stream)?;
    Ok(Some(pt))
}

/// Parses a list of points into latitude and longitude vectors.
fn parse_coordinates(
    stream: &mut StreamTokenizer,
    lats: &mut Vec<f64>,
    lons: &mut Vec<f64>,
) -> Result<(), ParseException> {
    let mut is_open_paren = false;
    if is_number_next(stream) || {
        is_open_paren = next_word(stream)? == LPAREN;
        is_open_paren
    } {
        parse_coordinate(stream, lats, lons)?;
    }

    while next_closer_or_comma(stream)? == COMMA {
        is_open_paren = false;
        if is_number_next(stream) || {
            is_open_paren = next_word(stream)? == LPAREN;
            is_open_paren
        } {
            parse_coordinate(stream, lats, lons)?;
        }
        if is_open_paren && next_closer(stream)? != RPAREN {
            return Err(ParseException::new(
                format!("expected: [{RPAREN}] but found: [{}]", token_string(stream)),
                stream.lineno(),
            ));
        }
    }

    if is_open_paren && next_closer(stream)? != RPAREN {
        return Err(ParseException::new(
            format!("expected: [{RPAREN}] but found: [{}]", token_string(stream)),
            stream.lineno(),
        ));
    }
    Ok(())
}

/// Parses a single coordinate, with an optional third dimension.
fn parse_coordinate(
    stream: &mut StreamTokenizer,
    lats: &mut Vec<f64>,
    lons: &mut Vec<f64>,
) -> Result<(), ParseException> {
    lons.push(next_number(stream)?);
    lats.push(next_number(stream)?);
    if is_number_next(stream) {
        next_number(stream)?;
    }
    Ok(())
}

/// Parses a `MULTIPOINT`.
fn parse_multi_point(
    stream: &mut StreamTokenizer,
) -> Result<Option<Vec<[f64; 2]>>, ParseException> {
    let token = next_empty_or_open(stream)?;
    if token == EMPTY {
        return Ok(None);
    }
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    parse_coordinates(stream, &mut lats, &mut lons)?;
    let result = lats
        .iter()
        .zip(lons.iter())
        .map(|(lat, lon)| [*lon, *lat])
        .collect();
    Ok(Some(result))
}

/// Parses a `LINESTRING`.
fn parse_line(stream: &mut StreamTokenizer) -> Result<Option<Line>, ParseException> {
    let token = next_empty_or_open(stream)?;
    if token == EMPTY {
        return Ok(None);
    }
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    parse_coordinates(stream, &mut lats, &mut lons)?;
    let line =
        Line::new(lats, lons).map_err(|e| ParseException::new(e.to_string(), stream.lineno()))?;
    Ok(Some(line))
}

/// Parses a `MULTILINESTRING`.
fn parse_multi_line(
    stream: &mut StreamTokenizer,
) -> Result<Option<Vec<Option<Line>>>, ParseException> {
    let token = next_empty_or_open(stream)?;
    if token == EMPTY {
        return Ok(None);
    }
    let mut lines = Vec::new();
    lines.push(parse_line(stream)?);
    while next_closer_or_comma(stream)? == COMMA {
        lines.push(parse_line(stream)?);
    }
    Ok(Some(lines))
}

/// Parses the hole of a polygon.
fn parse_polygon_hole(stream: &mut StreamTokenizer) -> Result<Polygon, ParseException> {
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    parse_coordinates(stream, &mut lats, &mut lons)?;
    Polygon::new(lats, lons, Vec::new())
        .map_err(|e| ParseException::new(e.to_string(), stream.lineno()))
}

/// Parses a `POLYGON`.
fn parse_polygon(stream: &mut StreamTokenizer) -> Result<Option<Polygon>, ParseException> {
    if next_empty_or_open(stream)? == EMPTY {
        return Ok(None);
    }
    next_opener(stream)?;
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    parse_coordinates(stream, &mut lats, &mut lons)?;
    let mut holes = Vec::new();
    while next_closer_or_comma(stream)? == COMMA {
        holes.push(parse_polygon_hole(stream)?);
    }

    let polygon = Polygon::new(lats, lons, holes)
        .map_err(|e| ParseException::new(e.to_string(), stream.lineno()))?;
    Ok(Some(polygon))
}

/// Parses a `MULTIPOLYGON`.
fn parse_multi_polygon(
    stream: &mut StreamTokenizer,
) -> Result<Option<Vec<Option<Polygon>>>, ParseException> {
    let token = next_empty_or_open(stream)?;
    if token == EMPTY {
        return Ok(None);
    }
    let mut polygons = Vec::new();
    polygons.push(parse_polygon(stream)?);
    while next_closer_or_comma(stream)? == COMMA {
        polygons.push(parse_polygon(stream)?);
    }
    Ok(Some(polygons))
}

/// Parses an `ENVELOPE`.
fn parse_bbox(stream: &mut StreamTokenizer) -> Result<Option<Rectangle>, ParseException> {
    if next_empty_or_open(stream)? == EMPTY {
        return Ok(None);
    }
    let min_lon = next_number(stream)?;
    next_comma(stream)?;
    let max_lon = next_number(stream)?;
    next_comma(stream)?;
    let max_lat = next_number(stream)?;
    next_comma(stream)?;
    let min_lat = next_number(stream)?;
    next_closer(stream)?;
    let rectangle = Rectangle::new(min_lat, max_lat, min_lon, max_lon)
        .map_err(|e| ParseException::new(e.to_string(), stream.lineno()))?;
    Ok(Some(rectangle))
}

/// Parses a `GEOMETRYCOLLECTION`.
fn parse_geometry_collection(
    stream: &mut StreamTokenizer,
) -> Result<Option<Vec<Option<WktGeometry>>>, ParseException> {
    if next_empty_or_open(stream)? == EMPTY {
        return Ok(None);
    }
    let mut geometries = Vec::new();
    geometries.push(parse_geometry(stream, Some(ShapeType::GeometryCollection))?);
    while next_closer_or_comma(stream)? == COMMA {
        geometries.push(parse_geometry(stream, None)?);
    }
    Ok(Some(geometries))
}

/// Reads the next word in the stream.
fn next_word(stream: &mut StreamTokenizer) -> Result<String, ParseException> {
    match stream.next_token() {
        Token::Word(word) => {
            if word.eq_ignore_ascii_case(EMPTY) {
                Ok(EMPTY.to_string())
            } else {
                Ok(word)
            }
        }
        Token::Ordinary('(') => Ok(LPAREN.to_string()),
        Token::Ordinary(')') => Ok(RPAREN.to_string()),
        Token::Ordinary(',') => Ok(COMMA.to_string()),
        _ => Err(ParseException::new(
            format!("expected word but found: {}", token_string(stream)),
            stream.lineno(),
        )),
    }
}

/// Reads the next number in the stream.
fn next_number(stream: &mut StreamTokenizer) -> Result<f64, ParseException> {
    if let Token::Word(word) = stream.next_token() {
        if word.eq_ignore_ascii_case(NAN) {
            return Ok(f64::NAN);
        }
        return match parse_java_double(&word) {
            Some(value) => Ok(value),
            None => Err(ParseException::new(
                format!("invalid number found: {word}"),
                stream.lineno(),
            )),
        };
    }
    Err(ParseException::new(
        format!("expected number but found: {}", token_string(stream)),
        stream.lineno(),
    ))
}

/// Parses a double the way `Double.parseDouble` does for the inputs this
/// tokenizer can produce: Rust's `f64::from_str` accepts the same grammar,
/// except for Java's trailing type suffixes and its leading/trailing
/// whitespace, neither of which a word token can contain.
fn parse_java_double(text: &str) -> Option<f64> {
    text.parse::<f64>().ok()
}

/// Names the current token the way `tokenString` names it.
fn token_string(stream: &StreamTokenizer) -> String {
    match stream.current() {
        Token::Word(word) => word.clone(),
        Token::Eof => EOF.to_string(),
        Token::Eol => EOL.to_string(),
        Token::Number => NUMBER.to_string(),
        Token::Ordinary(ch) => format!("'{ch}'"),
        Token::None => EOF.to_string(),
    }
}

/// Checks whether the next token is a word, then pushes it back.
fn is_number_next(stream: &mut StreamTokenizer) -> bool {
    let is_word = matches!(stream.next_token(), Token::Word(_));
    stream.push_back();
    is_word
}

/// Checks that the next token is `EMPTY` or an opening parenthesis.
fn next_empty_or_open(stream: &mut StreamTokenizer) -> Result<String, ParseException> {
    let next = next_word(stream)?;
    if next == EMPTY || next == LPAREN {
        return Ok(next);
    }
    Err(ParseException::new(
        format!(
            "expected {EMPTY} or {LPAREN} but found: {}",
            token_string(stream)
        ),
        stream.lineno(),
    ))
}

/// Checks that the next token is a closing parenthesis.
fn next_closer(stream: &mut StreamTokenizer) -> Result<String, ParseException> {
    if next_word(stream)? == RPAREN {
        return Ok(RPAREN.to_string());
    }
    Err(ParseException::new(
        format!("expected {RPAREN} but found: {}", token_string(stream)),
        stream.lineno(),
    ))
}

/// Expects a comma as the next token.
fn next_comma(stream: &mut StreamTokenizer) -> Result<String, ParseException> {
    if next_word(stream)? == COMMA {
        return Ok(COMMA.to_string());
    }
    Err(ParseException::new(
        format!("expected {COMMA} but found: {}", token_string(stream)),
        stream.lineno(),
    ))
}

/// Expects an opening parenthesis as the next token.
fn next_opener(stream: &mut StreamTokenizer) -> Result<String, ParseException> {
    if next_word(stream)? == LPAREN {
        return Ok(LPAREN.to_string());
    }
    Err(ParseException::new(
        format!("expected {LPAREN} but found: {}", token_string(stream)),
        stream.lineno(),
    ))
}

/// Expects either a closing parenthesis or a comma as the next token.
fn next_closer_or_comma(stream: &mut StreamTokenizer) -> Result<String, ParseException> {
    let token = next_word(stream)?;
    if token == COMMA || token == RPAREN {
        return Ok(token);
    }
    Err(ParseException::new(
        format!(
            "expected {COMMA} or {RPAREN} but found: {}",
            token_string(stream)
        ),
        stream.lineno(),
    ))
}

/// Checks that the stream holds nothing but the geometry just parsed.
fn check_eof(stream: &mut StreamTokenizer) -> Result<(), ParseException> {
    if stream.next_token() != Token::Eof {
        return Err(ParseException::new(
            format!(
                "expected end of WKT string but found additional text: {}",
                token_string(stream)
            ),
            stream.lineno(),
        ));
    }
    Ok(())
}

/// A token produced by [`StreamTokenizer`].
///
/// The variants correspond to the `ttype` values of `java.io.StreamTokenizer`:
/// `Word` is `TT_WORD`, `Eof` is `TT_EOF`, `Eol` is `TT_EOL`, `Number` is
/// `TT_NUMBER` (which this configuration never produces, since `parseNumbers`
/// is off after `resetSyntax`), and `Ordinary` carries the character itself.
/// `None` is the state before the first `nextToken`, which Java leaves as the
/// field's default of zero.
#[derive(Clone, Debug, PartialEq)]
// `Number` and `Eol` are never produced by this configuration -- `parseNumbers`
// is off after `resetSyntax` and `eolIsSignificant` is false -- but Java's
// `tokenString` names them, so they are kept for fidelity and matched there.
#[allow(dead_code)]
enum Token {
    Word(String),
    Ordinary(char),
    Number,
    Eol,
    Eof,
    None,
}

/// The subset of `java.io.StreamTokenizer` that `SimpleWKTShapeParser` uses,
/// configured exactly as Lucene configures it.
///
/// Word characters are `a-z`, `A-Z`, `0-9`, `-`, `+`, `.` and `160..=255`;
/// characters `0..=32` are whitespace; `#` begins a comment that runs to the end
/// of the line; every other character below 256 is ordinary; and every character
/// at or above 256 is alphabetic, which is how `nextToken` treats a code unit
/// outside its 256-entry character-type table.
struct StreamTokenizer {
    input: Vec<char>,
    pos: usize,
    lineno: usize,
    current: Token,
    pushed_back: bool,
}

impl StreamTokenizer {
    /// Creates the tokenizer over the given text.
    fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            // StreamTokenizer starts at line 1.
            lineno: 1,
            current: Token::None,
            pushed_back: false,
        }
    }

    /// Returns the line number of the current token.
    ///
    /// Equivalent to `StreamTokenizer.lineno()`.
    fn lineno(&self) -> usize {
        self.lineno
    }

    /// Returns the token most recently produced.
    fn current(&self) -> &Token {
        &self.current
    }

    /// Makes the next call to [`StreamTokenizer::next_token`] return the current
    /// token again.
    ///
    /// Equivalent to `StreamTokenizer.pushBack()`.
    fn push_back(&mut self) {
        if self.current != Token::None {
            self.pushed_back = true;
        }
    }

    /// Reads the next token.
    ///
    /// Equivalent to `StreamTokenizer.nextToken()`.
    fn next_token(&mut self) -> Token {
        if self.pushed_back {
            self.pushed_back = false;
            return self.current.clone();
        }

        loop {
            let ch = match self.read() {
                Some(ch) => ch,
                None => {
                    self.current = Token::Eof;
                    return self.current.clone();
                }
            };

            if is_whitespace_char(ch) {
                // StreamTokenizer counts a line at '\n' and at '\r'; a "\r\n"
                // pair counts once, because the '\n' is swallowed by the
                // lookahead.
                if ch == '\r' {
                    self.lineno += 1;
                    if self.peek() == Some('\n') {
                        self.pos += 1;
                    }
                } else if ch == '\n' {
                    self.lineno += 1;
                }
                continue;
            }

            if ch == '#' {
                // A comment runs to the end of the line; the terminator is left
                // for the whitespace branch above to count.
                while let Some(next) = self.peek() {
                    if next == '\n' || next == '\r' {
                        break;
                    }
                    self.pos += 1;
                }
                continue;
            }

            if is_word_char(ch) {
                let start = self.pos - 1;
                while let Some(next) = self.peek() {
                    if is_word_char(next) {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let word: String = self.input[start..self.pos].iter().collect();
                self.current = Token::Word(word);
                return self.current.clone();
            }

            self.current = Token::Ordinary(ch);
            return self.current.clone();
        }
    }

    /// Consumes and returns the next character.
    fn read(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    /// Returns the next character without consuming it.
    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }
}

/// `whitespaceChars(0, ' ')`.
fn is_whitespace_char(ch: char) -> bool {
    (ch as u32) <= 0x20
}

/// The word characters Lucene configures, plus every character at or above 256,
/// which `nextToken` classifies as alphabetic.
fn is_word_char(ch: char) -> bool {
    let code = ch as u32;
    if code >= 256 {
        return true;
    }
    ch.is_ascii_lowercase()
        || ch.is_ascii_uppercase()
        || ch.is_ascii_digit()
        || ch == '-'
        || ch == '+'
        || ch == '.'
        || (160..=255).contains(&code)
}
