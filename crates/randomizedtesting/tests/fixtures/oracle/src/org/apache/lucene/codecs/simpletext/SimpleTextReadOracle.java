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

package org.apache.lucene.codecs.simpletext;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.index.BinaryDocValues;
import org.apache.lucene.index.ByteVectorValues;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesSkipper;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.Fields;
import org.apache.lucene.index.FloatVectorValues;
import org.apache.lucene.index.Impacts;
import org.apache.lucene.index.ImpactsEnum;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.KnnVectorValues;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.PointValues;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.SegmentReader;
import org.apache.lucene.index.SortedDocValues;
import org.apache.lucene.index.SortedNumericDocValues;
import org.apache.lucene.index.SortedSetDocValues;
import org.apache.lucene.index.StoredFieldVisitor;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.search.AcceptDocs;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.TopKnnCollector;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.NIOFSDirectory;
import org.apache.lucene.util.Bits;
import org.apache.lucene.util.BytesRef;

/**
 * Prints what Lucene 10.5.0 reads from an index written with {@code SimpleTextCodec}: every leaf's
 * field infos, live docs, terms and postings (positions, offsets, payloads, an advance schedule and
 * the impacts), norms, stored fields, term vectors, doc values and their skippers, points and kNN
 * vectors with a nearest-neighbour search. {@code crates/rucene-codecs/tests/simpletext_oracle.rs}
 * prints the same lines with Rucene, and runs this file ({@code java -cp <jars>
 * SimpleTextReadOracle.java <dir>}) on the indexes Rucene writes.
 */
public final class SimpleTextReadOracle {

  /** The targets of the advance schedule. */
  static final int[] ADVANCE_TARGETS = {0, 1, 5, 9, 17, 33, 65, 129, 257};

  private SimpleTextReadOracle() {}

  public static void main(String[] args) throws IOException {
    for (String line : dump(Paths.get(args[0]))) {
      System.out.println(line);
    }
  }

  static String hex(byte[] b, int off, int len) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < len; i++) {
      sb.append(String.format("%02x", b[off + i] & 0xff));
    }
    return sb.toString();
  }

  static String hex(BytesRef b) {
    return b == null ? "-" : hex(b.bytes, b.offset, b.length);
  }

  /** The dump of the index at {@code path}, opened with {@code NIOFSDirectory}. */
  public static List<String> dump(Path path) throws IOException {
    List<String> out = new ArrayList<>();
    try (Directory dir = new NIOFSDirectory(path);
        DirectoryReader reader = DirectoryReader.open(dir)) {
      out.add("reader maxDoc=" + reader.maxDoc() + " numDocs=" + reader.numDocs());
      for (LeafReaderContext ctx : reader.leaves()) {
        dumpLeaf(ctx, out);
      }
      IndexSearcher searcher = new IndexSearcher(reader);
      int[][][] boxes = {
        {{10}, {60}}, {{3, 2}, {10, 10}}, {{2, -9, 1000}, {9, -2, 9000}}
      };
      String[] names = {"p_int", "p_2d", "e_int3"};
      for (int i = 0; i < names.length; i++) {
        if (FieldInfos.getMergedFieldInfos(reader).fieldInfo(names[i]) != null) {
          out.add(
              "count "
                  + names[i]
                  + "="
                  + searcher.count(IntPoint.newRangeQuery(names[i], boxes[i][0], boxes[i][1])));
        }
      }
    }
    return out;
  }

  static void dumpLeaf(LeafReaderContext ctx, List<String> out) throws IOException {
    dumpLeaf(ctx.ord, ctx.reader(), out);
  }

  /**
   * The dump of one leaf; a leaf that is not a {@code SegmentReader} (a wrapper, {@code
   * IndexWrappersOracle}) prints {@code codec=null}.
   */
  public static void dumpLeaf(int leafOrd, LeafReader leaf, List<String> out) throws IOException {
    String codec =
        leaf instanceof SegmentReader sr ? sr.getSegmentInfo().info.getCodec().getName() : "null";
    out.add(
        "leaf ord="
            + leafOrd
            + " maxDoc="
            + leaf.maxDoc()
            + " numDocs="
            + leaf.numDocs()
            + " codec="
            + codec);
    Bits live = leaf.getLiveDocs();
    if (live == null) {
      out.add(" live all");
    } else {
      StringBuilder sb = new StringBuilder(" live ");
      for (int i = 0; i < live.length(); i++) {
        sb.append(live.get(i) ? '1' : '0');
      }
      out.add(sb.toString());
    }
    for (FieldInfo fi : leaf.getFieldInfos()) {
      out.add(
          " fi "
              + fi.name
              + " num="
              + fi.number
              + " io="
              + fi.getIndexOptions()
              + " tv="
              + fi.hasTermVectors()
              + " norms="
              + fi.hasNorms()
              + " pay="
              + fi.hasPayloads()
              + " dv="
              + fi.getDocValuesType()
              + " skip="
              + fi.docValuesSkipIndexType()
              + " gen="
              + fi.getDocValuesGen()
              + " pts="
              + fi.getPointDimensionCount()
              + ","
              + fi.getPointIndexDimensionCount()
              + ","
              + fi.getPointNumBytes()
              + " vec="
              + fi.getVectorDimension()
              + ","
              + fi.getVectorEncoding()
              + ","
              + fi.getVectorSimilarityFunction()
              + " soft="
              + fi.isSoftDeletesField()
              + " parent="
              + fi.isParentField()
              + " atts="
              + fi.attributes().size());
    }
    for (FieldInfo fi : leaf.getFieldInfos()) {
      if (fi.getIndexOptions() != IndexOptions.NONE) {
        dumpTerms(leaf, fi, out);
      }
    }
    for (FieldInfo fi : leaf.getFieldInfos()) {
      if (fi.hasNorms()) {
        NumericDocValues norms = leaf.getNormValues(fi.name);
        StringBuilder sb = new StringBuilder(" norms " + fi.name);
        if (norms == null) {
          sb.append(" null");
        } else {
          for (int d = norms.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = norms.nextDoc()) {
            sb.append(' ').append(d).append('=').append(norms.longValue());
          }
        }
        out.add(sb.toString());
      }
    }
    for (int doc = 0; doc < leaf.maxDoc(); doc++) {
      dumpStored(leaf, doc, out);
    }
    for (int doc = 0; doc < leaf.maxDoc(); doc++) {
      Fields fields = leaf.termVectors().get(doc);
      if (fields != null) {
        out.add(" tv doc=" + doc + " size=" + fields.size());
        for (String field : fields) {
          Terms terms = fields.terms(field);
          out.add(
              "  tvf "
                  + field
                  + " size="
                  + terms.size()
                  + " sumTTF="
                  + terms.getSumTotalTermFreq()
                  + " sumDF="
                  + terms.getSumDocFreq()
                  + " docCount="
                  + terms.getDocCount()
                  + " pos="
                  + terms.hasPositions()
                  + " off="
                  + terms.hasOffsets()
                  + " pay="
                  + terms.hasPayloads());
          TermsEnum te = terms.iterator();
          for (BytesRef t = te.next(); t != null; t = te.next()) {
            StringBuilder sb =
                new StringBuilder("   t " + hex(t) + " ttf=" + te.totalTermFreq() + " ");
            PostingsEnum pe = te.postings(null, PostingsEnum.ALL);
            pe.nextDoc();
            positions(pe, sb);
            out.add(sb.toString());
          }
        }
      }
    }
    for (FieldInfo fi : leaf.getFieldInfos()) {
      switch (fi.getDocValuesType()) {
        case NONE -> {}
        case NUMERIC -> {
          NumericDocValues v = leaf.getNumericDocValues(fi.name);
          StringBuilder sb = new StringBuilder(" dv " + fi.name);
          for (int d = v.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = v.nextDoc()) {
            sb.append(' ').append(d).append('=').append(v.longValue());
          }
          out.add(sb.toString());
        }
        case BINARY -> {
          BinaryDocValues v = leaf.getBinaryDocValues(fi.name);
          StringBuilder sb = new StringBuilder(" dv " + fi.name);
          for (int d = v.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = v.nextDoc()) {
            sb.append(' ').append(d).append('=').append(hex(v.binaryValue()));
          }
          out.add(sb.toString());
        }
        case SORTED -> {
          SortedDocValues v = leaf.getSortedDocValues(fi.name);
          StringBuilder sb = new StringBuilder(" dv " + fi.name + " count=" + v.getValueCount());
          for (int ord = 0; ord < v.getValueCount(); ord++) {
            sb.append(" o").append(ord).append('=').append(hex(v.lookupOrd(ord)));
          }
          for (int d = v.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = v.nextDoc()) {
            sb.append(' ').append(d).append('=').append(v.ordValue());
          }
          out.add(sb.toString());
        }
        case SORTED_NUMERIC -> {
          SortedNumericDocValues v = leaf.getSortedNumericDocValues(fi.name);
          StringBuilder sb = new StringBuilder(" dv " + fi.name);
          for (int d = v.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = v.nextDoc()) {
            sb.append(' ').append(d).append('=');
            for (int i = 0; i < v.docValueCount(); i++) {
              sb.append(i == 0 ? "" : ",").append(v.nextValue());
            }
          }
          out.add(sb.toString());
        }
        case SORTED_SET -> {
          SortedSetDocValues v = leaf.getSortedSetDocValues(fi.name);
          StringBuilder sb = new StringBuilder(" dv " + fi.name + " count=" + v.getValueCount());
          for (long ord = 0; ord < v.getValueCount(); ord++) {
            sb.append(" o").append(ord).append('=').append(hex(v.lookupOrd(ord)));
          }
          for (int d = v.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = v.nextDoc()) {
            sb.append(' ').append(d).append('=');
            for (int i = 0; i < v.docValueCount(); i++) {
              sb.append(i == 0 ? "" : ",").append(v.nextOrd());
            }
          }
          out.add(sb.toString());
        }
      }
      if (fi.docValuesSkipIndexType() != DocValuesSkipIndexType.NONE) {
        DocValuesSkipper s = leaf.getDocValuesSkipper(fi.name);
        StringBuilder sb =
            new StringBuilder(
                " skip "
                    + fi.name
                    + " min="
                    + s.minValue()
                    + " max="
                    + s.maxValue()
                    + " docCount="
                    + s.docCount()
                    + " maxValueCount="
                    + s.maxValueCount()
                    + " levels="
                    + s.numLevels());
        for (int target = 0; ; ) {
          s.advance(target);
          sb.append(" [")
              .append(s.minDocID(0))
              .append(',')
              .append(s.maxDocID(0))
              .append(',')
              .append(s.minValue(0))
              .append(',')
              .append(s.maxValue(0))
              .append(',')
              .append(s.docCount(0))
              .append(']');
          if (s.maxDocID(0) == DocIdSetIterator.NO_MORE_DOCS) {
            break;
          }
          target = s.maxDocID(0) + 1;
        }
        out.add(sb.toString());
      }
    }
    for (FieldInfo fi : leaf.getFieldInfos()) {
      if (fi.getPointDimensionCount() > 0) {
        PointValues pv = leaf.getPointValues(fi.name);
        if (pv == null) {
          out.add(" points " + fi.name + " null");
          continue;
        }
        out.add(
            " points "
                + fi.name
                + " size="
                + pv.size()
                + " docCount="
                + pv.getDocCount()
                + " dims="
                + pv.getNumDimensions()
                + ","
                + pv.getNumIndexDimensions()
                + ","
                + pv.getBytesPerDimension()
                + " min="
                + hex(new BytesRef(pv.getMinPackedValue()))
                + " max="
                + hex(new BytesRef(pv.getMaxPackedValue()))
                + " treeSize="
                + pv.getPointTree().size());
        StringBuilder sb = new StringBuilder("  pv");
        pv.getPointTree()
            .visitDocValues(
                new PointValues.IntersectVisitor() {
                  @Override
                  public void visit(int docID) {
                    throw new IllegalStateException();
                  }

                  @Override
                  public void visit(int docID, byte[] packedValue) {
                    sb.append(' ')
                        .append(docID)
                        .append(':')
                        .append(hex(packedValue, 0, packedValue.length));
                  }

                  @Override
                  public PointValues.Relation compare(byte[] min, byte[] max) {
                    return PointValues.Relation.CELL_CROSSES_QUERY;
                  }
                });
        out.add(sb.toString());
        StringBuilder ids = new StringBuilder("  pids");
        pv.getPointTree()
            .visitDocIDs(
                new PointValues.IntersectVisitor() {
                  @Override
                  public void visit(int docID) {
                    ids.append(' ').append(docID);
                  }

                  @Override
                  public void visit(int docID, byte[] packedValue) {
                    throw new IllegalStateException();
                  }

                  @Override
                  public PointValues.Relation compare(byte[] min, byte[] max) {
                    return PointValues.Relation.CELL_CROSSES_QUERY;
                  }
                });
        out.add(ids.toString());
        StringBuilder tree = new StringBuilder("  tree");
        PointValues.PointTree root = pv.getPointTree();
        walk(root, tree);
        out.add(tree.toString());
        out.add("  clone size=" + root.clone().size());
      }
    }
    for (FieldInfo fi : leaf.getFieldInfos()) {
      if (fi.getVectorDimension() > 0) {
        dumpVectors(leaf, fi, out);
      }
    }
  }

  /** Pre-order walk of the tree: each node's bounds and size, then its children. */
  static void walk(PointValues.PointTree t, StringBuilder sb) throws IOException {
    sb.append(" [")
        .append(hex(new BytesRef(t.getMinPackedValue())))
        .append(',')
        .append(hex(new BytesRef(t.getMaxPackedValue())))
        .append(',')
        .append(t.size());
    if (t.moveToChild()) {
      do {
        walk(t, sb);
      } while (t.moveToSibling());
      t.moveToParent();
    }
    sb.append(']');
  }

  static void positions(PostingsEnum pe, StringBuilder sb) throws IOException {
    int freq = pe.freq();
    sb.append("f=").append(freq);
    for (int i = 0; i < freq; i++) {
      int pos = pe.nextPosition();
      sb.append(" [")
          .append(pos)
          .append(',')
          .append(pe.startOffset())
          .append(',')
          .append(pe.endOffset())
          .append(',')
          .append(hex(pe.getPayload()))
          .append(']');
    }
  }

  static void dumpTerms(LeafReader leaf, FieldInfo fi, List<String> out) throws IOException {
    Terms terms = leaf.terms(fi.name);
    if (terms == null) {
      out.add(" terms " + fi.name + " null");
      return;
    }
    out.add(
        " terms "
            + fi.name
            + " size="
            + terms.size()
            + " sumTTF="
            + terms.getSumTotalTermFreq()
            + " sumDF="
            + terms.getSumDocFreq()
            + " docCount="
            + terms.getDocCount()
            + " freqs="
            + terms.hasFreqs()
            + " pos="
            + terms.hasPositions()
            + " off="
            + terms.hasOffsets()
            + " pay="
            + terms.hasPayloads());
    TermsEnum te = terms.iterator();
    List<BytesRef> all = new ArrayList<>();
    for (BytesRef t = te.next(); t != null; t = te.next()) {
      all.add(BytesRef.deepCopyOf(t));
      out.add("  t " + hex(t) + " df=" + te.docFreq() + " ttf=" + te.totalTermFreq());
      PostingsEnum pe = te.postings(null, PostingsEnum.ALL);
      for (int d = pe.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = pe.nextDoc()) {
        StringBuilder sb = new StringBuilder("   d=" + d + " ");
        positions(pe, sb);
        out.add(sb.toString());
      }
      PostingsEnum docs = te.postings(null, PostingsEnum.NONE);
      StringBuilder sb = new StringBuilder("   docs");
      for (int d = docs.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = docs.nextDoc()) {
        sb.append(' ').append(d).append(':').append(docs.freq());
      }
      out.add(sb.toString());
      if (te.docFreq() > 8) {
        PostingsEnum adv = te.postings(null, PostingsEnum.FREQS);
        StringBuilder a = new StringBuilder("   adv");
        int doc = -1;
        for (int target : ADVANCE_TARGETS) {
          if (target > doc) {
            doc = adv.advance(target);
            a.append(' ').append(target).append("->").append(doc);
            if (doc == DocIdSetIterator.NO_MORE_DOCS) {
              break;
            }
            a.append(':').append(adv.freq());
          }
        }
        out.add(a.toString());
        ImpactsEnum ie = te.impacts(PostingsEnum.FREQS);
        StringBuilder im = new StringBuilder("   imp");
        for (int target : ADVANCE_TARGETS) {
          ie.advanceShallow(target);
          Impacts impacts = ie.getImpacts();
          im.append(" @").append(target).append(" levels=").append(impacts.numLevels());
          for (int level = 0; level < impacts.numLevels(); level++) {
            im.append(" L").append(level).append(" upTo=").append(impacts.getDocIdUpTo(level));
            var buf = impacts.getImpacts(level);
            for (int i = 0; i < buf.size; i++) {
              im.append(' ').append(buf.freqs[i]).append('/').append(buf.norms[i]);
            }
          }
          if (ie.docID() < target) {
            ie.advance(target);
          }
          if (ie.docID() == DocIdSetIterator.NO_MORE_DOCS) {
            break;
          }
        }
        out.add(im.toString());
        // The Impacts getImpacts() returns is a view of the enum: one taken before the first
        // advanceShallow reports the state each later one leaves.
        ImpactsEnum held = te.impacts(PostingsEnum.FREQS);
        Impacts view = held.getImpacts();
        StringBuilder lv = new StringBuilder("   live");
        for (int target : ADVANCE_TARGETS) {
          held.advanceShallow(target);
          lv.append(" @").append(target).append(" levels=").append(view.numLevels());
          for (int level = 0; level < view.numLevels(); level++) {
            lv.append(" L").append(level).append(" upTo=").append(view.getDocIdUpTo(level));
            var buf = view.getImpacts(level);
            for (int i = 0; i < buf.size; i++) {
              lv.append(' ').append(buf.freqs[i]).append('/').append(buf.norms[i]);
            }
          }
          if (held.docID() < target) {
            held.advance(target);
          }
          if (held.docID() == DocIdSetIterator.NO_MORE_DOCS) {
            break;
          }
        }
        out.add(lv.toString());
      }
    }
    // seekCeil and seekExact probes: every term, a term past each term, and the empty term.
    TermsEnum probe = terms.iterator();
    StringBuilder sb = new StringBuilder("  seek");
    List<BytesRef> targets = new ArrayList<>();
    targets.add(new BytesRef(""));
    for (BytesRef t : all) {
      targets.add(t);
      BytesRef past = new BytesRef(new byte[t.length + 1]);
      System.arraycopy(t.bytes, t.offset, past.bytes, 0, t.length);
      targets.add(past);
    }
    targets.add(new BytesRef(new byte[] {(byte) 0xff, (byte) 0xff}));
    for (BytesRef target : targets) {
      TermsEnum.SeekStatus status = probe.seekCeil(target);
      sb.append(' ').append(hex(target)).append(':').append(status);
      if (status != TermsEnum.SeekStatus.END) {
        sb.append(':').append(hex(probe.term())).append(':').append(probe.docFreq());
      }
    }
    out.add(sb.toString());
    TermsEnum exact = terms.iterator();
    StringBuilder ex = new StringBuilder("  exact");
    for (BytesRef target : targets) {
      boolean found = exact.seekExact(target);
      ex.append(' ').append(found);
      if (found) {
        ex.append(':').append(exact.totalTermFreq());
      }
    }
    out.add(ex.toString());
  }

  static void dumpStored(LeafReader leaf, int doc, List<String> out) throws IOException {
    StringBuilder sb = new StringBuilder(" stored doc=" + doc);
    leaf.storedFields()
        .document(
            doc,
            new StoredFieldVisitor() {
              @Override
              public Status needsField(FieldInfo fieldInfo) {
                return Status.YES;
              }

              @Override
              public void stringField(FieldInfo fieldInfo, String value) {
                sb.append(' ')
                    .append(fieldInfo.name)
                    .append("=string:")
                    .append(hex(new BytesRef(value)));
              }

              @Override
              public void binaryField(FieldInfo fieldInfo, byte[] value) {
                sb.append(' ')
                    .append(fieldInfo.name)
                    .append("=binary:")
                    .append(hex(value, 0, value.length));
              }

              @Override
              public void intField(FieldInfo fieldInfo, int value) {
                sb.append(' ').append(fieldInfo.name).append("=int:").append(value);
              }

              @Override
              public void longField(FieldInfo fieldInfo, long value) {
                sb.append(' ').append(fieldInfo.name).append("=long:").append(value);
              }

              @Override
              public void floatField(FieldInfo fieldInfo, float value) {
                sb.append(' ')
                    .append(fieldInfo.name)
                    .append("=float:")
                    .append(Integer.toHexString(Float.floatToRawIntBits(value)));
              }

              @Override
              public void doubleField(FieldInfo fieldInfo, double value) {
                sb.append(' ')
                    .append(fieldInfo.name)
                    .append("=double:")
                    .append(Long.toHexString(Double.doubleToRawLongBits(value)));
              }
            });
    out.add(sb.toString());
  }

  static void dumpVectors(LeafReader leaf, FieldInfo fi, List<String> out) throws IOException {
    int dim = fi.getVectorDimension();
    TopKnnCollector collector = new TopKnnCollector(3, Integer.MAX_VALUE);
    AcceptDocs acceptDocs = AcceptDocs.fromLiveDocs(leaf.getLiveDocs(), leaf.maxDoc());
    StringBuilder sb = new StringBuilder(" vec " + fi.name);
    if (fi.getVectorEncoding() == VectorEncoding.FLOAT32) {
      FloatVectorValues values = leaf.getFloatVectorValues(fi.name);
      if (values == null) {
        out.add(" vec " + fi.name + " null");
        return;
      }
      sb.append(" size=").append(values.size()).append(" dim=").append(values.dimension());
      KnnVectorValues.DocIndexIterator it = values.iterator();
      for (int d = it.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = it.nextDoc()) {
        sb.append(' ').append(d).append(':');
        float[] v = values.vectorValue(it.index());
        for (int i = 0; i < v.length; i++) {
          sb.append(i == 0 ? "" : ",").append(Integer.toHexString(Float.floatToRawIntBits(v[i])));
        }
      }
      float[] target = new float[dim];
      for (int i = 0; i < dim; i++) {
        target[i] = (i + 1) / 4f;
      }
      leaf.searchNearestVectors(fi.name, target, collector, acceptDocs);
    } else {
      ByteVectorValues values = leaf.getByteVectorValues(fi.name);
      if (values == null) {
        out.add(" vec " + fi.name + " null");
        return;
      }
      sb.append(" size=").append(values.size()).append(" dim=").append(values.dimension());
      KnnVectorValues.DocIndexIterator it = values.iterator();
      for (int d = it.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = it.nextDoc()) {
        byte[] v = values.vectorValue(it.index());
        sb.append(' ').append(d).append(':').append(hex(v, 0, v.length));
      }
      byte[] target = new byte[dim];
      for (int i = 0; i < dim; i++) {
        target[i] = (byte) (i * 3 - 2);
      }
      leaf.searchNearestVectors(fi.name, target, collector, acceptDocs);
    }
    out.add(sb.toString());
    TopDocs top = collector.topDocs();
    StringBuilder knn = new StringBuilder("  knn visited=" + collector.visitedCount());
    for (ScoreDoc sd : top.scoreDocs) {
      knn.append(' ')
          .append(sd.doc)
          .append(':')
          .append(Integer.toHexString(Float.floatToRawIntBits(sd.score)));
    }
    out.add(knn.toString());
  }

  /** Unused here; keeps {@link Codec} resolvable when the file is launched alone. */
  static String codecName() {
    return Codec.forName("SimpleText").getName();
  }

  static void unchecked(IOException e) {
    throw new UncheckedIOException(e);
  }
}
