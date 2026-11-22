/*******************************************************************************
 * Copyright (c) 2015 VoyagerSearch and others
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! `GeoJSONReader.java`, of the Java package `org.locationtech.spatial4j.io`:
//! not applicable.
//!
//! `org.locationtech.spatial4j.io.GeoJSONReader` parses GeoJSON with the
//! optional `org.noggit` JSON parser, which is not on Apache Lucene 10.5.0's
//! classpath. `SpatialContextFactory.checkDefaultFormats` registers the
//! reader only through `addReaderIfNoggitExists`, which finds no
//! `org.noggit.JSONParser` there and adds nothing
//! (`SpatialContextFactory.java:250,263-268`), so Lucene never instantiates
//! the class. The port reproduces that: `org.noggit.JSONParser` is not a
//! class [`load_class`](crate::context::spatial_context_factory::load_class)
//! finds, `add_reader_if_noggit_exists` adds nothing, and the reader is not
//! ported (user decision on rmp task #621).
