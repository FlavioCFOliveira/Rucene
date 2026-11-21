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

import org.apache.lucene.codecs.NormsProducer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.SegmentReadState;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.util.Version;

/**
 * Reads the norms of a segment that Rucene wrote, using the real Apache Lucene
 * Core 10.5.0 norms reader.
 *
 * <p>Rucene's indexing chain writes only the files of the consumers it drives,
 * so a {@link org.apache.lucene.index.DirectoryReader} cannot open such a
 * directory. This tool rebuilds the little metadata the norms reader needs —
 * segment name, segment id, {@code maxDoc} and the field name/number/index-option
 * mapping — from the command line, and prints the same {@code norm=} lines as
 * {@link NormsFixture#dump}, so the two directions of the comparison are
 * literally the same strings.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... NormsReaderFixture &lt;dir&gt; &lt;segment&gt; &lt;segment-id-hex&gt; &lt;maxDoc&gt; &lt;name:number:options,...&gt;
 * </pre>
 *
 * <p>{@code options} is the name of an {@link IndexOptions} constant. Only
 * fields that actually have norms may be listed: the Lucene reader refuses a
 * metadata entry whose field says {@code omitNorms}.
 */
public final class NormsReaderFixture {

  private NormsReaderFixture() {}

  public static void main(String[] args) {
    if (args.length != 5) {
      System.err.println(
          "Usage: NormsReaderFixture <dir> <segment> <segment-id-hex> <maxDoc> <name:number:options,...>");
      System.exit(1);
    }

    Path dir = Paths.get(args[0]);
    String segment = args[1];
    byte[] segmentId = StoredFieldsReaderFixture.unhex(args[2]);
    int maxDoc = Integer.parseInt(args[3]);
    String fieldSpec = args[4];

    try (FSDirectory directory = FSDirectory.open(dir)) {
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

      try (NormsProducer producer = codec.normsFormat().normsProducer(state)) {
        producer.checkIntegrity();
        for (FieldInfo fi : fieldInfos) {
          NumericDocValues norms = producer.getNorms(fi);
          for (int doc = norms.nextDoc();
              doc != DocIdSetIterator.NO_MORE_DOCS;
              doc = norms.nextDoc()) {
            System.out.println("norm=" + doc + " " + fi.name + " " + norms.longValue());
          }
        }
      }
      System.out.println("read_ok=true");
    } catch (Exception e) {
      e.printStackTrace();
      System.exit(2);
    }
  }

  /**
   * Builds the minimal {@link FieldInfos} the norms reader needs. Every field
   * listed has {@code omitNorms = false}, because a field that omits norms has
   * no metadata entry and the reader rejects one that appears anyway.
   */
  static FieldInfos parseFieldInfos(String spec) {
    List<FieldInfo> infos = new ArrayList<>();
    if (!spec.isEmpty() && !spec.equals("-")) {
      for (String entry : spec.split(",")) {
        String[] parts = entry.split(":");
        if (parts.length != 3) {
          throw new IllegalArgumentException("bad field spec: " + entry);
        }
        infos.add(
            new FieldInfo(
                parts[0],
                Integer.parseInt(parts[1]),
                false,
                false,
                false,
                IndexOptions.valueOf(parts[2]),
                DocValuesType.NONE,
                DocValuesSkipIndexType.NONE,
                -1,
                Map.of(),
                0,
                0,
                0,
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
