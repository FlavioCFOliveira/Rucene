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

import java.io.IOException;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

import org.apache.lucene.codecs.DocValuesProducer;
import org.apache.lucene.codecs.perfield.PerFieldDocValuesFormat;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.index.BinaryDocValues;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.SegmentReadState;
import org.apache.lucene.index.SortedDocValues;
import org.apache.lucene.index.SortedNumericDocValues;
import org.apache.lucene.index.SortedSetDocValues;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.Version;

import static org.apache.lucene.codecs.perfield.PerFieldDocValuesFormat.PER_FIELD_FORMAT_KEY;
import static org.apache.lucene.codecs.perfield.PerFieldDocValuesFormat.PER_FIELD_SUFFIX_KEY;

/**
 * Reads the doc values of a segment that Rucene wrote, using the real Apache
 * Lucene Core 10.5.0 doc-values reader.
 *
 * <p>Rucene's indexing chain writes only the files of the consumers it drives,
 * so a {@link org.apache.lucene.index.DirectoryReader} cannot open such a
 * directory. This tool rebuilds the little metadata the doc-values reader
 * needs — segment name, segment id, {@code maxDoc} and the field
 * name/number/doc-values-type mapping plus the per-field format attributes —
 * from the command line, and prints the same lines as {@link DocValuesFixture}
 * dumps, so the two directions of the comparison are literally the same
 * strings.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... DocValuesReaderFixture &lt;dir&gt; &lt;segment&gt; &lt;segment-id-hex&gt; &lt;maxDoc&gt;
 *       &lt;name:number:dvType:format:suffix,...&gt;
 * </pre>
 *
 * <p>{@code dvType} is the name of a {@link org.apache.lucene.index.DocValuesType}
 * constant, and {@code format}/{@code suffix} are the
 * {@code PerFieldDocValuesFormat.format}/{@code .suffix} attributes the
 * writing codec stored in the field infos (the doc-values reader resolves its
 * concrete format through them, so a field without them cannot be opened).
 * Only fields that actually carry doc values may be listed: the reader refuses
 * a metadata entry whose field says {@code NONE}.
 */
public final class DocValuesReaderFixture {

  private DocValuesReaderFixture() {}

  public static void main(String[] args) {
    if (args.length != 5) {
      System.err.println(
          "Usage: DocValuesReaderFixture <dir> <segment> <segment-id-hex> <maxDoc> "
              + "<name:number:dvType:format:suffix,...>");
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

      try (DocValuesProducer producer = codec.docValuesFormat().fieldsProducer(state)) {
        producer.checkIntegrity();
        for (FieldInfo fi : fieldInfos) {
          for (String line : dumpField(fi, producer)) {
            System.out.println(line);
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
   * Builds the minimal {@link FieldInfos} the doc-values reader needs. Every
   * field listed carries doc values, because a field without them has no
   * metadata entry and the reader rejects one that appears anyway.
   */
  static FieldInfos parseFieldInfos(String spec) {
    List<FieldInfo> infos = new ArrayList<>();
    if (!spec.isEmpty() && !spec.equals("-")) {
      for (String entry : spec.split(",")) {
        String[] parts = entry.split(":");
        if (parts.length != 5) {
          throw new IllegalArgumentException("bad field spec: " + entry);
        }
        FieldInfo info =
            new FieldInfo(
                parts[0],
                Integer.parseInt(parts[1]),
                false,
                false,
                false,
                IndexOptions.NONE,
                DocValuesType.valueOf(parts[2]),
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
                false);
        info.putAttribute(PER_FIELD_FORMAT_KEY, parts[3]);
        info.putAttribute(PER_FIELD_SUFFIX_KEY, parts[4]);
        infos.add(info);
      }
    }
    return new FieldInfos(infos.toArray(new FieldInfo[0]));
  }

  /**
   * Dumps the doc values of one segment as read by Lucene's own
   * {@link org.apache.lucene.index.LeafReader} accessors, so the two
   * directions of the comparison emit literally the same strings.
   */
  static List<String> dumpLeaf(org.apache.lucene.index.LeafReader reader) throws IOException {
    List<String> lines = new ArrayList<>();
    for (FieldInfo fi : reader.getFieldInfos()) {
      switch (fi.getDocValuesType()) {
        case NUMERIC -> dumpNumeric(fi, reader.getNumericDocValues(fi.name), lines);
        case BINARY -> dumpBinary(fi, reader.getBinaryDocValues(fi.name), lines);
        case SORTED -> dumpSorted(fi, reader.getSortedDocValues(fi.name), lines);
        case SORTED_NUMERIC -> dumpSortedNumeric(fi, reader.getSortedNumericDocValues(fi.name), lines);
        case SORTED_SET -> dumpSortedSet(fi, reader.getSortedSetDocValues(fi.name), lines);
        default -> {}
      }
    }
    return lines;
  }

  /** Dumps one field's values through a raw codec producer. */
  static List<String> dumpField(FieldInfo fi, DocValuesProducer producer) throws IOException {
    List<String> lines = new ArrayList<>();
    switch (fi.getDocValuesType()) {
      case NUMERIC -> dumpNumeric(fi, producer.getNumeric(fi), lines);
      case BINARY -> dumpBinary(fi, producer.getBinary(fi), lines);
      case SORTED -> dumpSorted(fi, producer.getSorted(fi), lines);
      case SORTED_NUMERIC -> dumpSortedNumeric(fi, producer.getSortedNumeric(fi), lines);
      case SORTED_SET -> dumpSortedSet(fi, producer.getSortedSet(fi), lines);
      default -> {}
    }
    return lines;
  }

  private static void dumpNumeric(FieldInfo fi, NumericDocValues dv, List<String> lines)
      throws IOException {
    for (int doc = dv.nextDoc(); doc != DocIdSetIterator.NO_MORE_DOCS; doc = dv.nextDoc()) {
      lines.add("dvnum=" + doc + " " + fi.name + " " + dv.longValue());
    }
  }

  private static void dumpBinary(FieldInfo fi, BinaryDocValues dv, List<String> lines)
      throws IOException {
    for (int doc = dv.nextDoc(); doc != DocIdSetIterator.NO_MORE_DOCS; doc = dv.nextDoc()) {
      lines.add("dvbin=" + doc + " " + fi.name + " " + hex(dv.binaryValue()));
    }
  }

  private static void dumpSorted(FieldInfo fi, SortedDocValues dv, List<String> lines)
      throws IOException {
    int count = dv.getValueCount();
    StringBuilder body = new StringBuilder();
    for (int ord = 0; ord < count; ord++) {
      if (ord > 0) body.append(',');
      body.append(hex(dv.lookupOrd(ord)));
    }
    lines.add("dvdict=" + fi.name + " " + count + ":" + body);
    for (int doc = dv.nextDoc(); doc != DocIdSetIterator.NO_MORE_DOCS; doc = dv.nextDoc()) {
      int ord = dv.ordValue();
      lines.add("dvsort=" + doc + " " + fi.name + " " + ord + " " + hex(dv.lookupOrd(ord)));
    }
  }

  private static void dumpSortedNumeric(
      FieldInfo fi, SortedNumericDocValues dv, List<String> lines) throws IOException {
    for (int doc = dv.nextDoc(); doc != DocIdSetIterator.NO_MORE_DOCS; doc = dv.nextDoc()) {
      int count = dv.docValueCount();
      StringBuilder body = new StringBuilder();
      for (int i = 0; i < count; i++) {
        if (i > 0) body.append(',');
        body.append(dv.nextValue());
      }
      lines.add("dvsortnum=" + doc + " " + fi.name + " " + count + ":" + body);
    }
  }

  private static void dumpSortedSet(FieldInfo fi, SortedSetDocValues dv, List<String> lines)
      throws IOException {
    long count = dv.getValueCount();
    StringBuilder body = new StringBuilder();
    for (long ord = 0; ord < count; ord++) {
      if (ord > 0) body.append(',');
      body.append(hex(dv.lookupOrd(ord)));
    }
    lines.add("dvdict=" + fi.name + " " + count + ":" + body);
    for (int doc = dv.nextDoc(); doc != DocIdSetIterator.NO_MORE_DOCS; doc = dv.nextDoc()) {
      StringBuilder ords = new StringBuilder();
      int n = dv.docValueCount();
      for (int i = 0; i < n; i++) {
        if (i > 0) ords.append(',');
        ords.append(dv.nextOrd());
      }
      lines.add("dvsortset=" + doc + " " + fi.name + " " + n + ":" + ords);
    }
  }

  private static String hex(BytesRef ref) {
    StringBuilder body = new StringBuilder(ref.length * 2);
    for (int i = 0; i < ref.length; i++) {
      body.append(String.format("%02x", ref.bytes[ref.offset + i]));
    }
    return body.toString();
  }
}