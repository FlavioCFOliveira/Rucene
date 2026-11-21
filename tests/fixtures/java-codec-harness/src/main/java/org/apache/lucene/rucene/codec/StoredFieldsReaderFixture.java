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

import org.apache.lucene.codecs.StoredFieldsReader;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.lucene90.Lucene90StoredFieldsFormat;
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
 * Reads the stored fields of a segment that Rucene wrote, using the real
 * Apache Lucene Core 10.5.0 stored-fields reader.
 *
 * <p>Rucene's indexing chain writes only the three stored-fields files of a
 * segment; the {@code segments_N}, {@code .si} and {@code .fnm} that a full
 * {@code IndexWriter} would also produce belong to components that are not part
 * of this port yet. A {@link org.apache.lucene.index.DirectoryReader} therefore
 * cannot open such a directory, so this tool rebuilds the little metadata the
 * stored-fields reader needs — segment name, segment id, {@code maxDoc}, the
 * compression mode and the field name/number mapping — from the command line,
 * exactly as the Rust side does when it reads a Lucene-written index.
 *
 * <p>It prints one line per document in the same format as
 * {@link StoredFieldsFixture.RecordingVisitor}, so the two directions of the
 * comparison are literally the same strings.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... StoredFieldsReaderFixture &lt;dir&gt; &lt;segment&gt; &lt;segment-id-hex&gt; &lt;maxDoc&gt; &lt;mode&gt; &lt;name:number,...&gt;
 * </pre>
 */
public final class StoredFieldsReaderFixture {

  private StoredFieldsReaderFixture() {}

  public static void main(String[] args) {
    if (args.length != 6) {
      System.err.println(
          "Usage: StoredFieldsReaderFixture <dir> <segment> <segment-id-hex> <maxDoc> <mode> <name:number,...>");
      System.exit(1);
    }

    Path dir = Paths.get(args[0]);
    String segment = args[1];
    byte[] segmentId = unhex(args[2]);
    int maxDoc = Integer.parseInt(args[3]);
    Lucene104Codec.Mode mode = Lucene104Codec.Mode.valueOf(args[4]);
    String fieldSpec = args[5];

    try (FSDirectory directory = FSDirectory.open(dir)) {
      Map<String, String> attributes = new HashMap<>();
      // `Lucene90StoredFieldsFormat.fieldsReader` refuses to open a segment
      // that does not declare which mode it was written in.
      attributes.put(Lucene90StoredFieldsFormat.MODE_KEY, mode.name());

      SegmentInfo info =
          new SegmentInfo(
              directory,
              Version.LATEST,
              Version.LATEST,
              segment,
              maxDoc,
              false,
              false,
              new Lucene104Codec(mode),
              Map.of(),
              segmentId,
              attributes,
              null);

      FieldInfos fieldInfos = parseFieldInfos(fieldSpec);

      try (StoredFieldsReader reader =
          new Lucene104Codec(mode)
              .storedFieldsFormat()
              .fieldsReader(directory, info, fieldInfos, IOContext.DEFAULT)) {
        reader.checkIntegrity();
        for (int docID = 0; docID < maxDoc; docID++) {
          StoredFieldsFixture.RecordingVisitor visitor = new StoredFieldsFixture.RecordingVisitor();
          reader.document(docID, visitor);
          System.out.println("doc " + docID + " " + String.join("|", visitor.seen));
        }
      }
      System.out.println("read_ok=true");
    } catch (Exception e) {
      e.printStackTrace();
      System.exit(2);
    }
  }

  /**
   * Builds the minimal {@link FieldInfos} the stored-fields reader needs: it
   * only ever looks a field up by number and reads its name.
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
                false,
                false,
                false,
                IndexOptions.NONE,
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

  static byte[] unhex(String hex) {
    byte[] out = new byte[hex.length() / 2];
    for (int i = 0; i < out.length; i++) {
      out[i] = (byte) Integer.parseInt(hex.substring(i * 2, i * 2 + 2), 16);
    }
    return out;
  }
}
