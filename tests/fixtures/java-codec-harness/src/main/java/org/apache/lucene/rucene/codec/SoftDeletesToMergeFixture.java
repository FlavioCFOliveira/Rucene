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
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.FilterMergePolicy;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.MergePolicy;
import org.apache.lucene.index.MergeTrigger;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SoftDeletesRetentionMergePolicy;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.store.ByteBuffersDirectory;

/**
 * Reference tool for {@code IndexWriter.numDeletesToMerge}: a segment of ten
 * documents, five of them soft-deleted by updates, three of those still
 * matched by the retention query of a {@link SoftDeletesRetentionMergePolicy}.
 * A merge policy asks the writer how many deletes a merge of each segment
 * would reclaim, from inside {@code findMerges} where it is handed the live
 * segments, and the tool prints the answers.
 *
 * <p>This is what {@code tests/index_writer_reopen_and_soft_deletes.rs} judges
 * Rucene's {@code num_deletes_to_merge} against.
 *
 * <p>Command-line usage: {@code java ... SoftDeletesToMergeFixture}.
 */
public final class SoftDeletesToMergeFixture {

  private SoftDeletesToMergeFixture() {}

  static Document doc(int i, int version) {
    Document d = new Document();
    d.add(new StringField("id", "id" + i, Field.Store.NO));
    d.add(new StringField("keep", i % 2 == 0 ? "yes" : "no", Field.Store.NO));
    d.add(new NumericDocValuesField("version", version));
    return d;
  }

  /** Runs the case. */
  public static void main(String[] args) throws IOException {
    MergePolicy retention =
        new SoftDeletesRetentionMergePolicy(
            "soft", () -> new TermQuery(new Term("keep", "yes")), NoMergePolicy.INSTANCE);
    MergePolicy asking =
        new FilterMergePolicy(retention) {
          @Override
          public MergeSpecification findMerges(
              MergeTrigger trigger, SegmentInfos infos, MergeContext context) throws IOException {
            for (SegmentCommitInfo info : infos) {
              System.out.println(
                  "numDeletesToMerge "
                      + info.info.name
                      + " "
                      + context.numDeletesToMerge(info)
                      + " delCount="
                      + info.getDelCount()
                      + " softDelCount="
                      + info.getSoftDelCount());
            }
            return null;
          }
        };
    try (ByteBuffersDirectory dir = new ByteBuffersDirectory()) {
      IndexWriterConfig config = new IndexWriterConfig();
      config.setSoftDeletesField("soft");
      config.setMergePolicy(asking);
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        for (int i = 0; i < 10; i++) {
          writer.addDocument(doc(i, 1));
        }
        writer.commit();
        for (int i = 0; i < 5; i++) {
          writer.softUpdateDocument(
              new Term("id", "id" + i), doc(i, 2), new NumericDocValuesField("soft", 1));
        }
        writer.commit();
        writer.maybeMerge();
      }
    }
  }
}
