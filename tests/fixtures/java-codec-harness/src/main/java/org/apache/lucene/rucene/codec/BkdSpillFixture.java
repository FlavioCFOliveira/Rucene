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
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.FilterDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.util.IORunnable;
import org.apache.lucene.util.bkd.BKDConfig;
import org.apache.lucene.util.bkd.BKDWriter;

/**
 * Apache Lucene Core 10.5.0 oracle for the {@code BKDWriter.add}/{@code finish} path of rmp #244,
 * with and without spilling to temporary files.
 *
 * <p>Usage: {@code java -cp lucene-core-10.5.0.jar BkdSpillFixture.java <out-dir>}.
 *
 * <p>For every case of {@link #CASES} the fixture adds {@link #point} for every ordinal to a
 * {@code BKDWriter} whose temporary directory records every {@code createTempOutput} and {@code
 * deleteFile}, finishes it into {@code <case>.kdd}, {@code <case>.kdi} and {@code <case>.kdm}, and
 * prints one {@code event <case> create|delete <name>} line per temporary-file operation, in order.
 * A case whose {@code maxMBSortInHeap} cannot hold all of its points spills: {@code initPointWriter}
 * writes a {@code spill} file and {@code BKDRadixSelector} partitions through further files until a
 * slice fits in heap (BKDWriter.java:222-232, 930-1034, 1922-2126).
 */
public final class BkdSpillFixture {

  /** name, numDims, numIndexDims, bytesPerDim, maxPointsInLeafNode, points, maxMBSortInHeap. */
  static final Object[][] CASES = {
    {"spill2d", 2, 2, 4, 64, 5000, 0.005},
    {"heap2d", 2, 2, 4, 64, 5000, 16.0},
    {"spill3d2i", 3, 2, 4, 64, 3000, 0.004},
    {"heap3d2i", 3, 2, 4, 64, 3000, 16.0},
    {"spill4d", 4, 4, 2, 32, 4000, 0.002},
    {"heap4d", 4, 4, 2, 32, 4000, 16.0},
    // Duplicate values: the common prefix of a partition spans past the
    // dimension into the data dimensions and the doc ID, where
    // `heapRadixSelect`/`heapRadixSort`'s `dimCmpBytes` is negative.
    {"dupspill2d", 2, 2, 4, 64, 5000, 0.005},
    {"dupspill1d", 1, 1, 4, 16, 5000, 0.001},
    {"ddspill2d", 2, 2, 4, 64, 5000, 0.005},
    {"ddspill3d", 3, 2, 4, 64, 5000, 0.005},
    // Dimension 0 has the larger span, so it is split first although it is not
    // the last index dimension; its values are duplicated, so its partitions
    // tie-break on the doc ID, while dimension 1 decreases as the doc ID
    // grows: reading dimension 1 in place of the doc ID orders differently.
    {"dupsplit0", 2, 2, 4, 64, 5000, 0.005},
  };

  static final long[] MULTIPLIERS = {7919, 104729, 1299709, 15485863};
  static final long[] MODULI = {1000, 997, 13, 50021};

  private BkdSpillFixture() {}

  /** The packed value of point {@code i}: dimension {@code d} is a big-endian unsigned value. */
  static byte[] point(String name, int i, int numDims, int bytesPerDim) {
    byte[] packed = new byte[numDims * bytesPerDim];
    for (int d = 0; d < numDims; d++) {
      long value;
      if (name.startsWith("dup")) {
        value = name.equals("dupsplit0")
            ? (d == 0 ? (i % 2) * 1_000_000L : (5000 - i) % 4999)
            : (i * MULTIPLIERS[d]) % 3;
      } else if (name.startsWith("dd")) {
        value = d == 0 ? (i < 2500 ? 0 : 1000) : d == 1 ? (5000 - i) % 997 : (i * 31L) % 7;
      } else {
        value = (i * MULTIPLIERS[d]) % MODULI[d];
      }
      for (int b = 0; b < bytesPerDim; b++) {
        packed[d * bytesPerDim + b] = (byte) (value >>> (8 * (bytesPerDim - 1 - b)));
      }
    }
    return packed;
  }

  public static void main(String[] args) throws IOException {
    Path out = Paths.get(args[0]);
    Files.createDirectories(out);
    for (Object[] c : CASES) {
      String name = (String) c[0];
      int numDims = (Integer) c[1];
      int numIndexDims = (Integer) c[2];
      int bytesPerDim = (Integer) c[3];
      int maxPointsInLeafNode = (Integer) c[4];
      int points = (Integer) c[5];
      double maxMB = (Double) c[6];
      Path tempPath = out.resolve(name + "-tmp");
      Files.createDirectories(tempPath);
      try (FSDirectory outDir = FSDirectory.open(out);
          FSDirectory tempFs = FSDirectory.open(tempPath)) {
        Directory temp =
            new FilterDirectory(tempFs) {
              @Override
              public IndexOutput createTempOutput(String prefix, String suffix, IOContext context)
                  throws IOException {
                IndexOutput output = super.createTempOutput(prefix, suffix, context);
                System.out.println("event " + name + " create " + output.getName());
                return output;
              }

              @Override
              public void deleteFile(String file) throws IOException {
                System.out.println("event " + name + " delete " + file);
                super.deleteFile(file);
              }
            };
        BKDConfig config = new BKDConfig(numDims, numIndexDims, bytesPerDim, maxPointsInLeafNode);
        try (BKDWriter writer =
                new BKDWriter(points / 2 + 1, temp, "bkd" + name, config, maxMB, points);
            IndexOutput meta = outDir.createOutput(name + ".kdm", IOContext.DEFAULT);
            IndexOutput index = outDir.createOutput(name + ".kdi", IOContext.DEFAULT);
            IndexOutput data = outDir.createOutput(name + ".kdd", IOContext.DEFAULT)) {
          for (int i = 0; i < points; i++) {
            writer.add(point(name, i, numDims, bytesPerDim), i / 2);
          }
          IORunnable finalizer = writer.finish(meta, index, data);
          finalizer.run();
        }
        System.out.println("left " + name + " " + String.join(",", tempFs.listAll()));
      }
    }
    System.out.println("done");
  }
}
