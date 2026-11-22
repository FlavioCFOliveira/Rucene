/*******************************************************************************
 * Copyright (c) 2015 MITRE and VoyagerSearch
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `SpatialContextFactory.java`, of the Java package
//! `org.locationtech.spatial4j.context`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use rucene::error::{LuceneError, Result};

use crate::context::spatial_context::SpatialContext;
use crate::distance::cartesian_dist_calc::CartesianDistCalc;
use crate::distance::distance_calculator::DistanceCalculator;
use crate::distance::geodesic_sphere_dist_calc::{Haversine, LawOfCosines, Vincenty};
use crate::io::binary_codec::{BinaryCodec, BinaryCodecBase};
use crate::io::geo_json_writer::GeoJSONWriter;
use crate::io::legacy_shape_reader::LegacyShapeReader;
use crate::io::legacy_shape_writer::LegacyShapeWriter;
use crate::io::polyshape_reader::PolyshapeReader;
use crate::io::polyshape_writer::PolyshapeWriter;
use crate::io::shape_reader::ShapeReader;
use crate::io::shape_writer::ShapeWriter;
use crate::io::supported_formats::SupportedFormats;
use crate::io::wkt_reader::WKTReader;
use crate::io::wkt_writer::WKTWriter;
use crate::jdk::unchecked_with_cause;
use crate::shape::r#impl::shape_factory_impl::ShapeFactoryImpl;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape_factory::ShapeFactory;

/// The constructor `makeClassInstance` would call: one taking `(ctx,
/// factory)`, or the empty one.
pub type Ctor<T> = fn(&Arc<SpatialContext>, &dyn SpatialContextFactoryOps) -> Result<Arc<T>>;

/// What a registered class is, and how to instantiate it.
#[derive(Clone, Copy)]
pub enum JavaClassKind {
    /// A `ShapeFactory` implementation.
    ShapeFactory(Ctor<dyn ShapeFactory>),
    /// A `BinaryCodec` (sub)class.
    BinaryCodec(Ctor<dyn BinaryCodec>),
    /// A `ShapeReader` implementation.
    ShapeReader(Ctor<dyn ShapeReader>),
    /// A `ShapeWriter` implementation.
    ShapeWriter(Ctor<dyn ShapeWriter>),
    /// A `SpatialContextFactory` (sub)class, with its no-argument
    /// constructor.
    SpatialContextFactory(fn() -> Box<dyn SpatialContextFactoryOps>),
    /// A class of none of those kinds.
    Other,
}

/// A Java class, by binary name, as `Class.forName` would load it.
///
/// **Divergence.** Java loads the classes named in a factory's arguments by
/// reflection (`ClassLoader.loadClass`, `Class.forName`) and instantiates
/// them by their constructors (`SpatialContextFactory.java:115-207,
/// 296-327`). Rust has no reflection: a class is found in a process-wide
/// registry ([`register_class`], [`load_class`]) that this crate fills with
/// its own classes, and that a crate with more classes (Lucene's
/// spatial-extras) adds to before their names are looked up. The core's
/// `rucene::util::resource_loader::ClassRegistry` does not fit here: it is
/// a per-instance table of one result type built by argument-less
/// factories, reporting `Cannot load class`, while `Class.forName` here is
/// process-wide, spans five kinds of class whose constructors take `(ctx,
/// factory)`, and fails with `ClassNotFoundException` naming the class.
#[derive(Clone, Copy)]
pub struct JavaClass {
    /// `getName()`.
    pub name: &'static str,
    /// The kind and constructor.
    pub kind: JavaClassKind,
}

impl std::fmt::Debug for JavaClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JavaClass").field("name", &self.name).finish()
    }
}

impl PartialEq for JavaClass {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl std::fmt::Display for JavaClass {
    /// `Class.toString()`: `class <name>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "class {}", self.name)
    }
}

static REGISTRY: LazyLock<RwLock<HashMap<&'static str, JavaClass>>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    for class in builtin_classes() {
        map.insert(class.name, class);
    }
    RwLock::new(map)
});

fn builtin_classes() -> Vec<JavaClass> {
    vec![
        SpatialContextFactory::SHAPE_FACTORY_IMPL_CLASS,
        SpatialContextFactory::BINARY_CODEC_CLASS,
        SpatialContextFactory::WKT_READER_CLASS,
        SpatialContextFactory::POLYSHAPE_READER_CLASS,
        SpatialContextFactory::LEGACY_SHAPE_READER_CLASS,
        SpatialContextFactory::GEO_JSON_WRITER_CLASS,
        SpatialContextFactory::WKT_WRITER_CLASS,
        SpatialContextFactory::POLYSHAPE_WRITER_CLASS,
        SpatialContextFactory::LEGACY_SHAPE_WRITER_CLASS,
        SpatialContextFactory::SPATIAL_CONTEXT_FACTORY_CLASS,
    ]
}

/// Adds `class` to the classes [`load_class`] finds.
pub fn register_class(class: JavaClass) {
    REGISTRY.write().unwrap_or_else(|e| e.into_inner()).insert(class.name, class);
}

/// `Class.forName(name)`.
///
/// # Errors
///
/// `ClassNotFoundException` (its message the name) for an unknown class.
pub fn load_class(name: &str) -> Result<JavaClass> {
    REGISTRY
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .copied()
        .ok_or_else(|| class_not_found(name))
}

fn class_not_found(name: &str) -> LuceneError {
    unchecked_with_cause("java.lang.ClassNotFoundException", Some(name.to_string()), None)
}

/// `new RuntimeException(cause)`: its message is `cause.toString()`.
fn runtime_exception(cause: LuceneError) -> LuceneError {
    LuceneError::runtime_exception(cause)
}

/// `Class.asSubclass` failing: `ClassCastException(this.toString())`.
fn class_cast(class: &JavaClass) -> LuceneError {
    LuceneError::unchecked("java.lang.ClassCastException", class.to_string())
}

/// Equivalent to `org.locationtech.spatial4j.context.SpatialContextFactory`
/// (`public class`): the configurable factory of a [`SpatialContext`]. Its
/// public fields are the fields here; its overridable methods are the
/// [`SpatialContextFactoryOps`] trait, which a subclass (Lucene's
/// `Geo3dSpatialContextFactory`) implements over an embedded instance.
#[derive(Clone, Debug)]
pub struct SpatialContextFactory {
    /// `args`: the arguments given to `init`.
    pub args: Option<HashMap<String, String>>,
    /// `geo`.
    pub geo: bool,
    /// `distCalc`: defaults in the `SpatialContext` constructor, based on
    /// `geo`.
    pub dist_calc: Option<Arc<dyn DistanceCalculator>>,
    /// `worldBounds`: defaults in the `SpatialContext` constructor, based on
    /// `geo`.
    pub world_bounds: Option<Arc<dyn Rectangle>>,
    /// `normWrapLongitude`.
    pub norm_wrap_longitude: bool,
    /// `shapeFactoryClass`.
    pub shape_factory_class: JavaClass,
    /// `binaryCodecClass`.
    pub binary_codec_class: JavaClass,
    /// `readers`.
    pub readers: Vec<JavaClass>,
    /// `writers`.
    pub writers: Vec<JavaClass>,
    /// `hasFormatConfig`.
    pub has_format_config: bool,
}

impl Default for SpatialContextFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl SpatialContextFactory {
    /// `ShapeFactoryImpl.class`.
    pub const SHAPE_FACTORY_IMPL_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.shape.impl.ShapeFactoryImpl",
        kind: JavaClassKind::ShapeFactory(|ctx, factory| Ok(Arc::new(ShapeFactoryImpl::new(ctx, factory)))),
    };
    /// `BinaryCodec.class`.
    pub const BINARY_CODEC_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.io.BinaryCodec",
        kind: JavaClassKind::BinaryCodec(|ctx, factory| Ok(Arc::new(BinaryCodecBase::new(ctx, factory)))),
    };
    /// `WKTReader.class`.
    pub const WKT_READER_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.io.WKTReader",
        kind: JavaClassKind::ShapeReader(|ctx, factory| Ok(Arc::new(WKTReader::new(ctx, factory)))),
    };
    /// `PolyshapeReader.class`.
    pub const POLYSHAPE_READER_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.io.PolyshapeReader",
        kind: JavaClassKind::ShapeReader(|ctx, factory| Ok(Arc::new(PolyshapeReader::new(ctx, factory)))),
    };
    /// `LegacyShapeReader.class`.
    pub const LEGACY_SHAPE_READER_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.io.LegacyShapeReader",
        kind: JavaClassKind::ShapeReader(|ctx, factory| Ok(Arc::new(LegacyShapeReader::new(ctx, factory)))),
    };
    /// `GeoJSONReader.class`: not ported (see
    /// [`geo_json_reader`](crate::io::geo_json_reader)); its name only.
    pub const GEO_JSON_READER_CLASS: JavaClass =
        JavaClass { name: "org.locationtech.spatial4j.io.GeoJSONReader", kind: JavaClassKind::Other };
    /// `GeoJSONWriter.class`.
    pub const GEO_JSON_WRITER_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.io.GeoJSONWriter",
        kind: JavaClassKind::ShapeWriter(|ctx, factory| Ok(Arc::new(GeoJSONWriter::new(ctx, factory)))),
    };
    /// `WKTWriter.class` (its empty constructor).
    pub const WKT_WRITER_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.io.WKTWriter",
        kind: JavaClassKind::ShapeWriter(|_, _| Ok(Arc::new(WKTWriter::new()))),
    };
    /// `PolyshapeWriter.class`.
    pub const POLYSHAPE_WRITER_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.io.PolyshapeWriter",
        kind: JavaClassKind::ShapeWriter(|ctx, factory| Ok(Arc::new(PolyshapeWriter::new(ctx, factory)))),
    };
    /// `LegacyShapeWriter.class`.
    pub const LEGACY_SHAPE_WRITER_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.io.LegacyShapeWriter",
        kind: JavaClassKind::ShapeWriter(|ctx, factory| Ok(Arc::new(LegacyShapeWriter::new(ctx, factory)))),
    };
    /// `SpatialContextFactory.class`.
    pub const SPATIAL_CONTEXT_FACTORY_CLASS: JavaClass = JavaClass {
        name: "org.locationtech.spatial4j.context.SpatialContextFactory",
        kind: JavaClassKind::SpatialContextFactory(|| Box::new(SpatialContextFactory::new())),
    };

    /// `SpatialContextFactory()`.
    pub fn new() -> Self {
        Self {
            args: None,
            geo: true,
            dist_calc: None,
            world_bounds: None,
            norm_wrap_longitude: false,
            shape_factory_class: Self::SHAPE_FACTORY_IMPL_CLASS,
            binary_codec_class: Self::BINARY_CODEC_CLASS,
            readers: Vec::new(),
            writers: Vec::new(),
            has_format_config: false,
        }
    }

    /// `makeSpatialContext(Map<String,String> args, ClassLoader
    /// classLoader)`: instantiates the factory class named by the
    /// `spatialContextFactory` argument (or the `SpatialContextFactory`
    /// system property; this one by default), initialises it with `args`
    /// and returns its context.
    ///
    /// # Errors
    ///
    /// `RuntimeException` when the factory class cannot be instantiated; the
    /// errors of `init` and `newSpatialContext`.
    pub fn make_spatial_context(args: &HashMap<String, String>) -> Result<Arc<SpatialContext>> {
        let mut cname = args.get("spatialContextFactory").cloned();
        if cname.is_none() {
            cname = std::env::var("SpatialContextFactory").ok();
        }
        let mut instance: Box<dyn SpatialContextFactoryOps> = match cname {
            None => Box::new(SpatialContextFactory::new()),
            Some(cname) => {
                let class = load_class(&cname).map_err(runtime_exception)?;
                match class.kind {
                    JavaClassKind::SpatialContextFactory(ctor) => ctor(),
                    _ => return Err(runtime_exception(class_cast(&class))),
                }
            }
        };
        instance.init(args.clone())?;
        instance.new_spatial_context()
    }
}

/// The overridable methods of `SpatialContextFactory`, over the fields of an
/// embedded [`SpatialContextFactory`].
pub trait SpatialContextFactoryOps: Send + Sync + Any {
    /// The `SpatialContextFactory` fields.
    fn base(&self) -> &SpatialContextFactory;

    /// The `SpatialContextFactory` fields, mutably.
    fn base_mut(&mut self) -> &mut SpatialContextFactory;

    /// This factory as `Any`, for casts to a subclass.
    fn as_any(&self) -> &dyn Any;

    /// `init(Map<String, String> args, ClassLoader classLoader)`.
    ///
    /// # Errors
    ///
    /// The errors of the `init*` steps.
    fn init(&mut self, args: HashMap<String, String>) -> Result<()> {
        init_default(self, args)
    }

    /// `initField(String name)`: sets the public field `name` from the
    /// argument of the same name, when present.
    ///
    /// # Errors
    ///
    /// `RuntimeException` (`Invalid value '...' on field ... of type ...`)
    /// for an argument that does not parse; `java.lang.Error` for a name
    /// that is not a field.
    fn init_field(&mut self, name: &str) -> Result<()> {
        init_field_default(self, name)
    }

    /// `initCalculator()`.
    ///
    /// # Errors
    ///
    /// `RuntimeException` for an unknown calculator name.
    fn init_calculator(&mut self) -> Result<()> {
        init_calculator_default(self)
    }

    /// `initFormats()`.
    ///
    /// # Errors
    ///
    /// `RuntimeException` when a format class cannot be found.
    fn init_formats(&mut self) -> Result<()> {
        init_formats_default(self)
    }

    /// `makeFormats(SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// `RuntimeException` when a format cannot be instantiated.
    fn make_formats(&mut self, ctx: &Arc<SpatialContext>) -> Result<SupportedFormats> {
        make_formats_default(self, ctx)
    }

    /// `checkDefaultFormats()`.
    fn check_default_formats(&mut self) {
        check_default_formats_default(self);
    }

    /// `addReaderIfNoggitExists(Class<? extends ShapeReader> reader)`: adds
    /// `reader` only when `org.noggit.JSONParser` loads.
    fn add_reader_if_noggit_exists(&mut self, reader: JavaClass) {
        if load_class("org.noggit.JSONParser").is_ok() {
            self.base_mut().readers.push(reader);
        }
    }

    /// `initWorldBounds()`.
    ///
    /// # Errors
    ///
    /// The errors of `newSpatialContext` and of reading the bounds.
    fn init_world_bounds(&mut self) -> Result<()> {
        init_world_bounds_default(self)
    }

    /// `newSpatialContext()`.
    ///
    /// # Errors
    ///
    /// The errors of the `SpatialContext` constructor.
    fn new_spatial_context(&mut self) -> Result<Arc<SpatialContext>> {
        SpatialContext::new(self.as_ops_mut())
    }

    /// `makeShapeFactory(SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// `RuntimeException` when the class cannot be instantiated.
    fn make_shape_factory(&self, ctx: &Arc<SpatialContext>) -> Result<Arc<dyn ShapeFactory>> {
        let class = self.base().shape_factory_class;
        match class.kind {
            JavaClassKind::ShapeFactory(ctor) => ctor(ctx, self.as_ops()).map_err(runtime_exception),
            _ => Err(class_cast(&class)),
        }
    }

    /// `makeBinaryCodec(SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// `RuntimeException` when the class cannot be instantiated.
    fn make_binary_codec(&self, ctx: &Arc<SpatialContext>) -> Result<Arc<dyn BinaryCodec>> {
        let class = self.base().binary_codec_class;
        match class.kind {
            JavaClassKind::BinaryCodec(ctor) => ctor(ctx, self.as_ops()).map_err(runtime_exception),
            _ => Err(class_cast(&class)),
        }
    }

    /// This factory as the trait object.
    fn as_ops(&self) -> &dyn SpatialContextFactoryOps;

    /// This factory as the mutable trait object.
    fn as_ops_mut(&mut self) -> &mut dyn SpatialContextFactoryOps;
}

/// `SpatialContextFactory.init` (the body a subclass reaches with `super`).
///
/// # Errors
///
/// The errors of the `init*` steps.
pub fn init_default<F: SpatialContextFactoryOps + ?Sized>(f: &mut F, args: HashMap<String, String>) -> Result<()> {
    f.base_mut().args = Some(args);
    f.init_field("geo")?;
    f.init_field("shapeFactoryClass")?;
    f.init_calculator()?;
    f.init_formats()?;
    f.init_world_bounds()?;
    f.init_field("normWrapLongitude")?;
    f.init_field("binaryCodecClass")?;
    Ok(())
}

/// `SpatialContextFactory.initField`.
///
/// # Errors
///
/// As [`SpatialContextFactoryOps::init_field`].
pub fn init_field_default<F: SpatialContextFactoryOps + ?Sized>(f: &mut F, name: &str) -> Result<()> {
    // `getClass().getField(name)`: the public fields of the class.
    let field_type = match name {
        "geo" | "normWrapLongitude" | "hasFormatConfig" => "boolean",
        "shapeFactoryClass" | "binaryCodecClass" => "class java.lang.Class",
        "distCalc" => "interface org.locationtech.spatial4j.distance.DistanceCalculator",
        "worldBounds" => "interface org.locationtech.spatial4j.shape.Rectangle",
        "readers" | "writers" => "interface java.util.List",
        _ => {
            return Err(LuceneError::unchecked(
                "java.lang.Error",
                format!("java.lang.NoSuchFieldException: {name}"),
            ))
        }
    };
    let Some(str) = f.base().args.as_ref().and_then(|a| a.get(name)).cloned() else {
        return Ok(());
    };
    let invalid = |cause: LuceneError| {
        unchecked_with_cause(
            "java.lang.RuntimeException",
            Some(format!("Invalid value '{str}' on field {name} of type {field_type}")),
            Some(cause),
        )
    };
    match field_type {
        "boolean" => {
            // `Boolean.valueOf(str)`
            let v = str.eq_ignore_ascii_case("true");
            let base = f.base_mut();
            match name {
                "geo" => base.geo = v,
                "normWrapLongitude" => base.norm_wrap_longitude = v,
                _ => base.has_format_config = v,
            }
            Ok(())
        }
        "class java.lang.Class" => {
            let class = load_class(&str).map_err(|e| invalid(runtime_exception(e)))?;
            let base = f.base_mut();
            if name == "shapeFactoryClass" {
                base.shape_factory_class = class;
            } else {
                base.binary_codec_class = class;
            }
            Ok(())
        }
        _ => Err(LuceneError::unchecked(
            "java.lang.Error",
            format!("unsupported field type: {field_type}"),
        )),
    }
}

/// `SpatialContextFactory.initCalculator`.
///
/// # Errors
///
/// `RuntimeException` for an unknown calculator name.
pub fn init_calculator_default<F: SpatialContextFactoryOps + ?Sized>(f: &mut F) -> Result<()> {
    let Some(calc_str) = f.base().args.as_ref().and_then(|a| a.get("distCalculator")).cloned() else {
        return Ok(());
    };
    let calc: Arc<dyn DistanceCalculator> = if calc_str.eq_ignore_ascii_case("haversine") {
        Arc::new(Haversine)
    } else if calc_str.eq_ignore_ascii_case("lawOfCosines") {
        Arc::new(LawOfCosines)
    } else if calc_str.eq_ignore_ascii_case("vincentySphere") {
        Arc::new(Vincenty)
    } else if calc_str.eq_ignore_ascii_case("cartesian") {
        Arc::new(CartesianDistCalc::new())
    } else if calc_str.eq_ignore_ascii_case("cartesian^2") {
        Arc::new(CartesianDistCalc::with_squared(true))
    } else {
        return Err(LuceneError::unchecked(
            "java.lang.RuntimeException",
            format!("Unknown calculator: {calc_str}"),
        ));
    };
    f.base_mut().dist_calc = Some(calc);
    Ok(())
}

/// `SpatialContextFactory.initFormats`.
///
/// # Errors
///
/// `RuntimeException` (`Unable to find format class`) when a class cannot be
/// found; `ClassCastException` when it is not a reader or writer.
pub fn init_formats_default<F: SpatialContextFactoryOps + ?Sized>(f: &mut F) -> Result<()> {
    let not_found = |e: LuceneError| {
        unchecked_with_cause("java.lang.RuntimeException", Some("Unable to find format class".to_string()), Some(e))
    };
    let args = f.base().args.clone().unwrap_or_default();
    let reader_class = |name: &str| -> Result<JavaClass> {
        let class = load_class(name.trim()).map_err(not_found)?;
        match class.kind {
            JavaClassKind::ShapeReader(_) => Ok(class),
            _ => Err(class_cast(&class)),
        }
    };
    if let Some(val) = args.get("readers") {
        for name in val.split(',') {
            let class = reader_class(name)?;
            f.base_mut().readers.push(class);
        }
    } else if let Some(val) = args.get("wktShapeParserClass") {
        //deprecated; a parameter from when this was a raw class
        let class = reader_class(val)?;
        f.base_mut().readers.push(class);
    }
    if let Some(val) = args.get("writers") {
        for name in val.split(',') {
            let class = load_class(name.trim()).map_err(not_found)?;
            match class.kind {
                JavaClassKind::ShapeWriter(_) => f.base_mut().writers.push(class),
                _ => return Err(class_cast(&class)),
            }
        }
    }
    Ok(())
}

/// `SpatialContextFactory.makeFormats`.
///
/// # Errors
///
/// `RuntimeException` when a format cannot be instantiated.
pub fn make_formats_default<F: SpatialContextFactoryOps + ?Sized>(
    f: &mut F,
    ctx: &Arc<SpatialContext>,
) -> Result<SupportedFormats> {
    f.check_default_formats(); // easy to override
    let mut read: Vec<Arc<dyn ShapeReader>> = Vec::with_capacity(f.base().readers.len());
    for class in f.base().readers.clone() {
        match class.kind {
            JavaClassKind::ShapeReader(ctor) => read.push(ctor(ctx, f.as_ops()).map_err(runtime_exception)?),
            _ => return Err(runtime_exception(class_cast(&class))),
        }
    }
    let mut write: Vec<Arc<dyn ShapeWriter>> = Vec::with_capacity(f.base().writers.len());
    for class in f.base().writers.clone() {
        match class.kind {
            JavaClassKind::ShapeWriter(ctor) => write.push(ctor(ctx, f.as_ops()).map_err(runtime_exception)?),
            _ => return Err(runtime_exception(class_cast(&class))),
        }
    }
    Ok(SupportedFormats::new(read, write))
}

/// `SpatialContextFactory.checkDefaultFormats`.
pub fn check_default_formats_default<F: SpatialContextFactoryOps + ?Sized>(f: &mut F) {
    if f.base().readers.is_empty() {
        f.add_reader_if_noggit_exists(SpatialContextFactory::GEO_JSON_READER_CLASS);
        let base = f.base_mut();
        base.readers.push(SpatialContextFactory::WKT_READER_CLASS);
        base.readers.push(SpatialContextFactory::POLYSHAPE_READER_CLASS);
        base.readers.push(SpatialContextFactory::LEGACY_SHAPE_READER_CLASS);
    }
    let base = f.base_mut();
    if base.writers.is_empty() {
        base.writers.push(SpatialContextFactory::GEO_JSON_WRITER_CLASS);
        base.writers.push(SpatialContextFactory::WKT_WRITER_CLASS);
        base.writers.push(SpatialContextFactory::POLYSHAPE_WRITER_CLASS);
        base.writers.push(SpatialContextFactory::LEGACY_SHAPE_WRITER_CLASS);
    }
}

/// `SpatialContextFactory.initWorldBounds`.
///
/// # Errors
///
/// The errors of `newSpatialContext`; `ClassCastException` when the bounds
/// do not read as a rectangle.
pub fn init_world_bounds_default<F: SpatialContextFactoryOps + ?Sized>(f: &mut F) -> Result<()> {
    let Some(world_bounds_str) = f.base().args.as_ref().and_then(|a| a.get("worldBounds")).cloned() else {
        return Ok(());
    };
    //kinda ugly we do this just to read a rectangle.  TODO refactor
    let ctx = f.new_spatial_context()?;
    let shape = ctx.read_shape(&world_bounds_str)?; //TODO use readShapeFromWkt
    let Some(shape) = shape else {
        f.base_mut().world_bounds = None;
        return Ok(());
    };
    // Java casts the shape itself; it is copied here, with its coordinates
    // unchanged, since the shape is shared as `Arc<dyn Shape>`.
    if shape.as_rectangle().is_none() {
        return Err(LuceneError::unchecked(
            "java.lang.ClassCastException",
            format!(
                "class {} cannot be cast to class org.locationtech.spatial4j.shape.Rectangle",
                shape.java_class_name()
            ),
        ));
    }
    let rect = shape.as_rectangle().expect("checked above");
    let bounds: Arc<dyn Rectangle> = crate::shape::r#impl::rectangle_impl::RectangleImpl::from_rect(
        rect,
        Some(crate::context::spatial_context::SpatialContextRef::from(&ctx)),
    );
    f.base_mut().world_bounds = Some(bounds);
    Ok(())
}

impl SpatialContextFactoryOps for SpatialContextFactory {
    fn base(&self) -> &SpatialContextFactory {
        self
    }

    fn base_mut(&mut self) -> &mut SpatialContextFactory {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_ops(&self) -> &dyn SpatialContextFactoryOps {
        self
    }

    fn as_ops_mut(&mut self) -> &mut dyn SpatialContextFactoryOps {
        self
    }
}
