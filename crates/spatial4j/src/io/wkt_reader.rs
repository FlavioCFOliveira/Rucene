/*******************************************************************************
 * Copyright (c) 2015 ElasticSearch and MITRE, and others
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `WKTReader.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::util::extra::ParseException;
use rucene_analysis_common::jdk::character;

use crate::context::spatial_context::SpatialContext;
use crate::context::spatial_context_factory::SpatialContextFactoryOps;
use crate::exception::invalid_shape_exception::InvalidShapeException;
use crate::io::one_points_builder::OnePointsBuilder;
use crate::io::shape_io::{self, ShapeIO};
use crate::io::shape_reader::{read_all, ShapeReader};
use crate::jdk::{is_digit, parse_double};

/// `Character.isLetter(char)`.
fn is_letter(c: char) -> bool {
    character::is_letter(c as i32)
}

/// `Character.isWhitespace(char)`.
fn is_whitespace(c: char) -> bool {
    character::is_whitespace(c as i32)
}

/// `Character.isJavaIdentifierPart(char)`.
fn is_java_identifier_part(c: char) -> bool {
    character::is_java_identifier_part(c as i32)
}
use crate::shape::shape::Shape;
use crate::shape::shape_factory::{PointsBuilder, PolygonBuilder, ShapeFactory};

/// `getMessage()` of a Java exception.
fn message_of(e: &LuceneError) -> Option<String> {
    match e.kind() {
        LuceneErrorKind::IllegalArgument { message, .. } => Some(message.clone()),
        LuceneErrorKind::Unchecked { message, .. } => message.clone(),
        _ => Some(e.to_string()),
    }
}

/// Equivalent to `org.locationtech.spatial4j.io.WKTReader` (`public class
/// WKTReader implements ShapeReader`): reads shapes from Well Known Text,
/// with spatial4j's extensions (`ENVELOPE`, `BUFFER`).
#[derive(Debug)]
pub struct WKTReader {
    ctx: Weak<SpatialContext>,
}

impl WKTReader {
    /// `WKTReader(SpatialContext ctx, SpatialContextFactory factory)`.
    pub fn new(ctx: &Arc<SpatialContext>, _factory: &dyn SpatialContextFactoryOps) -> Self {
        Self { ctx: Arc::downgrade(ctx) }
    }

    /// `ctx`.
    pub fn ctx(&self) -> Arc<SpatialContext> {
        self.ctx.upgrade().expect("the SpatialContext of a WKTReader is alive")
    }

    /// `shapeFactory`: `ctx.getShapeFactory()`.
    pub fn shape_factory(&self) -> Arc<dyn ShapeFactory> {
        self.ctx().get_shape_factory()
    }

    /// `parse(String wktString)`.
    ///
    /// # Errors
    ///
    /// `ParseException` for text that is not a known shape;
    /// `InvalidShapeException` for an invalid shape.
    pub fn parse(&self, wkt_string: &str) -> Result<Arc<dyn Shape>> {
        if let Some(shape) = self.parse_if_supported(wkt_string)? {
            return Ok(shape);
        }
        let units: Vec<u16> = wkt_string.encode_utf16().collect();
        let shortened_string = if units.len() <= 128 {
            wkt_string.to_string()
        } else {
            format!("{}...", String::from_utf16_lossy(&units[..128 - 3]))
        };
        Err(ParseException::error(format!("Unknown Shape definition [{shortened_string}]"), 0))
    }

    /// `parseIfSupported(String wktString)`: `None` (Java `null`) for text
    /// that does not start with a shape keyword this reader knows.
    ///
    /// # Errors
    ///
    /// `ParseException` for malformed text; `InvalidShapeException` for an
    /// invalid shape (an `IllegalArgumentException` becomes one).
    pub fn parse_if_supported(&self, wkt_string: &str) -> Result<Option<Arc<dyn Shape>>> {
        let mut state = self.new_state(wkt_string);
        state.next_if_whitespace(); // leading
        if state.eof() {
            return Ok(None);
        }
        //shape types must start with a letter
        if !is_letter(state.char_at(state.offset)) {
            return Ok(None);
        }
        let shape_type = state.next_word()?;
        let result = match self.parse_shape_by_type(&mut state, &shape_type) {
            Ok(r) => r,
            Err(e) if ParseException::of(&e).is_some() || InvalidShapeException::is(&e) => return Err(e),
            Err(e) if e.is_illegal_argument_exception() => {
                // NOTE: JTS Throws IAE for bad WKT
                return Err(InvalidShapeException::with_cause(message_of(&e), e));
            }
            Err(e) => {
                let message = e.to_string();
                return Err(ParseException::error_with_cause(message, state.offset as i32, e));
            }
        };
        if result.is_some() && !state.eof() {
            return Err(ParseException::error("end of shape expected", state.offset as i32));
        }
        Ok(result)
    }

    /// `newState(String wktString)`.
    pub fn new_state(&self, wkt_string: &str) -> State {
        State::new(wkt_string)
    }

    /// `parseShapeByType(State state, String shapeType)`: `None` (Java
    /// `null`) for an unknown shape type.
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_shape_by_type(&self, state: &mut State, shape_type: &str) -> Result<Option<Arc<dyn Shape>>> {
        debug_assert!(
            shape_type.chars().next().is_some_and(is_letter),
            "Shape must start with letter: {shape_type}"
        );
        let shape = if shape_type.eq_ignore_ascii_case("POINT") {
            self.parse_point_shape(state)?
        } else if shape_type.eq_ignore_ascii_case("MULTIPOINT") {
            self.parse_multi_point_shape(state)?
        } else if shape_type.eq_ignore_ascii_case("ENVELOPE") {
            self.parse_envelope_shape(state)?
        } else if shape_type.eq_ignore_ascii_case("LINESTRING") {
            self.parse_line_string_shape(state)?
        } else if shape_type.eq_ignore_ascii_case("POLYGON") {
            self.parse_polygon_shape(state)?
        } else if shape_type.eq_ignore_ascii_case("GEOMETRYCOLLECTION") {
            self.parse_geometry_collection_shape(state)?
        } else if shape_type.eq_ignore_ascii_case("MULTILINESTRING") {
            self.parse_multi_line_string_shape(state)?
        } else if shape_type.eq_ignore_ascii_case("MULTIPOLYGON") {
            self.parse_mulit_polygon_shape(state)?
        } else if shape_type.eq_ignore_ascii_case("BUFFER") {
            // extension
            self.parse_buffer_shape(state)?
        } else {
            // HEY! Update class Javadocs if add more shapes
            return Ok(None);
        };
        Ok(Some(shape))
    }

    /// `parseBufferShape(State)`: `BUFFER(shape, distance)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_buffer_shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        state.next_expect('(')?;
        let shape = self.shape(state)?;
        state.next_expect(',')?;
        let distance = self.shape_factory().norm_dist(state.next_double()?);
        state.next_expect(')')?;
        shape.get_buffered(distance, &self.ctx())
    }

    /// `parsePointShape(State)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_point_shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        let shape_factory = self.shape_factory();
        if state.next_if_empty_and_skip_zm()? {
            return Ok(shape_factory.point_xy(f64::NAN, f64::NAN)?);
        }
        state.next_expect('(')?;
        let mut one_points_builder = OnePointsBuilder::new(Arc::clone(&shape_factory));
        self.point(state, &mut one_points_builder)?;
        state.next_expect(')')?;
        Ok(one_points_builder.get_point().ok_or_else(LuceneError::null_pointer)?)
    }

    /// `parseMultiPointShape(State)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_multi_point_shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        let mut builder = self.shape_factory().multi_point();
        if state.next_if_empty_and_skip_zm()? {
            return builder.build();
        }
        state.next_expect('(')?;
        loop {
            let open_paren = state.next_if('(');
            self.point(state, &mut *builder)?;
            if open_paren {
                state.next_expect(')')?;
            }
            if !state.next_if(',') {
                break;
            }
        }
        state.next_expect(')')?;
        builder.build()
    }

    /// `parseEnvelopeShape(State)`: `ENVELOPE(x1, x2, y2, y1)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_envelope_shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        //FYI no dimension or EMPTY
        state.next_expect('(')?;
        let x1 = state.next_double()?;
        state.next_expect(',')?;
        let x2 = state.next_double()?;
        state.next_expect(',')?;
        let y2 = state.next_double()?;
        state.next_expect(',')?;
        let y1 = state.next_double()?;
        state.next_expect(')')?;
        let f = self.shape_factory();
        Ok(f.rect(f.norm_x(x1), f.norm_x(x2), f.norm_y(y1), f.norm_y(y2))?)
    }

    /// `parseLineStringShape(State)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_line_string_shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        let mut line_string_builder = self.shape_factory().line_string();
        if state.next_if_empty_and_skip_zm()? {
            return line_string_builder.build();
        }
        self.point_list(state, &mut *line_string_builder)?;
        line_string_builder.build()
    }

    /// `parseMultiLineStringShape(State)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_multi_line_string_shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        let mut multi_line_string_builder = self.shape_factory().multi_line_string();
        if !state.next_if_empty_and_skip_zm()? {
            state.next_expect('(')?;
            loop {
                let mut line_string = multi_line_string_builder.line_string();
                self.point_list(state, &mut *line_string)?;
                multi_line_string_builder.add(&mut *line_string)?;
                if !state.next_if(',') {
                    break;
                }
            }
            state.next_expect(')')?;
        }
        multi_line_string_builder.build()
    }

    /// `parsePolygonShape(State)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_polygon_shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        let mut polygon_builder = self.shape_factory().polygon()?;
        if !state.next_if_empty_and_skip_zm()? {
            self.polygon(state, &mut *polygon_builder)?;
        }
        polygon_builder.build_or_rect()
    }

    /// `parseMulitPolygonShape(State)` (Java's spelling).
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_mulit_polygon_shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        let mut multi_polygon_builder = self.shape_factory().multi_polygon();
        if !state.next_if_empty_and_skip_zm()? {
            state.next_expect('(')?;
            loop {
                let mut polygon = multi_polygon_builder.polygon()?;
                self.polygon(state, &mut *polygon)?;
                multi_polygon_builder.add(&mut *polygon)?;
                if !state.next_if(',') {
                    break;
                }
            }
            state.next_expect(')')?;
        }
        multi_polygon_builder.build()
    }

    /// `parseGeometryCollectionShape(State)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the factory's errors.
    pub fn parse_geometry_collection_shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        let mut multi_shape_builder = self.shape_factory().multi_shape();
        if state.next_if_empty_and_skip_zm()? {
            return multi_shape_builder.build();
        }
        state.next_expect('(')?;
        loop {
            let s = self.shape(state)?;
            multi_shape_builder.add(s);
            if !state.next_if(',') {
                break;
            }
        }
        state.next_expect(')')?;
        multi_shape_builder.build()
    }

    /// `shape(State)`: a nested shape.
    ///
    /// # Errors
    ///
    /// `ParseException` for an unknown shape type; the errors of its parse.
    pub fn shape(&self, state: &mut State) -> Result<Arc<dyn Shape>> {
        let type_ = state.next_word()?;
        let shape = self.parse_shape_by_type(state, &type_)?;
        shape.ok_or_else(|| ParseException::error(format!("Shape of type {type_} is unknown"), state.offset as i32))
    }

    /// `pointList(State, B pointsBuilder)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the builder's errors.
    pub fn point_list(&self, state: &mut State, points_builder: &mut dyn PointsBuilder) -> Result<()> {
        state.next_expect('(')?;
        loop {
            self.point(state, points_builder)?;
            if !state.next_if(',') {
                break;
            }
        }
        state.next_expect(')')
    }

    /// `point(State, PointsBuilder)`: reads `x y`, skipping further
    /// coordinates.
    ///
    /// # Errors
    ///
    /// `ParseException` and the builder's errors.
    pub fn point(&self, state: &mut State, points_builder: &mut dyn PointsBuilder) -> Result<()> {
        let x = state.next_double()?;
        let y = state.next_double()?;
        state.skip_next_doubles(); //TODO capture to call pointXYZ
        let f = self.shape_factory();
        points_builder.point_xy(f.norm_x(x), f.norm_y(y))
    }

    /// `polygon(State, PolygonBuilder)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and the builder's errors.
    pub fn polygon(&self, state: &mut State, polygon_builder: &mut dyn PolygonBuilder) -> Result<()> {
        state.next_expect('(')?;
        self.point_list(state, polygon_builder)?; // outer ring
        while state.next_if(',') {
            let mut hole_builder = polygon_builder.hole();
            self.point_list(state, &mut *hole_builder)?;
            hole_builder.end_hole()?;
        }
        state.next_expect(')')
    }

    /// `readString(Reader reader)` (package-private).
    pub(crate) fn read_string(reader: &mut dyn std::io::Read) -> Result<String> {
        read_all(reader)
    }
}

impl ShapeIO for WKTReader {
    fn get_format_name(&self) -> &'static str {
        shape_io::WKT
    }
}

impl ShapeReader for WKTReader {
    fn read(&self, value: &str) -> Result<Option<Arc<dyn Shape>>> {
        Ok(Some(self.parse(value)?))
    }

    fn read_if_supported(&self, value: &str) -> Result<Option<Arc<dyn Shape>>> {
        match self.parse_if_supported(value) {
            Ok(s) => Ok(s),
            Err(e) if ParseException::of(&e).is_some() => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn read_reader(&self, reader: &mut dyn std::io::Read) -> Result<Option<Arc<dyn Shape>>> {
        let s = Self::read_string(reader)?;
        Ok(Some(self.parse(&s)?))
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Equivalent to `org.locationtech.spatial4j.io.WKTReader.State` (`public
/// class State`, an inner class): the parse state of one WKT string, as its
/// UTF-16 units. Its `getCtx()` and `getParser()` reach the enclosing
/// reader, which the parse methods hold here as `self`.
#[derive(Clone, Debug)]
pub struct State {
    /// `rawString`.
    pub raw_string: Vec<u16>,
    /// `offset`: the index into `rawString`.
    pub offset: usize,
    /// `dimension`: a dimension word (`Z`, `M`, `ZM`) if one was found.
    pub dimension: Option<String>,
}

impl State {
    /// `State(String rawString)`.
    pub fn new(raw_string: &str) -> Self {
        Self { raw_string: raw_string.encode_utf16().collect(), offset: 0, dimension: None }
    }

    /// `rawString.charAt(index)`, a lone surrogate as U+FFFD (which no
    /// predicate here accepts, as none accepts a surrogate in Java).
    fn char_at(&self, index: usize) -> char {
        char::from_u32(u32::from(self.raw_string[index])).unwrap_or('\u{FFFD}')
    }

    fn substring(&self, start: usize, end: usize) -> String {
        String::from_utf16_lossy(&self.raw_string[start..end])
    }

    /// `nextWord()`: reads the next word (Java identifier characters).
    ///
    /// # Errors
    ///
    /// `ParseException` (`Word expected`) when there is none.
    pub fn next_word(&mut self) -> Result<String> {
        let start_offset = self.offset;
        while self.offset < self.raw_string.len() && is_java_identifier_part(self.char_at(self.offset)) {
            self.offset += 1;
        }
        if start_offset == self.offset {
            return Err(ParseException::error("Word expected", start_offset as i32));
        }
        let result = self.substring(start_offset, self.offset);
        self.next_if_whitespace();
        Ok(result)
    }

    /// `nextIfEmptyAndSkipZM()`: `true` for `EMPTY`, skipping a dimension
    /// word first.
    ///
    /// # Errors
    ///
    /// `ParseException` for a dimension word not followed by `EMPTY` or `(`.
    pub fn next_if_empty_and_skip_zm(&mut self) -> Result<bool> {
        if self.eof() {
            return Ok(false);
        }
        let c = self.char_at(self.offset);
        if c == '(' || !is_java_identifier_part(c) {
            return Ok(false);
        }
        let word = self.next_word()?;
        if word.eq_ignore_ascii_case("EMPTY") {
            return Ok(true);
        }
        //we figure this word is Z or ZM or some other dimensionality signifier. We skip it.
        self.dimension = Some(word);

        if self.eof() {
            return Ok(false);
        }
        let c = self.char_at(self.offset);
        if c == '(' || !is_java_identifier_part(c) {
            return Ok(false);
        }
        let word = self.next_word()?;
        if word.eq_ignore_ascii_case("EMPTY") {
            return Ok(true);
        }
        Err(ParseException::error(
            format!("Expected EMPTY because found dimension; but got [{word}]"),
            self.offset as i32,
        ))
    }

    /// `nextDouble()`.
    ///
    /// # Errors
    ///
    /// `ParseException` when there is no number.
    pub fn next_double(&mut self) -> Result<f64> {
        let start_offset = self.offset;
        self.skip_double();
        if start_offset == self.offset {
            return Err(ParseException::error("Expected a number", self.offset as i32));
        }
        let result = match parse_double(&self.substring(start_offset, self.offset)) {
            Ok(v) => v,
            Err(e) => return Err(ParseException::error(e.to_string(), self.offset as i32)),
        };
        self.next_if_whitespace();
        Ok(result)
    }

    /// `skipDouble()`.
    pub fn skip_double(&mut self) {
        let start_offset = self.offset;
        while self.offset < self.raw_string.len() {
            let c = self.char_at(self.offset);
            if !(is_digit(c) || c == '.' || c == '-' || c == '+') {
                //'e' is okay as long as it isn't first
                if self.offset != start_offset && (c == 'e' || c == 'E') {
                    self.offset += 1;
                    continue;
                }
                break;
            }
            self.offset += 1;
        }
    }

    /// `skipNextDoubles()`.
    pub fn skip_next_doubles(&mut self) {
        while !self.eof() {
            let start_offset = self.offset;
            self.skip_double();
            if start_offset == self.offset {
                return;
            }
            self.next_if_whitespace();
        }
    }

    /// `nextExpect(char expected)`.
    ///
    /// # Errors
    ///
    /// `ParseException` when the next character is not `expected`.
    pub fn next_expect(&mut self, expected: char) -> Result<()> {
        if self.eof() {
            return Err(ParseException::error(format!("Expected [{expected}] found EOF"), self.offset as i32));
        }
        let c = self.char_at(self.offset);
        if c != expected {
            return Err(ParseException::error(
                format!("Expected [{expected}] found [{}]", String::from_utf16_lossy(&[self.raw_string[self.offset]])),
                self.offset as i32,
            ));
        }
        self.offset += 1;
        self.next_if_whitespace();
        Ok(())
    }

    /// `eof()`.
    pub fn eof(&self) -> bool {
        self.offset >= self.raw_string.len()
    }

    /// `nextIf(char expected)`.
    pub fn next_if(&mut self, expected: char) -> bool {
        if !self.eof() && self.char_at(self.offset) == expected {
            self.offset += 1;
            self.next_if_whitespace();
            return true;
        }
        false
    }

    /// `nextIfWhitespace()`.
    pub fn next_if_whitespace(&mut self) {
        while self.offset < self.raw_string.len() {
            if !is_whitespace(self.char_at(self.offset)) {
                return;
            }
            self.offset += 1;
        }
    }

    /// `nextSubShapeString()`.
    ///
    /// # Errors
    ///
    /// `ParseException` for unbalanced parentheses.
    pub fn next_sub_shape_string(&mut self) -> Result<String> {
        let start_offset = self.offset;
        let mut paren_stack = 0; // how many parenthesis levels are we in?
        while self.offset < self.raw_string.len() {
            let c = self.char_at(self.offset);
            if c == ',' {
                if paren_stack == 0 {
                    break;
                }
            } else if c == ')' {
                if paren_stack == 0 {
                    break;
                }
                paren_stack -= 1;
            } else if c == '(' {
                paren_stack += 1;
            }
            self.offset += 1;
        }
        if paren_stack != 0 {
            return Err(ParseException::error("Unbalanced parenthesis", start_offset as i32));
        }
        Ok(self.substring(start_offset, self.offset))
    }
}
