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
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Arrays;
import java.util.List;
import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.TokenFilter;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.standard.StandardTokenizer;
import org.apache.lucene.analysis.tokenattributes.PayloadAttribute;
import org.apache.lucene.codecs.lucene104.ForUtil;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FreqAndNormBuffer;
import org.apache.lucene.index.Impacts;
import org.apache.lucene.index.ImpactsEnum;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.internal.vectorization.PostingDecodingUtil;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.PhraseQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.store.ByteBuffersDataInput;
import org.apache.lucene.store.ByteBuffersIndexInput;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Apache Lucene Core 10.5.0 oracle for the postings contracts of rmp #238, #240 and #241.
 *
 * <p>Usage: {@code java -cp lucene-core-10.5.0.jar PostingsSkipFixture.java <index-dir>
 * [<impacts-index-dir>]}.
 *
 * <ul>
 *   <li>Writes one segment whose term {@code a} occurs in all {@value #DOCS} documents of two
 *       fields: {@code body}, indexed with positions, offsets and payloads, and {@code plain}, with
 *       positions only. Every term past 256 documents has level-0 skip data, and past 8192 also
 *       level-1 skip data, so {@code advance} reads the position skip data that rmp #238 is about.
 *       The text of document {@code i} is {@link #text(int)}; the payload of a token is its
 *       position, as one byte, when the position is odd, and absent otherwise.
 *   <li>Prints, per field, the postings of {@code a} as Lucene's reader returns them after the
 *       {@code advance} schedule of {@link #TARGETS}, and the hits of the phrase {@code "a b"}.
 *   <li>Prints the {@code PostingsEnum} flag constants (rmp #240) and what {@code
 *       ForUtil.decode} throws for widths outside its mask table (rmp #241).
 *   <li>With a second directory, writes {@value #IMPACT_DOCS} documents whose term {@code a} of
 *       field {@code imp} alternates level-0 blocks of many competitive (freq, norm) pairs with
 *       blocks of one pair, and level-1 blocks the same way ({@link #impText(int)}), and prints
 *       the impacts of every level Lucene's reader returns after each {@code advanceShallow} of
 *       {@link #IMPACT_TARGETS} (rmp #351: a level's impacts are only the bytes its last skip
 *       entry holds, {@code Lucene104PostingsReader.java:702, 751, 850, 1398}).
 * </ul>
 */
public final class PostingsSkipFixture {

  static final int DOCS = 10_000;

  /** Monotonic advance targets crossing level-0 and level-1 skip boundaries. */
  static final int[] TARGETS = {
    0, 1, 255, 256, 257, 300, 600, 1024, 1025, 4097, 8191, 8192, 8193, 8600, 9000, 9999
  };

  /** Three full level-1 blocks and a tail. */
  static final int IMPACT_DOCS = 3 * 8192 + 1000;

  /** Monotonic {@code advanceShallow} targets crossing level-0 and level-1 boundaries. */
  static final int[] IMPACT_TARGETS = {
    0, 100, 255, 256, 300, 511, 512, 513, 1000, 8191, 8192, 8193, 9000, 16383, 16384, 16385,
    16700, 17000, 24575, 24576, 25000, 25575
  };

  private PostingsSkipFixture() {}

  /**
   * The text of document {@code i} of the impacts index: in a rich block, {@code a} occurs {@code
   * 1 + i % 11} times beside {@code (i * 7) % 19} filler tokens, so the block has many competitive
   * pairs; in a poor block, the text is {@code a} alone. Level-0 blocks alternate rich and poor,
   * except in the second level-1 block, which is all poor.
   */
  static String impText(int i) {
    boolean rich = (i / 256) % 2 == 0 && (i / 8192) != 1;
    if (rich == false) {
      return "a";
    }
    StringBuilder sb = new StringBuilder();
    for (int k = 0; k < 1 + i % 11; k++) {
      sb.append("a ");
    }
    for (int k = 0; k < (i * 7) % 19; k++) {
      sb.append("x ");
    }
    return sb.toString().trim();
  }

  /** The impacts of every level, as {@code level:docIdUpTo:freq/norm,...}. */
  static String describe(Impacts impacts) {
    StringBuilder sb = new StringBuilder();
    for (int level = 0; level < impacts.numLevels(); level++) {
      FreqAndNormBuffer buffer = impacts.getImpacts(level);
      sb.append(level == 0 ? "" : " ").append(level).append(':');
      sb.append(impacts.getDocIdUpTo(level)).append(':');
      for (int k = 0; k < buffer.size; k++) {
        sb.append(k == 0 ? "" : ",").append(buffer.freqs[k]).append('/').append(buffer.norms[k]);
      }
    }
    return sb.toString();
  }

  /** Writes the impacts index and prints {@code impacts <target> <levels>} per target. */
  static void writeAndReadImpacts(Path dirPath) throws IOException {
    Files.createDirectories(dirPath);
    IndexWriterConfig config = new IndexWriterConfig(new PlainAnalyzer());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    config.setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH);
    config.setRAMBufferSizeMB(256.0);
    try (FSDirectory dir = FSDirectory.open(dirPath);
        IndexWriter writer = new IndexWriter(dir, config)) {
      for (int i = 0; i < IMPACT_DOCS; i++) {
        Document doc = new Document();
        doc.add(new TextField("imp", impText(i), Field.Store.NO));
        writer.addDocument(doc);
      }
      writer.commit();
    }
    try (FSDirectory dir = FSDirectory.open(dirPath);
        DirectoryReader reader = DirectoryReader.open(dir)) {
      System.out.println("impacts leaves=" + reader.leaves().size());
      TermsEnum terms = reader.leaves().get(0).reader().terms("imp").iterator();
      if (terms.seekExact(new BytesRef("a")) == false) {
        throw new AssertionError("no term a");
      }
      ImpactsEnum impacts = terms.impacts(PostingsEnum.FREQS);
      for (int target : IMPACT_TARGETS) {
        impacts.advanceShallow(target);
        System.out.println("impacts " + target + " " + describe(impacts.getImpacts()));
      }
    }
  }

  /** {@code StandardTokenizer} alone, no filter. */
  static final class PlainAnalyzer extends Analyzer {
    @Override
    protected TokenStreamComponents createComponents(String fieldName) {
      StandardTokenizer tokenizer = new StandardTokenizer();
      return new TokenStreamComponents(tokenizer, tokenizer);
    }
  }

  /** The text of document {@code i}. */
  static String text(int i) {
    StringBuilder sb = new StringBuilder();
    int repeats = 1 + i % 4;
    for (int k = 0; k < repeats; k++) {
      if (sb.length() > 0) {
        sb.append(' ');
      }
      sb.append("a ");
      sb.append(i % 5 == 0 && k == 0 ? "b" : "w" + ((i + k) % 7));
    }
    if (i % 2 == 0) {
      sb.append(" c");
    }
    return sb.toString();
  }

  /** Adds the token's position as a one-byte payload at odd positions. */
  static final class PositionPayloadFilter extends TokenFilter {
    private final PayloadAttribute payload = addAttribute(PayloadAttribute.class);
    private int position = -1;

    PositionPayloadFilter(TokenStream in) {
      super(in);
    }

    @Override
    public boolean incrementToken() throws IOException {
      if (input.incrementToken() == false) {
        return false;
      }
      position++;
      payload.setPayload(position % 2 == 1 ? new BytesRef(new byte[] {(byte) position}) : null);
      return true;
    }

    @Override
    public void reset() throws IOException {
      super.reset();
      position = -1;
    }
  }

  public static void main(String[] args) throws IOException {
    if (args.length > 1) {
      writeAndReadImpacts(Paths.get(args[1]));
    }
    Path dirPath = Paths.get(args[0]);
    Files.createDirectories(dirPath);

    Analyzer analyzer =
        new Analyzer() {
          @Override
          protected TokenStreamComponents createComponents(String fieldName) {
            StandardTokenizer tokenizer = new StandardTokenizer();
            TokenStream stream = tokenizer;
            if (fieldName.equals("body")) {
              stream = new PositionPayloadFilter(stream);
            }
            return new TokenStreamComponents(tokenizer, stream);
          }
        };
    FieldType bodyType = new FieldType(TextField.TYPE_NOT_STORED);
    bodyType.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    bodyType.freeze();

    IndexWriterConfig config = new IndexWriterConfig(analyzer);
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    config.setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH);
    config.setRAMBufferSizeMB(256.0);
    try (FSDirectory dir = FSDirectory.open(dirPath);
        IndexWriter writer = new IndexWriter(dir, config)) {
      for (int i = 0; i < DOCS; i++) {
        Document doc = new Document();
        doc.add(new Field("body", text(i), bodyType));
        doc.add(new TextField("plain", text(i), Field.Store.NO));
        writer.addDocument(doc);
      }
      writer.commit();
    }

    try (FSDirectory dir = FSDirectory.open(dirPath);
        DirectoryReader reader = DirectoryReader.open(dir)) {
      System.out.println("leaves=" + reader.leaves().size());
      LeafReader leaf = reader.leaves().get(0).reader();
      IndexSearcher searcher = new IndexSearcher(reader);
      for (String field : new String[] {"body", "plain"}) {
        for (int flags : new int[] {PostingsEnum.POSITIONS, PostingsEnum.ALL}) {
          PostingsEnum postings = leaf.postings(new Term(field, "a"), flags);
          for (int target : TARGETS) {
            int doc = postings.advance(target);
            System.out.println(
                "advance " + field + " " + flags + " " + target + " " + describe(postings, doc));
          }
        }
        // Docs and freqs only: the exhaustive path `moveToNextLevel0Block` optimises.
        PostingsEnum freqs = leaf.postings(new Term(field, "a"), PostingsEnum.FREQS);
        long count = 0;
        long docSum = 0;
        long freqSum = 0;
        for (int d = freqs.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = freqs.nextDoc()) {
          count++;
          docSum += d;
          freqSum += (long) freqs.freq() * (d + 1);
        }
        System.out.println("freqs " + field + " " + count + " " + docSum + " " + freqSum);
        // Interleaves nextDoc with advance, so a skip starts from the middle of a block.
        PostingsEnum postings = leaf.postings(new Term(field, "a"), PostingsEnum.ALL);
        int doc = postings.nextDoc();
        while (doc != DocIdSetIterator.NO_MORE_DOCS) {
          System.out.println("walk " + field + " " + describe(postings, doc));
          doc = postings.nextDoc();
          if (doc != DocIdSetIterator.NO_MORE_DOCS) {
            doc = postings.advance(doc + 397);
          }
        }

        PhraseQuery phrase = new PhraseQuery(field, "a", "b");
        TopDocs top = searcher.search(phrase, DOCS);
        int[] docs = new int[top.scoreDocs.length];
        for (int i = 0; i < docs.length; i++) {
          docs[i] = top.scoreDocs[i].doc;
        }
        Arrays.sort(docs);
        StringBuilder sb = new StringBuilder();
        for (int d : docs) {
          if (sb.length() > 0) {
            sb.append(',');
          }
          sb.append(d);
        }
        System.out.println("phrase " + field + " " + top.totalHits.value() + " " + sb);
      }
    }

    System.out.println(
        "flags NONE="
            + PostingsEnum.NONE
            + " FREQS="
            + PostingsEnum.FREQS
            + " POSITIONS="
            + PostingsEnum.POSITIONS
            + " OFFSETS="
            + PostingsEnum.OFFSETS
            + " PAYLOADS="
            + PostingsEnum.PAYLOADS
            + " ALL="
            + PostingsEnum.ALL);

    for (int bitsPerValue : new int[] {32, 33, 40, 127}) {
      ByteBuffersIndexInput input =
          new ByteBuffersIndexInput(
              new ByteBuffersDataInput(List.of(java.nio.ByteBuffer.allocate(4096))), "corrupt");
      PostingDecodingUtil pdu = new PostingDecodingUtil(input) {};
      try {
        // `ForUtil.decode` is package-private; the jar is on the class path, so its
        // members are reachable reflectively.
        Method decode =
            ForUtil.class.getDeclaredMethod(
                "decode", int.class, PostingDecodingUtil.class, int[].class);
        decode.setAccessible(true);
        decode.invoke(new ForUtil(), bitsPerValue, pdu, new int[ForUtil.BLOCK_SIZE]);
        System.out.println("forutil " + bitsPerValue + " ok");
      } catch (ReflectiveOperationException reflective) {
        Throwable e =
            reflective instanceof InvocationTargetException ? reflective.getCause() : reflective;
        System.out.println(
            "forutil "
                + bitsPerValue
                + " "
                + e.getClass().getName()
                + " "
                + e.getMessage()
                + " fp="
                + input.getFilePointer());
      }
    }
  }

  /** One posting: doc, freq, then every position as pos/start-end/payload. */
  static String describe(PostingsEnum postings, int doc) throws IOException {
    if (doc == DocIdSetIterator.NO_MORE_DOCS) {
      return "END";
    }
    StringBuilder sb = new StringBuilder();
    int freq = postings.freq();
    sb.append(doc).append(" freq=").append(freq);
    for (int i = 0; i < freq; i++) {
      int position = postings.nextPosition();
      BytesRef payload = postings.getPayload();
      sb.append(' ')
          .append(position)
          .append('/')
          .append(postings.startOffset())
          .append('-')
          .append(postings.endOffset())
          .append('/')
          .append(payload == null ? "-" : hex(payload));
    }
    return sb.toString();
  }

  static String hex(BytesRef bytes) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < bytes.length; i++) {
      sb.append(String.format(java.util.Locale.ROOT, "%02x", bytes.bytes[bytes.offset + i]));
    }
    return sb.toString();
  }
}
