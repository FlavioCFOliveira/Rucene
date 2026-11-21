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
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.MergePolicy;
import org.apache.lucene.index.MergeTrigger;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.TemporalMergePolicy;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.InfoStream;

/**
 * Reference tool for {@code TemporalMergePolicy}: writes, with a real Apache Lucene Core 10.5.0
 * {@link IndexWriter} and no merging, a fixed set of segments whose {@code ts} {@link LongPoint}
 * values are seconds, milliseconds or microseconds of dates in 2000 and 2001, some in compound
 * files and some not, some with deletions; then asks several configurations of the policy for
 * merges and prints them. The policy reads every segment's date range from its files through the
 * codec ({@code TemporalMergePolicy.java:592-700}).
 *
 * <p>Usage: {@code java -cp lucene-core-10.5.0.jar TemporalMergeFixture.java <index-dir>}.
 *
 * <p>Each output line is {@code <case> <spec>}, where {@code <spec>} is {@code null} or the merges
 * separated by {@code |}, each the comma-separated segment names. Forced merges group the
 * segments of a window through a {@code HashMap} keyed by identity, whose order is unspecified, so
 * the {@code forced} case prints each merge's names sorted and the merges sorted; the other cases
 * print the policy's own order.
 */
public final class TemporalMergeFixture {

  /** 2000-01-01T00:00:00Z, in seconds. */
  private static final long Y2000 = 946_684_800L;

  private static final long DAY = 86_400L;

  private TemporalMergeFixture() {}

  /** One segment: the unit of its timestamps, its day, its document count, its compound flag. */
  private record Seg(String unit, long day, int docs, boolean compound) {}

  private static final List<Seg> SEGMENTS =
      List.of(
          new Seg("s", 0, 3, true),
          new Seg("ms", 0, 5, false),
          new Seg("us", 0, 2, true),
          new Seg("s", 0, 4, false),
          new Seg("s", 1, 6, true),
          new Seg("ms", 1, 1, false),
          new Seg("s", 2, 3, true),
          new Seg("s", 40, 2, false),
          new Seg("s", 400, 7, true),
          new Seg("s", 400, 2, false),
          new Seg("s", 401, 3, true),
          new Seg("none", 0, 2, false));

  public static void main(String[] args) throws IOException {
    try (Directory dir = FSDirectory.open(Paths.get(args[0]))) {
      write(dir);
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      MergePolicy.MergeContext context = new Context();

      TemporalMergePolicy daily = flat(DAY);
      daily.setMinThreshold(2);
      System.out.println("daily " + render(daily.findMerges(MergeTrigger.EXPLICIT, infos, context)));

      TemporalMergePolicy aggressive = flat(DAY);
      aggressive.setMinThreshold(2).setMaxThreshold(3).setCompactionRatio(1.0);
      System.out.println(
          "aggressive " + render(aggressive.findMerges(MergeTrigger.EXPLICIT, infos, context)));

      TemporalMergePolicy exponential = new TemporalMergePolicy().setTemporalField("ts");
      System.out.println(
          "exponential " + render(exponential.findMerges(MergeTrigger.EXPLICIT, infos, context)));

      TemporalMergePolicy tooOld = new TemporalMergePolicy().setTemporalField("ts");
      tooOld.setMaxAgeSeconds(DAY);
      System.out.println("tooOld " + render(tooOld.findMerges(MergeTrigger.EXPLICIT, infos, context)));

      Map<SegmentCommitInfo, Boolean> all = new HashMap<>();
      for (SegmentCommitInfo info : infos) {
        all.put(info, Boolean.TRUE);
      }
      TemporalMergePolicy forced = flat(DAY);
      forced.setMinThreshold(2);
      System.out.println(
          "forced " + renderSorted(forced.findForcedMerges(infos, 1, all, context)));

      TemporalMergePolicy deletes = flat(7 * DAY);
      deletes.setMinThreshold(2).setMaxThreshold(2);
      System.out.println("deletes " + render(deletes.findForcedDeletesMerges(infos, context)));
    }
  }

  private static TemporalMergePolicy flat(long baseTimeSeconds) {
    return new TemporalMergePolicy()
        .setTemporalField("ts")
        .setBaseTimeInSeconds(baseTimeSeconds)
        .disableExponentialBuckets();
  }

  /** Writes one commit per segment of {@link #SEGMENTS}, then the deletions. */
  private static void write(Directory dir) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    try (IndexWriter writer = new IndexWriter(dir, config)) {
      for (int s = 0; s < SEGMENTS.size(); s++) {
        Seg seg = SEGMENTS.get(s);
        writer.getConfig().setUseCompoundFile(seg.compound());
        for (int d = 0; d < seg.docs(); d++) {
          Document doc = new Document();
          doc.add(new StringField("id", s + "-" + d, Field.Store.NO));
          long seconds = Y2000 + seg.day() * DAY + d * 60L + s;
          switch (seg.unit()) {
            case "s" -> doc.add(new LongPoint("ts", seconds));
            case "ms" -> doc.add(new LongPoint("ts", seconds * 1_000L));
            case "us" -> doc.add(new LongPoint("ts", seconds * 1_000_000L));
            default -> {}
          }
          writer.addDocument(doc);
        }
        writer.commit();
      }
      // Distinct deletion ratios: 1/3, 2/5, 1/4 and 3/6.
      writer.deleteDocuments(new Term("id", "0-0"));
      writer.deleteDocuments(new Term("id", "1-0"), new Term("id", "1-1"));
      writer.deleteDocuments(new Term("id", "3-0"));
      writer.deleteDocuments(new Term("id", "4-0"), new Term("id", "4-1"), new Term("id", "4-2"));
      writer.commit();
    }
  }

  /** A merge context with no merge running, counting the committed deletions. */
  private static final class Context implements MergePolicy.MergeContext {
    @Override
    public int numDeletesToMerge(SegmentCommitInfo info) {
      return info.getDelCount() + info.getSoftDelCount();
    }

    @Override
    public int numDeletedDocs(SegmentCommitInfo info) {
      return info.getDelCount() + info.getSoftDelCount();
    }

    @Override
    public InfoStream getInfoStream() {
      return InfoStream.NO_OUTPUT;
    }

    @Override
    public Set<SegmentCommitInfo> getMergingSegments() {
      return Collections.emptySet();
    }
  }

  private static String render(MergePolicy.MergeSpecification spec) {
    if (spec == null) {
      return "null";
    }
    List<String> merges = new ArrayList<>();
    for (MergePolicy.OneMerge merge : spec.merges) {
      List<String> names = new ArrayList<>();
      for (SegmentCommitInfo info : merge.segments) {
        names.add(info.info.name);
      }
      merges.add(String.join(",", names));
    }
    return String.join("|", merges);
  }

  private static String renderSorted(MergePolicy.MergeSpecification spec) {
    if (spec == null) {
      return "null";
    }
    List<String> merges = new ArrayList<>();
    for (MergePolicy.OneMerge merge : spec.merges) {
      List<String> names = new ArrayList<>();
      for (SegmentCommitInfo info : merge.segments) {
        names.add(info.info.name);
      }
      Collections.sort(names);
      merges.add(String.join(",", names));
    }
    Collections.sort(merges);
    return String.join("|", merges);
  }
}
