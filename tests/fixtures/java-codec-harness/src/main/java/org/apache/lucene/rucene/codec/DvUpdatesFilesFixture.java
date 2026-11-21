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
import java.util.HexFormat;
import java.util.Map;
import java.util.Set;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.Term;
import org.apache.lucene.store.FSDirectory;

/**
 * Apache Lucene Core 10.5.0 oracle for the order in which {@code segments_N} lists a segment's
 * doc-values update files (round 2 of the codecs effort).
 *
 * <p>Usage: {@code java -cp lucene-core-10.5.0.jar DvUpdatesFilesFixture.java <index-dir>}.
 *
 * <p>Ten documents carry {@code id} (field 0) and {@code f0}..{@code f19} (fields 1..20). After the
 * first commit, {@link #UPDATED} fields are updated; field numbers 2 and 18, and 4 and 20, share a
 * bucket of the 16-slot {@code HashMap<Integer, Set<String>>} that {@code SegmentCommitInfo} keeps
 * them in, so the order {@code SegmentInfos.write} writes them in depends on Java's chain order
 * (SegmentInfos.java:696-702). The fixture prints the commit's ids and the map's iteration order.
 */
public final class DvUpdatesFilesFixture {

  static final String[] UPDATED = {"f17", "f1", "f19", "f3", "f8"};

  private DvUpdatesFilesFixture() {}

  public static void main(String[] args) throws IOException {
    Path path = Paths.get(args[0]);
    Files.createDirectories(path);
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    config.setUseCompoundFile(false);
    try (FSDirectory dir = FSDirectory.open(path);
        IndexWriter writer = new IndexWriter(dir, config)) {
      for (int i = 0; i < 10; i++) {
        Document doc = new Document();
        doc.add(new StringField("id", "d" + i, Field.Store.NO));
        for (int f = 0; f < 20; f++) {
          doc.add(new NumericDocValuesField("f" + f, i * 100 + f));
        }
        writer.addDocument(doc);
      }
      writer.commit();
      int value = 1000;
      // One commit per update: `FrozenBufferedUpdates` holds its field updates in a
      // `Map.copyOf`, whose iteration order is salted per JVM, so several fields
      // updated in one commit get their generations in a run-dependent order.
      for (String field : UPDATED) {
        writer.updateNumericDocValue(new Term("id", "d3"), field, value++);
        writer.commit();
      }
    }
    try (FSDirectory dir = FSDirectory.open(path)) {
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      HexFormat hex = HexFormat.of();
      System.out.println("segments=" + infos.getSegmentsFileName());
      System.out.println("infos_id=" + hex.formatHex(infos.getId()));
      System.out.println("version=" + infos.getVersion());
      SegmentCommitInfo commit = infos.info(0);
      System.out.println("si_id=" + hex.formatHex(commit.info.getId()));
      System.out.println("sci_id=" + hex.formatHex(commit.getId()));
      for (Map.Entry<Integer, Set<String>> e : commit.getDocValuesUpdatesFiles().entrySet()) {
        System.out.println("dv " + e.getKey() + " " + String.join(",", e.getValue()));
      }
    }
  }
}
