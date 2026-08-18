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
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Base64;
import java.util.Collections;
import java.util.Iterator;
import java.util.List;
import java.util.Map;

import org.apache.lucene.codecs.FieldsConsumer;
import org.apache.lucene.codecs.NormsProducer;
import org.apache.lucene.codecs.lucene104.Lucene104PostingsFormat;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.Fields;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.SegmentWriteState;
import org.apache.lucene.index.TermState;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.util.AttributeSource;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.IOBooleanSupplier;
import org.apache.lucene.util.StringHelper;
import org.apache.lucene.util.Version;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.index.ImpactsEnum;

/**
 * Generates deterministic Lucene 10.4.0 postings files for representative term
 * shapes and prints the Base64-encoded bytes of the .doc, .pos, .pay and .psm
 * files.
 *
 * <p>This fixture drives the real Lucene104PostingsFormat through its
 * {@link FieldsConsumer} API, using a fixed all-zero segment ID so that the
 * produced files can be compared byte-for-byte with Rucene's Rust port.
 *
 * <p>Usage:
 * <pre>
 *   mvn -q -f tests/fixtures/java-codec-harness/pom.xml \
 *       exec:java -Dexec.mainClass=org.apache.lucene.rucene.codec.PostingsWriterFixture \
 *       -Dexec.args="/tmp/rucene-postings-fixtures CASE"
 * </pre>
 * where CASE is one of SINGLETON, MULTI_DOC, POSITIONS, BLOCK_256, LEVEL1_8193.
 */
public final class PostingsWriterFixture {

  private PostingsWriterFixture() {}

  /** Fixed segment id, matches the all-zero id used by Rucene unit tests. */
  private static final byte[] SEGMENT_ID = new byte[StringHelper.ID_LENGTH];

  public static void main(String[] args) throws IOException {
    if (args.length != 2) {
      System.err.println("Usage: PostingsWriterFixture <output-dir> <case>");
      System.err.println("Cases: SINGLETON, MULTI_DOC, POSITIONS, BLOCK_256, LEVEL1_8193");
      System.exit(1);
    }

    Path outDir = Paths.get(args[0]);
    Case case_ = Case.valueOf(args[1]);

    Files.createDirectories(outDir);
    try (Directory dir = FSDirectory.open(outDir)) {
      writeFixture(dir, "_0", case_);
    }

    String[] extensions = {"doc", "pos", "pay", "psm"};
    System.out.println("case=" + case_);
    for (String ext : extensions) {
      Path file = outDir.resolve("_0." + ext);
      byte[] bytes;
      if (Files.exists(file)) {
        bytes = Files.readAllBytes(file);
      } else {
        bytes = new byte[0];
      }
      System.out.println(ext + "=" + Base64.getEncoder().encodeToString(bytes));
    }
  }

  private static void writeFixture(Directory dir, String segmentName, Case case_) throws IOException {
    SegmentInfo segmentInfo = new SegmentInfo(
        dir,
        Version.LUCENE_10_5_0,
        Version.LUCENE_10_5_0,
        segmentName,
        case_.maxDoc,
        false,
        false,
        new Lucene104Codec(),
        Collections.emptyMap(),
        SEGMENT_ID.clone(),
        Collections.emptyMap(),
        null);

    FieldInfo fieldInfo = new FieldInfo(
        "body",
        0,
        false,
        true, // omitNorms
        case_.hasPayloads,
        case_.indexOptions,
        DocValuesType.NONE,
        DocValuesSkipIndexType.NONE,
        -1,
        Collections.emptyMap(),
        0,
        0,
        0,
        0,
        VectorEncoding.FLOAT32,
        VectorSimilarityFunction.EUCLIDEAN,
        false,
        false);

    FieldInfos fieldInfos = new FieldInfos(new FieldInfo[] {fieldInfo});
    SegmentWriteState state = new SegmentWriteState(
        null, dir, segmentInfo, fieldInfos, null, IOContext.DEFAULT);

    Lucene104PostingsFormat format = new Lucene104PostingsFormat();
    try (FieldsConsumer consumer = format.fieldsConsumer(state)) {
      consumer.write(new TestFields(case_.terms), new EmptyNormsProducer());
    }
  }

  private enum Case {
    SINGLETON(IndexOptions.DOCS_AND_FREQS, false, 2, List.of(
        term("singleton", List.of(new Posting(0, 1, List.of()))))),

    MULTI_DOC(IndexOptions.DOCS_AND_FREQS, false, 8, List.of(
        term("multidoc", List.of(
            new Posting(0, 3, List.of()),
            new Posting(2, 1, List.of()),
            new Posting(5, 2, List.of()),
            new Posting(7, 1, List.of()))))),

    POSITIONS(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS, true, 4, List.of(
        term("positions", List.of(
            new Posting(0, 2, List.of(
                new Position(0, payload("p0"), 0, 2),
                new Position(3, null, 5, 8))),
            new Posting(2, 1, List.of(
                new Position(1, payload("p2"), 0, 3))),
            new Posting(3, 3, List.of(
                new Position(0, null, 0, 4),
                new Position(4, payload("p3"), 5, 7),
                new Position(6, null, 8, 10))))))),

    BLOCK_256(IndexOptions.DOCS_AND_FREQS, false, 300, List.of(
        term("block256", blockPostings(256, 1)))),

    LEVEL1_8193(IndexOptions.DOCS_AND_FREQS, false, 9000, List.of(
        term("level1", blockPostings(8193, 1))));

    final IndexOptions indexOptions;
    final boolean hasPayloads;
    final int maxDoc;
    final List<TermEntry> terms;

    Case(IndexOptions indexOptions, boolean hasPayloads, int maxDoc, List<TermEntry> terms) {
      this.indexOptions = indexOptions;
      this.hasPayloads = hasPayloads;
      this.maxDoc = maxDoc;
      this.terms = terms;
    }
  }

  private static TermEntry term(String text, List<Posting> postings) {
    return new TermEntry(new BytesRef(text), postings);
  }

  private static List<Posting> blockPostings(int count, int freq) {
    List<Posting> postings = new java.util.ArrayList<>(count);
    for (int i = 0; i < count; i++) {
      postings.add(new Posting(i, freq, List.of()));
    }
    return postings;
  }

  private static BytesRef payload(String text) {
    return new BytesRef(text);
  }

  private record TermEntry(BytesRef term, List<Posting> postings) {}
  private record Posting(int docId, int freq, List<Position> positions) {}
  private record Position(int pos, BytesRef payload, int startOffset, int endOffset) {}

  private static final class TestFields extends Fields {
    private final List<TermEntry> terms;

    TestFields(List<TermEntry> terms) {
      this.terms = terms;
    }

    @Override
    public Iterator<String> iterator() {
      return Collections.singletonList("body").iterator();
    }

    @Override
    public Terms terms(String field) {
      if ("body".equals(field)) {
        return new TestTerms(terms);
      }
      return null;
    }

    @Override
    public int size() {
      return 1;
    }
  }

  private static final class TestTerms extends Terms {
    private final List<TermEntry> terms;

    TestTerms(List<TermEntry> terms) {
      this.terms = terms;
    }

    @Override
    public TermsEnum iterator() {
      return new TestTermsEnum(terms);
    }

    @Override
    public long size() {
      return terms.size();
    }

    @Override
    public long getSumTotalTermFreq() {
      long sum = 0;
      for (TermEntry t : terms) {
        for (Posting p : t.postings()) {
          sum += p.freq();
        }
      }
      return sum;
    }

    @Override
    public long getSumDocFreq() {
      long sum = 0;
      for (TermEntry t : terms) {
        sum += t.postings().size();
      }
      return sum;
    }

    @Override
    public int getDocCount() {
      return terms.isEmpty() ? 0 : terms.get(0).postings().get(terms.get(0).postings().size() - 1).docId() + 1;
    }

    @Override
    public boolean hasFreqs() {
      return true;
    }

    @Override
    public boolean hasPositions() {
      return true;
    }

    @Override
    public boolean hasOffsets() {
      return true;
    }

    @Override
    public boolean hasPayloads() {
      return true;
    }
  }

  private static final class TestTermsEnum extends TermsEnum {
    private final List<TermEntry> terms;
    private int idx = -1;
    private final AttributeSource atts = new AttributeSource();

    TestTermsEnum(List<TermEntry> terms) {
      this.terms = terms;
    }

    @Override
    public AttributeSource attributes() {
      return atts;
    }

    @Override
    public BytesRef term() {
      return terms.get(idx).term();
    }

    @Override
    public PostingsEnum postings(PostingsEnum reuse, int flags) {
      return new TestPostingsEnum(terms.get(idx).postings());
    }

    @Override
    public boolean seekExact(BytesRef text) {
      return false;
    }

    @Override
    public IOBooleanSupplier prepareSeekExact(BytesRef text) {
      return null;
    }

    @Override
    public SeekStatus seekCeil(BytesRef text) {
      return SeekStatus.END;
    }

    @Override
    public void seekExact(long ord) {}

    @Override
    public void seekExact(BytesRef term, TermState state) {}

    @Override
    public long ord() {
      return idx;
    }

    @Override
    public int docFreq() {
      return terms.get(idx).postings().size();
    }

    @Override
    public long totalTermFreq() {
      long sum = 0;
      for (Posting p : terms.get(idx).postings()) {
        sum += p.freq();
      }
      return sum;
    }

    @Override
    public ImpactsEnum impacts(int flags) {
      throw new UnsupportedOperationException();
    }

    @Override
    public TermState termState() {
      throw new UnsupportedOperationException();
    }

    @Override
    public BytesRef next() {
      idx++;
      if (idx >= terms.size()) {
        return null;
      }
      return terms.get(idx).term();
    }
  }

  private static final class TestPostingsEnum extends PostingsEnum {
    private final List<Posting> postings;
    private int docIdx = -1;
    private int posIdx = -1;

    TestPostingsEnum(List<Posting> postings) {
      this.postings = postings;
    }

    @Override
    public int docID() {
      if (docIdx < 0) {
        return -1;
      }
      if (docIdx >= postings.size()) {
        return NO_MORE_DOCS;
      }
      return postings.get(docIdx).docId();
    }

    @Override
    public int nextDoc() {
      docIdx++;
      posIdx = -1;
      if (docIdx >= postings.size()) {
        return NO_MORE_DOCS;
      }
      return postings.get(docIdx).docId();
    }

    @Override
    public int advance(int target) {
      return nextDoc();
    }

    @Override
    public long cost() {
      return postings.size();
    }

    @Override
    public int freq() {
      return postings.get(docIdx).freq();
    }

    @Override
    public int nextPosition() {
      posIdx++;
      return postings.get(docIdx).positions().get(posIdx).pos();
    }

    @Override
    public int startOffset() {
      return postings.get(docIdx).positions().get(posIdx).startOffset();
    }

    @Override
    public int endOffset() {
      return postings.get(docIdx).positions().get(posIdx).endOffset();
    }

    @Override
    public BytesRef getPayload() {
      return postings.get(docIdx).positions().get(posIdx).payload();
    }
  }

  private static final class EmptyNormsProducer extends NormsProducer {
    @Override
    public NumericDocValues getNorms(FieldInfo field) {
      return null;
    }

    @Override
    public void checkIntegrity() {}

    @Override
    public void close() {}
  }
}
