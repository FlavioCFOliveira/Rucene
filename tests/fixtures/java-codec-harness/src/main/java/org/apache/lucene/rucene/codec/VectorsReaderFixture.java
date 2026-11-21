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
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import org.apache.lucene.codecs.KnnVectorsReader;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.index.ByteVectorValues;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.FloatVectorValues;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.KnnVectorValues;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.SegmentReadState;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.util.Version;

/**
 * Reads the KNN vector values of a segment that Rucene wrote, using the real
 * Apache Lucene Core 10.5.0 vectors reader.
 *
 * <p>Rucene's indexing chain writes only the files of the consumers it drives,
 * so a {@link org.apache.lucene.index.DirectoryReader} cannot open such a
 * directory. This tool rebuilds the metadata the vectors reader needs — the
 * segment name, the segment id, {@code maxDoc} and, per field, the name,
 * number, dimension, encoding and similarity — from the command line, and
 * prints the same {@code vec=} lines {@link VectorsFixture} prints, so the two
 * directions of the comparison are literally the same strings.
 *
 * <p>Unlike points, vectors <em>are</em> written through a per-field format, so
 * every field must also carry the two
 * {@code PerFieldKnnVectorsFormat} attributes the reader looks up: the concrete
 * format name and the suffix its files were written with. Passing them from the
 * command line rather than defaulting them is deliberate — a wrong value must
 * fail loudly rather than quietly read the wrong file.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... VectorsReaderFixture &lt;dir&gt; &lt;segment&gt; &lt;segment-id-hex&gt; &lt;maxDoc&gt;
 *       &lt;name:number:dim:encoding:similarity:format:suffix,...&gt;
 * </pre>
 */
public final class VectorsReaderFixture {

  private VectorsReaderFixture() {}

  public static void main(String[] args) throws Exception {
    if (args.length != 5) {
      System.err.println(
          "Usage: VectorsReaderFixture <dir> <segment> <segment-id-hex> <maxDoc> "
              + "<name:number:dim:encoding:similarity:format:suffix,...>");
      System.exit(1);
    }

    Path dirPath = Paths.get(args[0]);
    String segment = args[1];
    byte[] segmentId = StoredFieldsReaderFixture.unhex(args[2]);
    int maxDoc = Integer.parseInt(args[3]);
    String fieldSpec = args[4];

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

      try (KnnVectorsReader reader = codec.knnVectorsFormat().fieldsReader(state)) {
        reader.checkIntegrity();
        for (String line : dump(reader, fieldInfos)) {
          System.out.println(line);
        }
      }
      System.out.println("read_ok=true");
    }
  }

  /** Dumps every vector of every field, in the format {@link VectorsFixture} uses. */
  static List<String> dump(KnnVectorsReader reader, FieldInfos fieldInfos) throws Exception {
    List<String> lines = new ArrayList<>();
    for (FieldInfo fi : fieldInfos) {
      if (fi.getVectorDimension() == 0) {
        continue;
      }
      if (fi.getVectorEncoding() == VectorEncoding.FLOAT32) {
        FloatVectorValues values = reader.getFloatVectorValues(fi.name);
        if (values == null) {
          continue;
        }
        KnnVectorValues.DocIndexIterator iter = values.iterator();
        for (int doc = iter.nextDoc();
            doc != DocIdSetIterator.NO_MORE_DOCS;
            doc = iter.nextDoc()) {
          float[] value = values.vectorValue(iter.index());
          StringBuilder sb = new StringBuilder();
          sb.append("vec=").append(fi.name).append(",doc=").append(doc).append(",ord=")
              .append(iter.index()).append(",value=");
          for (int i = 0; i < value.length; i++) {
            if (i > 0) {
              sb.append(':');
            }
            sb.append(Integer.toHexString(Float.floatToRawIntBits(value[i])));
          }
          lines.add(sb.toString());
        }
      } else {
        ByteVectorValues values = reader.getByteVectorValues(fi.name);
        if (values == null) {
          continue;
        }
        KnnVectorValues.DocIndexIterator iter = values.iterator();
        for (int doc = iter.nextDoc();
            doc != DocIdSetIterator.NO_MORE_DOCS;
            doc = iter.nextDoc()) {
          byte[] value = values.vectorValue(iter.index());
          StringBuilder sb = new StringBuilder();
          sb.append("vec=").append(fi.name).append(",doc=").append(doc).append(",ord=")
              .append(iter.index()).append(",value=");
          for (byte b : value) {
            sb.append(String.format("%02x", b));
          }
          lines.add(sb.toString());
        }
      }
    }
    return lines;
  }

  /** Builds the minimal {@link FieldInfos} the vectors reader needs. */
  static FieldInfos parseFieldInfos(String spec) {
    List<FieldInfo> infos = new ArrayList<>();
    if (!spec.isEmpty() && !spec.equals("-")) {
      for (String entry : spec.split(",")) {
        String[] parts = entry.split(":");
        if (parts.length != 7) {
          throw new IllegalArgumentException("bad field spec: " + entry);
        }
        Map<String, String> attributes = new HashMap<>();
        attributes.put("PerFieldKnnVectorsFormat.format", parts[5]);
        attributes.put("PerFieldKnnVectorsFormat.suffix", parts[6]);
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
                attributes,
                0,
                0,
                0,
                Integer.parseInt(parts[2]),
                VectorEncoding.valueOf(parts[3]),
                VectorSimilarityFunction.valueOf(parts[4]),
                false,
                false));
      }
    }
    return new FieldInfos(infos.toArray(new FieldInfo[0]));
  }
}
