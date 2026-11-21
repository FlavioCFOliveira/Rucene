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
package org.apache.lucene.rucene.codec;

import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

import org.apache.lucene.codecs.PointsReader;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.PointValues;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.SegmentReadState;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.util.Version;

/**
 * Reads the point values of a segment that Rucene wrote, using the real Apache
 * Lucene Core 10.5.0 points reader.
 *
 * <p>Rucene's indexing chain writes only the files of the consumers it drives,
 * so a {@link org.apache.lucene.index.DirectoryReader} cannot open such a
 * directory. This tool rebuilds the little metadata the points reader needs —
 * segment name, segment id, {@code maxDoc} and the field
 * name/number/dimension mapping — from the command line, and prints the same
 * lines {@link PointsFixture} prints, so the two directions of the comparison
 * are literally the same strings.
 *
 * <p>Unlike doc values, points are not written through a per-field format, so
 * no {@code PerFieldDocValuesFormat} attribute is needed: the codec's single
 * {@code Lucene90PointsFormat} reads every field of the segment.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... PointsReaderFixture &lt;dir&gt; &lt;segment&gt; &lt;segment-id-hex&gt; &lt;maxDoc&gt;
 *       &lt;case&gt; &lt;name:number:dims:indexDims:bytes,...&gt;
 * </pre>
 *
 * <p>Only fields that actually carry points may be listed: the reader's
 * metadata is keyed by field number and it refuses an entry whose field
 * declares no dimensions.
 */
public final class PointsReaderFixture {

  private PointsReaderFixture() {}

  public static void main(String[] args) throws Exception {
    if (args.length != 6) {
      System.err.println(
          "Usage: PointsReaderFixture <dir> <segment> <segment-id-hex> <maxDoc> <case> "
              + "<name:number:dims:indexDims:bytes,...>");
      System.exit(1);
    }

    Path dirPath = Paths.get(args[0]);
    String segment = args[1];
    byte[] segmentId = StoredFieldsReaderFixture.unhex(args[2]);
    int maxDoc = Integer.parseInt(args[3]);
    String shape = args[4];
    String fieldSpec = args[5];

    try (FSDirectory directory = FSDirectory.open(dirPath)) {
      Lucene104Codec codec = new Lucene104Codec();
      SegmentInfo info =
          new SegmentInfo(
              directory,
              Version.LATEST,
              Version.LATEST,
              segment,
              maxDoc,
              false,
              false,
              codec,
              Map.of(),
              segmentId,
              Map.of(),
              null);

      FieldInfos fieldInfos = parseFieldInfos(fieldSpec);
      SegmentReadState state =
          new SegmentReadState(directory, info, fieldInfos, IOContext.DEFAULT);

      byte[][] range = PointsFixture.range(shape);
      try (PointsReader reader = codec.pointsFormat().fieldsReader(state)) {
        reader.checkIntegrity();
        for (FieldInfo fi : fieldInfos) {
          if (fi.getPointDimensionCount() == 0) {
            continue;
          }
          PointValues values = reader.getValues(fi.name);
          PointsFixture.dump(fi.name, values);
          PointsFixture.dumpRange(fi.name, values, range[0], range[1]);
        }
      }
      System.out.println("read_ok=true");
    }
  }

  /** Builds the minimal {@link FieldInfos} the points reader needs. */
  static FieldInfos parseFieldInfos(String spec) {
    List<FieldInfo> infos = new ArrayList<>();
    if (!spec.isEmpty() && !spec.equals("-")) {
      for (String entry : spec.split(",")) {
        String[] parts = entry.split(":");
        if (parts.length != 5) {
          throw new IllegalArgumentException("bad field spec: " + entry);
        }
        infos.add(
            new FieldInfo(
                parts[0],
                Integer.parseInt(parts[1]),
                false,
                false,
                false,
                IndexOptions.NONE,
                DocValuesType.NONE,
                DocValuesSkipIndexType.NONE,
                -1,
                Map.of(),
                Integer.parseInt(parts[2]),
                Integer.parseInt(parts[3]),
                Integer.parseInt(parts[4]),
                0,
                VectorEncoding.FLOAT32,
                VectorSimilarityFunction.EUCLIDEAN,
                false,
                false));
      }
    }
    return new FieldInfos(infos.toArray(new FieldInfo[0]));
  }
}
