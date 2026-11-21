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
import java.util.Collections;

import org.apache.lucene.codecs.hnsw.HnswGraphProvider;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsFormat;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.SegmentReadState;
import org.apache.lucene.index.SegmentWriteState;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.codecs.KnnVectorsReader;
import org.apache.lucene.codecs.KnnVectorsWriter;
import org.apache.lucene.codecs.KnnFieldVectorsWriter;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.util.InfoStream;
import org.apache.lucene.util.Version;
import org.apache.lucene.util.hnsw.HnswGraph;

/**
 * Writes a single HNSW field with {@code Lucene99HnswVectorsFormat} driven
 * directly, then dumps the graph its reader hands back through
 * {@code HnswGraphProvider.getGraph(String)}: the level count, the entry node,
 * the size, and every node's neighbour list on every level.
 *
 * <p>That dump is what a port has to reproduce, because it is the whole content
 * of the {@code .vex} file as the reader sees it — not merely the bytes, but the
 * decoded structure, which is what a search walks.
 *
 * <p>Usage: {@code java ... HnswGraphProviderFixture <dir> <numDocs> <dim>}.
 */
public final class HnswGraphProviderFixture {

  private HnswGraphProviderFixture() {}

  static final byte[] SEGMENT_ID = new byte[] {
    0x48, 0x4e, 0x53, 0x57, 0x47, 0x52, 0x41, 0x50, 0x48, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06
  };
  static final String FIELD = "vec";

  /** The vector of document {@code doc}; the Rust test mirrors this exactly. */
  static float[] vectorFor(int doc, int dim) {
    float[] v = new float[dim];
    for (int i = 0; i < dim; i++) {
      v[i] = ((doc * 37 + i * 11) % 29) - 14 + (i % 3) * 0.5f;
    }
    return v;
  }

  static FieldInfo fieldInfo(int dim) {
    return new FieldInfo(
        FIELD, 0, false, true, false, IndexOptions.NONE, DocValuesType.NONE,
        DocValuesSkipIndexType.NONE, -1, Collections.emptyMap(), 0, 0, 0, dim,
        VectorEncoding.FLOAT32, VectorSimilarityFunction.EUCLIDEAN, false, false);
  }

  public static void main(String[] args) throws Exception {
    Path dir = Paths.get(args[0]);
    int numDocs = Integer.parseInt(args[1]);
    int dim = Integer.parseInt(args[2]);

    try (FSDirectory directory = FSDirectory.open(dir)) {
      SegmentInfo segmentInfo =
          new SegmentInfo(directory, Version.LATEST, Version.LATEST, "_hg", numDocs, false, false,
              new Lucene104Codec(), Collections.emptyMap(), SEGMENT_ID.clone(),
              Collections.emptyMap(), null);
      FieldInfo info = fieldInfo(dim);
      FieldInfos fieldInfos = new FieldInfos(new FieldInfo[] {info});
      SegmentWriteState writeState =
          new SegmentWriteState(InfoStream.NO_OUTPUT, directory, segmentInfo, fieldInfos, null,
              IOContext.DEFAULT);

      Lucene99HnswVectorsFormat format = new Lucene99HnswVectorsFormat();
      try (KnnVectorsWriter writer = format.fieldsWriter(writeState)) {
        @SuppressWarnings("unchecked")
        KnnFieldVectorsWriter<float[]> fieldWriter =
            (KnnFieldVectorsWriter<float[]>) writer.addField(info);
        for (int doc = 0; doc < numDocs; doc++) {
          fieldWriter.addValue(doc, vectorFor(doc, dim));
        }
        writer.flush(numDocs, null);
        writer.finish();
      }

      SegmentReadState readState =
          new SegmentReadState(directory, segmentInfo, fieldInfos, IOContext.DEFAULT);
      try (KnnVectorsReader reader = format.fieldsReader(readState)) {
        HnswGraph graph = ((HnswGraphProvider) reader).getGraph(FIELD);
        System.out.println("size=" + graph.size());
        System.out.println("num_levels=" + graph.numLevels());
        System.out.println("entry_node=" + graph.entryNode());
        System.out.println("max_conn=" + graph.maxConn());
        for (int level = 0; level < graph.numLevels(); level++) {
          HnswGraph.NodesIterator nodes = graph.getNodesOnLevel(level);
          while (nodes.hasNext()) {
            int node = nodes.nextInt();
            StringBuilder sb = new StringBuilder();
            sb.append("nbr=").append(level).append(':').append(node).append(':');
            graph.seek(level, node);
            boolean first = true;
            for (int n = graph.nextNeighbor();
                n != org.apache.lucene.search.DocIdSetIterator.NO_MORE_DOCS;
                n = graph.nextNeighbor()) {
              if (!first) {
                sb.append(',');
              }
              sb.append(n);
              first = false;
            }
            System.out.println(sb);
          }
        }
      }
    }
    System.out.println("read_ok=true");
  }
}
