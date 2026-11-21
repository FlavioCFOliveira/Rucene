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

import org.apache.lucene.codecs.TermVectorsReader;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.util.Version;

/**
 * Reads the term vectors of a segment that Rucene wrote, using the real Apache
 * Lucene Core 10.5.0 term-vectors reader.
 *
 * <p>Rucene's indexing chain writes only the files of the consumers it drives;
 * the {@code segments_N}, {@code .si} and {@code .fnm} that a full
 * {@code IndexWriter} would also produce belong to components that are not part
 * of this port yet, so a {@link org.apache.lucene.index.DirectoryReader} cannot
 * open such a directory. This tool rebuilds the little metadata the
 * term-vectors reader needs — segment name, segment id, {@code maxDoc} and the
 * field name/number mapping — from the command line, exactly as the Rust side
 * does when it reads a Lucene-written segment.
 *
 * <p>It prints the same lines as {@link TermVectorsFixture#dump}, so the two
 * directions of the comparison are literally the same strings.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... TermVectorsReaderFixture &lt;dir&gt; &lt;segment&gt; &lt;segment-id-hex&gt; &lt;maxDoc&gt; &lt;name:number,...&gt;
 * </pre>
 */
public final class TermVectorsReaderFixture {

  private TermVectorsReaderFixture() {}

  public static void main(String[] args) {
    if (args.length != 5) {
      System.err.println(
          "Usage: TermVectorsReaderFixture <dir> <segment> <segment-id-hex> <maxDoc> <name:number,...>");
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

      try (TermVectorsReader reader =
          codec.termVectorsFormat().vectorsReader(directory, info, fieldInfos, IOContext.DEFAULT)) {
        reader.checkIntegrity();
        for (String line : TermVectorsFixture.dump(reader, maxDoc)) {
          System.out.println(line);
        }
      }
      System.out.println("read_ok=true");
    } catch (Exception e) {
      e.printStackTrace();
      System.exit(2);
    }
  }

  /**
   * Builds the minimal {@link FieldInfos} the term-vectors reader needs: it
   * looks a field up by number to recover its name, and every flag it reports
   * comes from the term-vector chunk itself, never from here.
   */
  static FieldInfos parseFieldInfos(String spec) {
    List<FieldInfo> infos = new ArrayList<>();
    // "-" stands for "no fields at all", which an empty argument could not
    // survive on a command line.
    if (!spec.isEmpty() && !spec.equals("-")) {
      for (String entry : spec.split(",")) {
        int colon = entry.lastIndexOf(':');
        String name = entry.substring(0, colon);
        int number = Integer.parseInt(entry.substring(colon + 1));
        infos.add(
            new FieldInfo(
                name,
                number,
                true,
                true,
                false,
                IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
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
