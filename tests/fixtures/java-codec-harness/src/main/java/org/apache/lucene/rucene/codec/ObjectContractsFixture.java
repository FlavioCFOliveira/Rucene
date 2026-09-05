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

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;

import org.apache.lucene.analysis.CharArraySet;
import org.apache.lucene.analysis.tokenattributes.CharTermAttributeImpl;
import org.apache.lucene.codecs.CompetitiveImpactAccumulator;
import org.apache.lucene.codecs.DocValuesFormat;
import org.apache.lucene.codecs.Impact;
import org.apache.lucene.codecs.PostingsFormat;
import org.apache.lucene.codecs.compressing.CompressionMode;
import org.apache.lucene.codecs.hnsw.DefaultFlatVectorScorer;
import org.apache.lucene.codecs.hnsw.ScalarQuantizedVectorScorer;
import org.apache.lucene.codecs.lucene104.Lucene104HnswScalarQuantizedVectorsFormat;
import org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingStoredFieldsFormat;
import org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsFormat;
import org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsFormat;
import org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsFormat;
import org.apache.lucene.index.LogByteSizeMergePolicy;
import org.apache.lucene.index.LogDocMergePolicy;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.OrdTermState;
import org.apache.lucene.index.PrefixCodedTerms;
import org.apache.lucene.index.SoftDeletesRetentionMergePolicy;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.TieredMergePolicy;
import org.apache.lucene.index.UpgradeIndexMergePolicy;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.store.ByteBuffersDataInput;
import org.apache.lucene.store.ByteBuffersDataOutput;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.store.NRTCachingDirectory;
import org.apache.lucene.util.BitDocIdSet;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.DenseLiveDocs;
import org.apache.lucene.util.FixedBitSet;
import org.apache.lucene.util.IntsRef;
import org.apache.lucene.util.LongBitSet;
import org.apache.lucene.util.RoaringDocIdSet;
import org.apache.lucene.util.SparseFixedBitSet;
import org.apache.lucene.util.SparseLiveDocs;
import org.apache.lucene.util.StringHelper;
import org.apache.lucene.util.automaton.Automata;
import org.apache.lucene.util.automaton.ByteRunAutomaton;
import org.apache.lucene.util.bkd.BKDConfig;
import org.apache.lucene.util.bkd.HeapPointWriter;
import org.apache.lucene.util.bkd.OfflinePointWriter;
import org.apache.lucene.util.hnsw.NeighborArray;
import org.apache.lucene.util.hnsw.NeighborQueue;
import org.apache.lucene.util.packed.MonotonicBlockPackedReader;
import org.apache.lucene.util.packed.MonotonicBlockPackedWriter;
import org.apache.lucene.util.packed.PackedInts;
import org.apache.lucene.util.packed.PagedGrowableWriter;
import org.apache.lucene.util.packed.PagedMutable;

/**
 * Prints the exact strings and hash codes that Apache Lucene Core 10.5.0 produces
 * for the inherited {@code Object} contracts this port has to reproduce, so that
 * the Rust side asserts against measured values instead of against its own
 * formatting.
 *
 * <p>Output is one {@code key=value} line per measurement, terminated by
 * {@code emitted=true}. Values never contain a newline; the ones that could
 * (multi-line {@code toString()} bodies) are not measured here.
 *
 * <p>Run with {@code -Dtests.seed=<seed>} so that {@code StringHelper.GOOD_FAST_HASH_SEED},
 * and therefore {@code BytesRef.hashCode()}, is reproducible.
 */
public final class ObjectContractsFixture {

  private static final List<String> LINES = new ArrayList<>();

  private static void out(String key, String value) {
    if (value.indexOf('\n') >= 0) {
      throw new IllegalStateException("value for " + key + " contains a newline");
    }
    LINES.add(key + "=" + value);
  }

  public static void main(String[] args) throws Exception {
    utilBitSets();
    utilRefs();
    utilBkd();
    utilHnsw();
    utilPacked();
    utilAutomaton();
    store();
    codecs();
    index();
    analysis();

    org.apache.lucene.util.hnsw.RuceneHnswContracts.emit(ObjectContractsFixture::out);
    org.apache.lucene.util.automaton.RuceneAutomatonContracts.emit(ObjectContractsFixture::out);
    org.apache.lucene.codecs.RuceneCodecsContracts.emit(ObjectContractsFixture::out);
    org.apache.lucene.codecs.lucene90.RuceneLucene90Contracts.emit(ObjectContractsFixture::out);
    org.apache.lucene.index.RuceneIndexContracts.emit(ObjectContractsFixture::out);

    for (String line : LINES) {
      System.out.println(line);
    }
    System.out.println("emitted=true");
  }

  // ---------------------------------------------------------------------------

  private static void utilBitSets() {
    SparseFixedBitSet empty = new SparseFixedBitSet(100);
    out("SparseFixedBitSet.toString/empty100", empty.toString());

    SparseFixedBitSet sparse = new SparseFixedBitSet(10_000);
    sparse.set(1);
    sparse.set(5_000);
    sparse.set(9_999);
    out("SparseFixedBitSet.toString/three-of-10000", sparse.toString());

    RoaringDocIdSet.Builder builder = new RoaringDocIdSet.Builder(1000);
    builder.add(3);
    builder.add(17);
    builder.add(999);
    out("RoaringDocIdSet.toString/three-of-1000", builder.build().toString());

    RoaringDocIdSet.Builder none = new RoaringDocIdSet.Builder(64);
    out("RoaringDocIdSet.toString/empty64", none.build().toString());

    // BitDocIdSet over the only BitSet whose toString() Lucene overrides.
    SparseFixedBitSet forDocIdSet = new SparseFixedBitSet(128);
    forDocIdSet.set(7);
    out("BitDocIdSet.toString/sparse128", new BitDocIdSet(forDocIdSet, 1L).toString());

    // DenseLiveDocs: set bits are live.
    FixedBitSet live = new FixedBitSet(200);
    live.set(0, 200);
    live.clear(3);
    live.clear(9);
    live.clear(150);
    out(
        "DenseLiveDocs.toString/3-of-200",
        DenseLiveDocs.builder(live, 200).withDeletedCount(3).build().toString());

    FixedBitSet allLive = new FixedBitSet(8);
    allLive.set(0, 8);
    out(
        "DenseLiveDocs.toString/0-of-8",
        DenseLiveDocs.builder(allLive, 8).withDeletedCount(0).build().toString());

    // 1 deleted of 8 is 12.5%, the tie "%.2f" rounds away from zero.
    FixedBitSet oneDeleted = new FixedBitSet(8);
    oneDeleted.set(0, 8);
    oneDeleted.clear(4);
    out(
        "DenseLiveDocs.toString/1-of-8-tie",
        DenseLiveDocs.builder(oneDeleted, 8).withDeletedCount(1).build().toString());

    // SparseLiveDocs: set bits are deleted.
    SparseFixedBitSet deleted = new SparseFixedBitSet(1000);
    deleted.set(4);
    deleted.set(77);
    out(
        "SparseLiveDocs.toString/2-of-1000",
        SparseLiveDocs.builder(deleted, 1000).withDeletedCount(2).build().toString());

    SparseFixedBitSet oneOfEight = new SparseFixedBitSet(8);
    oneOfEight.set(4);
    out(
        "SparseLiveDocs.toString/1-of-8-tie",
        SparseLiveDocs.builder(oneOfEight, 8).withDeletedCount(1).build().toString());

    LongBitSet lbEmpty = new LongBitSet(64);
    out("LongBitSet.hashCode/empty64", Integer.toString(lbEmpty.hashCode()));

    LongBitSet lb = new LongBitSet(200);
    lb.set(0);
    lb.set(63);
    lb.set(64);
    lb.set(199);
    out("LongBitSet.hashCode/four-of-200", Integer.toString(lb.hashCode()));

    LongBitSet lbOne = new LongBitSet(1);
    lbOne.set(0);
    out("LongBitSet.hashCode/one-of-1", Integer.toString(lbOne.hashCode()));
  }

  private static void utilRefs() {
    out("IntsRef.toString/empty", new IntsRef().toString());
    int[] values = new int[] {0, 1, 15, 16, 255, -1, Integer.MIN_VALUE, Integer.MAX_VALUE};
    out("IntsRef.toString/eight", new IntsRef(values, 0, values.length).toString());
    out("IntsRef.toString/slice", new IntsRef(values, 2, 3).toString());

    out("StringHelper.GOOD_FAST_HASH_SEED", Integer.toString(StringHelper.GOOD_FAST_HASH_SEED));
    out("BytesRef.hashCode/empty", Integer.toString(new BytesRef(new byte[0]).hashCode()));
    out("BytesRef.hashCode/abc", Integer.toString(new BytesRef("abc").hashCode()));
    byte[] wide = new byte[] {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10};
    out("BytesRef.hashCode/eleven", Integer.toString(new BytesRef(wide, 0, wide.length).hashCode()));
    out("BytesRef.hashCode/slice", Integer.toString(new BytesRef(wide, 3, 5).hashCode()));
  }

  private static void utilBkd() throws Exception {
    BKDConfig config = BKDConfig.of(1, 1, 4, 512);
    HeapPointWriter heap = new HeapPointWriter(config, 10);
    out("HeapPointWriter.toString/empty10", heap.toString());

    try (Directory dir = new ByteBuffersDirectory()) {
      OfflinePointWriter offline = new OfflinePointWriter(config, dir, "prefix", "desc", 0);
      out("OfflinePointWriter.toString/fresh", offline.toString());
      offline.close();
    }
  }

  private static void utilHnsw() {
    out("NeighborArray.toString/empty", new NeighborArray(10, true).toString());
    NeighborArray array = new NeighborArray(10, true);
    array.addInOrder(1, 0.9f);
    array.addInOrder(2, 0.8f);
    out("NeighborArray.toString/two", array.toString());

    out("NeighborQueue.toString/empty", new NeighborQueue(4, true).toString());
    NeighborQueue queue = new NeighborQueue(4, true);
    queue.add(1, 0.5f);
    queue.add(2, 0.6f);
    queue.add(3, 0.7f);
    out("NeighborQueue.toString/three", queue.toString());
  }

  private static void utilPacked() throws Exception {
    out(
        "PagedMutable.toString/1000-pageSize1024",
        new PagedMutable(1000, 1024, 8, PackedInts.COMPACT).toString());
    out(
        "PagedGrowableWriter.toString/1000-pageSize1024",
        new PagedGrowableWriter(1000, 1024, 8, PackedInts.COMPACT).toString());

    try (Directory dir = new ByteBuffersDirectory()) {
      int blockSize = 64;
      long valueCount = 200;
      try (IndexOutput output = dir.createOutput("mono", IOContext.DEFAULT)) {
        MonotonicBlockPackedWriter writer = new MonotonicBlockPackedWriter(output, blockSize);
        for (long i = 0; i < valueCount; i++) {
          writer.add(i * 3);
        }
        writer.finish();
      }
      try (IndexInput input = dir.openInput("mono", IOContext.DEFAULT)) {
        MonotonicBlockPackedReader reader =
            MonotonicBlockPackedReader.of(input, PackedInts.VERSION_CURRENT, blockSize, valueCount);
        out("MonotonicBlockPackedReader.toString/200-block64", reader.toString());
      }
    }
  }

  private static void utilAutomaton() {
    ByteRunAutomaton empty = new ByteRunAutomaton(Automata.makeEmpty());
    out("RunAutomaton.hashCode/empty", Integer.toString(empty.hashCode()));

    ByteRunAutomaton abc = new ByteRunAutomaton(Automata.makeString("abc"));
    out("RunAutomaton.hashCode/abc", Integer.toString(abc.hashCode()));

    ByteRunAutomaton any = new ByteRunAutomaton(Automata.makeAnyString());
    out("RunAutomaton.hashCode/anyString", Integer.toString(any.hashCode()));
  }

  private static void store() throws Exception {
    ByteBuffersDataOutput fresh = new ByteBuffersDataOutput();
    out("ByteBuffersDataOutput.toString/fresh", fresh.toString());

    ByteBuffersDataOutput written = new ByteBuffersDataOutput();
    written.writeBytes(new byte[1500], 1500);
    out("ByteBuffersDataOutput.toString/1500bytes", written.toString());

    ByteBuffersDataOutput big = new ByteBuffersDataOutput();
    big.writeBytes(new byte[1_234_567], 1_234_567);
    out("ByteBuffersDataOutput.toString/1234567bytes", big.toString());

    ByteBuffersDataInput single =
        new ByteBuffersDataInput(Collections.singletonList(ByteBuffer.allocate(1500)));
    out("ByteBuffersDataInput.toString/single-1500", single.toString());

    List<ByteBuffer> blocks =
        Arrays.asList(ByteBuffer.allocate(1024), ByteBuffer.allocate(1024), ByteBuffer.allocate(7));
    ByteBuffersDataInput multi = new ByteBuffersDataInput(blocks);
    out("ByteBuffersDataInput.toString/three-1024", multi.toString());
    multi.seek(1030);
    out("ByteBuffersDataInput.toString/three-1024-pos1030", multi.toString());

    // NRTCachingDirectory: only the megabyte figures are portable; the wrapped
    // directory renders with an identity hash.
    try (Directory delegate = new ByteBuffersDirectory();
        NRTCachingDirectory nrt = new NRTCachingDirectory(delegate, 5.0, 60.0)) {
      String s = nrt.toString();
      out("NRTCachingDirectory.toString/5-60/suffix", s.substring(s.indexOf("; maxCacheMB=")));
    }
    try (Directory delegate = new ByteBuffersDirectory();
        NRTCachingDirectory nrt = new NRTCachingDirectory(delegate, 0.5, 48.0)) {
      String s = nrt.toString();
      out("NRTCachingDirectory.toString/0.5-48/suffix", s.substring(s.indexOf("; maxCacheMB=")));
    }
  }

  private static void codecs() {
    out("Impact.toString/1-1", new Impact(1, 1L).toString());
    out("Impact.toString/big", new Impact(2_000_000_000, Long.MIN_VALUE).toString());

    CompetitiveImpactAccumulator emptyAcc = new CompetitiveImpactAccumulator();
    out("CompetitiveImpactAccumulator.toString/empty", emptyAcc.toString());
    CompetitiveImpactAccumulator acc = new CompetitiveImpactAccumulator();
    acc.add(3, 5L);
    acc.add(10, 40L);
    acc.add(7, 12L);
    out("CompetitiveImpactAccumulator.toString/three", acc.toString());

    out("DocValuesFormat.toString/Lucene90", DocValuesFormat.forName("Lucene90").toString());
    out("PostingsFormat.toString/Lucene104", PostingsFormat.forName("Lucene104").toString());

    DefaultFlatVectorScorer defaultScorer = new DefaultFlatVectorScorer();
    out("DefaultFlatVectorScorer.toString", defaultScorer.toString());
    out(
        "ScalarQuantizedVectorScorer.toString/default-delegate",
        new ScalarQuantizedVectorScorer(defaultScorer).toString());
    out(
        "Lucene99FlatVectorsFormat.toString/default-scorer",
        new Lucene99FlatVectorsFormat(defaultScorer).toString());

    out("Lucene99HnswVectorsFormat.toString/default", new Lucene99HnswVectorsFormat().toString());
    out(
        "Lucene99HnswVectorsFormat.toString/32-200",
        new Lucene99HnswVectorsFormat(32, 200).toString());
    out(
        "Lucene104HnswScalarQuantizedVectorsFormat.toString/default",
        new Lucene104HnswScalarQuantizedVectorsFormat().toString());

    out(
        "Lucene90CompressingStoredFieldsFormat.toString/best-speed",
        new Lucene90CompressingStoredFieldsFormat(
                "Lucene90StoredFieldsFastData", CompressionMode.FAST, 1 << 14, 128, 10)
            .toString());
    out(
        "Lucene90CompressingTermVectorsFormat.toString/fast",
        new Lucene90CompressingTermVectorsFormat(
                "Lucene90TermVectorsData", "", CompressionMode.FAST, 1 << 12, 128, 10)
            .toString());
  }

  private static void index() {
    OrdTermState ord = new OrdTermState();
    out("OrdTermState.toString/default", ord.toString());
    ord.ord = 987654321L;
    out("OrdTermState.toString/987654321", ord.toString());

    out("NoMergePolicy.toString", NoMergePolicy.INSTANCE.toString());
    out("TieredMergePolicy.toString/default", new TieredMergePolicy().toString());
    TieredMergePolicy tuned = new TieredMergePolicy();
    tuned.setMaxMergeAtOnce(4);
    tuned.setMaxMergedSegmentMB(512.0);
    tuned.setFloorSegmentMB(4.0);
    tuned.setSegmentsPerTier(6.0);
    tuned.setDeletesPctAllowed(25.0);
    tuned.setForceMergeDeletesPctAllowed(5.0);
    tuned.setNoCFSRatio(0.5);
    tuned.setMaxCFSSegmentSizeMB(64.0);
    tuned.setTargetSearchConcurrency(3);
    out("TieredMergePolicy.toString/tuned", tuned.toString());

    out("LogByteSizeMergePolicy.toString/default", new LogByteSizeMergePolicy().toString());
    out("LogDocMergePolicy.toString/default", new LogDocMergePolicy().toString());

    out(
        "FilterMergePolicy.toString/UpgradeIndexMergePolicy-over-NoMergePolicy",
        new UpgradeIndexMergePolicy(NoMergePolicy.INSTANCE).toString());
    out(
        "FilterMergePolicy.toString/SoftDeletesRetentionMergePolicy",
        new SoftDeletesRetentionMergePolicy(
                "__soft_deletes", MatchAllDocsQuery::new, NoMergePolicy.INSTANCE)
            .toString());

    PrefixCodedTerms.Builder a = new PrefixCodedTerms.Builder();
    a.add(new Term("f", "aaa"));
    a.add(new Term("f", "bbb"));
    PrefixCodedTerms first = a.finish();
    PrefixCodedTerms.Builder b = new PrefixCodedTerms.Builder();
    b.add(new Term("f", "aaa"));
    b.add(new Term("f", "bbb"));
    PrefixCodedTerms second = b.finish();
    PrefixCodedTerms.Builder c = new PrefixCodedTerms.Builder();
    c.add(new Term("f", "aaa"));
    PrefixCodedTerms third = c.finish();

    out("PrefixCodedTerms.equals/same-terms", Boolean.toString(first.equals(second)));
    out("PrefixCodedTerms.equals/different-terms", Boolean.toString(first.equals(third)));
    out(
        "PrefixCodedTerms.hashCode/consistent-with-equals",
        Boolean.toString(first.hashCode() == second.hashCode()));
    second.setDelGen(9);
    out("PrefixCodedTerms.equals/different-delGen", Boolean.toString(first.equals(second)));
  }

  private static void analysis() {
    CharArraySet set = new CharArraySet(Arrays.asList("alpha", "beta", "gamma"), false);
    // The iteration order is the hash-table slot order; sort the rendered
    // elements so the assertion does not depend on it.
    out("CharArraySet.toString/three-sorted", sortedElements(set.toString()));
    out("CharArraySet.toString/empty", new CharArraySet(Collections.emptyList(), false).toString());

    CharTermAttributeImpl term = new CharTermAttributeImpl();
    out("CharTermAttributeImpl.toString/empty", term.toString());
    term.append("hello");
    out("CharTermAttributeImpl.toString/hello", term.toString());
    term.setEmpty().append("café 日本");
    out("CharTermAttributeImpl.toString/unicode", term.toString());
  }

  private static String sortedElements(String rendered) {
    String inner = rendered.substring(1, rendered.length() - 1);
    if (inner.isEmpty()) {
      return "[]";
    }
    String[] parts = inner.split(", ");
    Arrays.sort(parts);
    return "[" + String.join(", ", parts) + "]";
  }
}
