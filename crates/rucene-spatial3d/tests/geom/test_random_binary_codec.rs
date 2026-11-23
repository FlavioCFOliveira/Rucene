/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `TestRandomBinaryCodec.java`.

use crate::spatial3d_support::prelude::*;

type G = RandomGeo3dShapeGenerator;

/// `TestRandomBinaryCodec`.
pub fn class_def() -> Arc<Class> {
    let repeat = |iterations| Repeat {
        iterations,
        use_constant_seed: false,
    };
    class(
        "org.apache.lucene.spatial3d.geom.TestRandomBinaryCodec",
        vec![
            test("testRandomPointCodec", test_random_point_codec)
                .annotate(Test::new())
                .annotate(repeat(10)),
            test(
                "testRandomPlanetObjectCodec",
                test_random_planet_object_codec,
            )
            .annotate(Test::new())
            .annotate(repeat(100)),
            test("testRandomShapeCodec", test_random_shape_codec)
                .annotate(Test::new())
                .annotate(repeat(100)),
        ],
    )
}

fn test_random_point_codec() -> Result<()> {
    let planet_model = G::random_planet_model();
    let shape = G::random_geo_point(&planet_model)?;
    let mut output_stream: Vec<u8> = Vec::new();
    serializable_object::write_object(&mut output_stream, &shape)?;
    let mut input_stream: &[u8] = &output_stream;
    let shape_copy = serializable_object::read_object_with_model(&planet_model, &mut input_stream)?;
    assert_equals_object_msg(&shape.to_string(), &shape, &*shape_copy);
    Ok(())
}

fn test_random_planet_object_codec() -> Result<()> {
    let planet_model = G::random_planet_model();
    let shape_type = G::random_shape_type();
    let shape = G::random_geo_shape(shape_type, &planet_model)?;
    let mut output_stream: Vec<u8> = Vec::new();
    serializable_object::write_planet_object(&mut output_stream, &*shape)?;
    let mut input_stream: &[u8] = &output_stream;
    let shape_copy = serializable_object::read_planet_object(&mut input_stream)?;
    assert_equals_object_msg(&shape.to_string(), &*shape, &*shape_copy);
    Ok(())
}

fn test_random_shape_codec() -> Result<()> {
    let planet_model = G::random_planet_model();
    let shape_type = G::random_shape_type();
    let shape = G::random_geo_shape(shape_type, &planet_model)?;
    let mut output_stream: Vec<u8> = Vec::new();
    serializable_object::write_object(&mut output_stream, &*shape)?;
    let mut input_stream: &[u8] = &output_stream;
    let shape_copy = serializable_object::read_object_with_model(&planet_model, &mut input_stream)?;
    assert_equals_object_msg(&shape.to_string(), &*shape, &*shape_copy);
    Ok(())
}
