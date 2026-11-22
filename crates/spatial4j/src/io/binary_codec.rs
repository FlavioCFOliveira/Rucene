/*******************************************************************************
 * Copyright (c) 2015 MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `BinaryCodec.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;
use std::io::{Read, Write};
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use crate::context::spatial_context::SpatialContext;
use crate::context::spatial_context_factory::SpatialContextFactoryOps;
use crate::exception::invalid_shape_exception::InvalidShapeException;
use crate::shape::circle::Circle;
use crate::shape::point::Point;
use crate::shape::rectangle::Rectangle;
use crate::shape::shape::Shape;
use crate::shape::shape_collection::ShapeCollection;

/// `DataInput.readByte()`.
pub(crate) fn read_byte(input: &mut dyn Read) -> Result<i8> {
    let mut b = [0u8; 1];
    input.read_exact(&mut b)?;
    Ok(b[0] as i8)
}

/// `DataInput.readInt()`: big-endian.
pub(crate) fn read_int(input: &mut dyn Read) -> Result<i32> {
    let mut b = [0u8; 4];
    input.read_exact(&mut b)?;
    Ok(i32::from_be_bytes(b))
}

/// `DataInput.readDouble()`: big-endian IEEE bits.
pub(crate) fn read_double(input: &mut dyn Read) -> Result<f64> {
    let mut b = [0u8; 8];
    input.read_exact(&mut b)?;
    Ok(f64::from_bits(u64::from_be_bytes(b)))
}

/// `DataOutput.writeByte(int)`.
pub(crate) fn write_byte(output: &mut dyn Write, v: i8) -> Result<()> {
    output.write_all(&[v as u8])?;
    Ok(())
}

/// `DataOutput.writeInt(int)`: big-endian.
pub(crate) fn write_int(output: &mut dyn Write, v: i32) -> Result<()> {
    output.write_all(&v.to_be_bytes())?;
    Ok(())
}

/// `DataOutput.writeDouble(double)`: `doubleToLongBits`, big-endian.
pub(crate) fn write_double(output: &mut dyn Write, v: f64) -> Result<()> {
    output.write_all(&(crate::jdk::double_to_long_bits(v) as u64).to_be_bytes())?;
    Ok(())
}

/// Equivalent to `org.locationtech.spatial4j.io.BinaryCodec` (`public
/// class`): a binary codec for shapes, over `java.io.DataInput` and
/// `DataOutput` (big-endian streams here).
///
/// Java's class is subclassed (Lucene's `Geo3dBinaryCodec`); its methods
/// are this trait's, with Java's bodies as the default methods, and the
/// plain class is [`BinaryCodecBase`].
pub trait BinaryCodec: Send + Sync + Any {
    /// `ctx`.
    fn ctx(&self) -> Arc<SpatialContext>;

    /// This codec as `Any`.
    fn as_any(&self) -> &dyn Any;

    /// `readShape(DataInput)`.
    ///
    /// # Errors
    ///
    /// `IOException`; `IllegalArgumentException` for an unknown type byte.
    fn read_shape(&self, data_input: &mut dyn Read) -> Result<Arc<dyn Shape>> {
        let type_ = read_byte(data_input)?;
        let s = self.read_shape_by_type_if_supported(data_input, type_)?;
        s.ok_or_else(|| LuceneError::illegal_argument(format!("Unsupported shape byte {type_}")))
    }

    /// `writeShape(DataOutput, Shape)`.
    ///
    /// # Errors
    ///
    /// `IOException`; `IllegalArgumentException` for an unsupported shape.
    fn write_shape(&self, data_output: &mut dyn Write, s: &dyn Shape) -> Result<()> {
        let written = self.write_shape_by_type_if_supported(data_output, s)?;
        if !written {
            return Err(LuceneError::illegal_argument(format!(
                "Unsupported shape class {}",
                s.java_class_name()
            )));
        }
        Ok(())
    }

    /// `readShapeByTypeIfSupported(DataInput, byte)`.
    ///
    /// # Errors
    ///
    /// `IOException`.
    fn read_shape_by_type_if_supported(&self, data_input: &mut dyn Read, type_: i8) -> Result<Option<Arc<dyn Shape>>> {
        Ok(match type_ {
            BinaryCodecBase::TYPE_POINT => Some(self.read_point(data_input)?),
            BinaryCodecBase::TYPE_RECT => Some(self.read_rect(data_input)?),
            BinaryCodecBase::TYPE_CIRCLE => Some(self.read_circle(data_input)?),
            BinaryCodecBase::TYPE_COLL => Some(self.read_collection(data_input)?),
            _ => None,
        })
    }

    /// `writeShapeByTypeIfSupported(DataOutput, Shape)`.
    ///
    /// # Errors
    ///
    /// `IOException`.
    fn write_shape_by_type_if_supported(&self, data_output: &mut dyn Write, s: &dyn Shape) -> Result<bool> {
        let type_ = self.type_for_shape(s);
        write_byte(data_output, type_)?;
        self.write_shape_by_type_if_supported_typed(data_output, s, type_)
    }

    /// `writeShapeByTypeIfSupported(DataOutput, Shape, byte)`.
    ///
    /// # Errors
    ///
    /// `IOException`.
    fn write_shape_by_type_if_supported_typed(&self, data_output: &mut dyn Write, s: &dyn Shape, type_: i8) -> Result<bool> {
        let cast = |what: &str| {
            LuceneError::unchecked(
                "java.lang.ClassCastException",
                format!("class {} cannot be cast to {what}", s.java_class_name()),
            )
        };
        match type_ {
            BinaryCodecBase::TYPE_POINT => {
                self.write_point(data_output, s.as_point().ok_or_else(|| cast("Point"))?)?
            }
            BinaryCodecBase::TYPE_RECT => {
                self.write_rect(data_output, s.as_rectangle().ok_or_else(|| cast("Rectangle"))?)?
            }
            BinaryCodecBase::TYPE_CIRCLE => {
                self.write_circle(data_output, s.as_circle().ok_or_else(|| cast("Circle"))?)?
            }
            BinaryCodecBase::TYPE_COLL => self.write_collection(
                data_output,
                s.as_shape_collection().ok_or_else(|| cast("ShapeCollection"))?,
            )?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `typeForShape(Shape)`.
    fn type_for_shape(&self, s: &dyn Shape) -> i8 {
        if s.as_point().is_some() {
            BinaryCodecBase::TYPE_POINT
        } else if s.as_rectangle().is_some() {
            BinaryCodecBase::TYPE_RECT
        } else if s.as_circle().is_some() {
            BinaryCodecBase::TYPE_CIRCLE
        } else if s.as_shape_collection().is_some() {
            BinaryCodecBase::TYPE_COLL
        } else {
            0
        }
    }

    /// `readDim(DataInput)`.
    ///
    /// # Errors
    ///
    /// `IOException`.
    fn read_dim(&self, data_input: &mut dyn Read) -> Result<f64> {
        read_double(data_input)
    }

    /// `writeDim(DataOutput, double)`.
    ///
    /// # Errors
    ///
    /// `IOException`.
    fn write_dim(&self, data_output: &mut dyn Write, v: f64) -> Result<()> {
        write_double(data_output, v)
    }

    /// `readPoint(DataInput)`.
    ///
    /// # Errors
    ///
    /// `IOException`; `InvalidShapeException`.
    fn read_point(&self, data_input: &mut dyn Read) -> Result<Arc<dyn Point>> {
        let x = self.read_dim(data_input)?;
        let y = self.read_dim(data_input)?;
        self.ctx().make_point(x, y)
    }

    /// `writePoint(DataOutput, Point)`.
    ///
    /// # Errors
    ///
    /// `IOException`.
    fn write_point(&self, data_output: &mut dyn Write, pt: &dyn Point) -> Result<()> {
        self.write_dim(data_output, pt.get_x())?;
        self.write_dim(data_output, pt.get_y())
    }

    /// `readRect(DataInput)`.
    ///
    /// # Errors
    ///
    /// `IOException`; `InvalidShapeException`.
    fn read_rect(&self, data_input: &mut dyn Read) -> Result<Arc<dyn Rectangle>> {
        let min_x = self.read_dim(data_input)?;
        let max_x = self.read_dim(data_input)?;
        let min_y = self.read_dim(data_input)?;
        let max_y = self.read_dim(data_input)?;
        self.ctx().make_rectangle(min_x, max_x, min_y, max_y)
    }

    /// `writeRect(DataOutput, Rectangle)`.
    ///
    /// # Errors
    ///
    /// `IOException`.
    fn write_rect(&self, data_output: &mut dyn Write, r: &dyn Rectangle) -> Result<()> {
        self.write_dim(data_output, r.get_min_x())?;
        self.write_dim(data_output, r.get_max_x())?;
        self.write_dim(data_output, r.get_min_y())?;
        self.write_dim(data_output, r.get_max_y())
    }

    /// `readCircle(DataInput)`.
    ///
    /// # Errors
    ///
    /// `IOException`; `InvalidShapeException`.
    fn read_circle(&self, data_input: &mut dyn Read) -> Result<Arc<dyn Circle>> {
        let p = self.read_point(data_input)?;
        let d = self.read_dim(data_input)?;
        self.ctx().make_circle_at(p, d)
    }

    /// `writeCircle(DataOutput, Circle)`.
    ///
    /// # Errors
    ///
    /// `IOException`.
    fn write_circle(&self, data_output: &mut dyn Write, c: &dyn Circle) -> Result<()> {
        self.write_point(data_output, &*c.get_center())?;
        self.write_dim(data_output, c.get_radius())
    }

    /// `readCollection(DataInput)`.
    ///
    /// # Errors
    ///
    /// `IOException`; `InvalidShapeException` for an unsupported type byte.
    fn read_collection(&self, data_input: &mut dyn Read) -> Result<Arc<dyn Shape>> {
        let type_ = read_byte(data_input)?;
        let size = read_int(data_input)?;
        let mut shapes: Vec<Arc<dyn Shape>> = Vec::with_capacity(size.max(0) as usize);
        for _ in 0..size {
            if type_ == 0 {
                shapes.push(self.read_shape(data_input)?);
            } else {
                let s = self.read_shape_by_type_if_supported(data_input, type_)?;
                let Some(s) = s else {
                    return Err(InvalidShapeException::error(format!("Unsupported shape byte {type_}")));
                };
                shapes.push(s);
            }
        }
        let coll: Arc<ShapeCollection> = self.ctx().make_collection(shapes)?;
        Ok(coll)
    }

    /// `writeCollection(DataOutput, ShapeCollection)`.
    ///
    /// # Errors
    ///
    /// `IOException`; `IllegalArgumentException` for an unsupported shape.
    fn write_collection(&self, data_output: &mut dyn Write, col: &ShapeCollection) -> Result<()> {
        let type_: i8 = 0; //TODO add type to ShapeCollection
        write_byte(data_output, type_)?;
        write_int(data_output, col.size() as i32)?;
        for i in 0..col.size() {
            let s = col.get(i);
            if type_ == 0 {
                self.write_shape(data_output, &**s)?;
            } else {
                let written = self.write_shape_by_type_if_supported_typed(data_output, &**s, type_)?;
                if !written {
                    return Err(LuceneError::illegal_argument(format!(
                        "Unsupported shape type class {}",
                        s.java_class_name()
                    )));
                }
            }
        }
        Ok(())
    }
}

/// The plain `org.locationtech.spatial4j.io.BinaryCodec` class: every
/// [`BinaryCodec`] method at its Java body.
#[derive(Debug)]
pub struct BinaryCodecBase {
    /// `ctx`: the context owns its codec, so the codec refers back weakly.
    ctx: Weak<SpatialContext>,
}

impl BinaryCodecBase {
    /// `TYPE_POINT`.
    pub const TYPE_POINT: i8 = 1;
    /// `TYPE_RECT`.
    pub const TYPE_RECT: i8 = 2;
    /// `TYPE_CIRCLE`.
    pub const TYPE_CIRCLE: i8 = 3;
    /// `TYPE_COLL`.
    pub const TYPE_COLL: i8 = 4;
    /// `TYPE_GEOM`.
    pub const TYPE_GEOM: i8 = 5;

    /// `BinaryCodec(SpatialContext ctx, SpatialContextFactory factory)`.
    pub fn new(ctx: &Arc<SpatialContext>, _factory: &dyn SpatialContextFactoryOps) -> Self {
        Self { ctx: Arc::downgrade(ctx) }
    }

    /// The context, for a subclass.
    pub fn weak_ctx(&self) -> &Weak<SpatialContext> {
        &self.ctx
    }
}

impl BinaryCodec for BinaryCodecBase {
    fn ctx(&self) -> Arc<SpatialContext> {
        self.ctx.upgrade().expect("the SpatialContext of a BinaryCodec is alive")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
