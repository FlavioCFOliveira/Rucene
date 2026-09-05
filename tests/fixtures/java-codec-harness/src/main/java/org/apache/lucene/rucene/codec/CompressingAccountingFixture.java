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

import java.util.Collections;

import org.apache.lucene.codecs.TermVectorsWriter;
import org.apache.lucene.codecs.compressing.CompressionMode;
import org.apache.lucene.codecs.lucene90.compressing.Lucene90CompressingTermVectorsFormat;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.store.ByteBuffersDataOutput;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.util.Accountable;
import org.apache.lucene.util.ArrayUtil;
import org.apache.lucene.util.RamUsageEstimator;
import org.apache.lucene.util.StringHelper;
import org.apache.lucene.util.Version;

/**
 * Prints the heap estimates Lucene 10.5.0 reports for the pieces this port has
 * to reproduce, so the Rust side asserts against measured numbers rather than
 * against its own arithmetic.
 *
 * <p>What it measures:
 *
 * <ul>
 *   <li>{@code ByteBuffersDataOutput.ramBytesUsed()} after a range of write
 *       sizes — the block model steps in 1 KiB units until the hundredth block;
 *   <li>{@code Lucene90CompressingTermVectorsWriter.ramBytesUsed()} for a fresh
 *       writer, which is `4 * positionsBuf.length + lastTerm.bytes.length`
 *       — note that Java adds the four {@code int[]} buffers by their
 *       <em>element count</em>, not their byte size;
 *   <li>the number of child resources that writer reports, which is the pair
 *       {@code (termSuffixes, payloadBytes)};
 *   <li>{@code ArrayUtil.oversize(30, 1)}, the length Lucene gives
 *       {@code lastTerm.bytes}.
 * </ul>
 *
 * <p>Usage: {@code java ... CompressingAccountingFixture}.
 */
public final class CompressingAccountingFixture {

  private CompressingAccountingFixture() {}

  public static void main(String[] args) throws Exception {
    for (int n : new int[] {0, 1, 100, 1024, 1025, 32768, 100000}) {
      ByteBuffersDataOutput out = ByteBuffersDataOutput.newResettableInstance();
      for (int i = 0; i < n; i++) {
        out.writeByte((byte) i);
      }
      System.out.println("bbdo=" + n + ":" + out.ramBytesUsed());
    }
    System.out.println("oversize30=" + ArrayUtil.oversize(30, 1));
    System.out.println("num_bytes_object_ref=" + RamUsageEstimator.NUM_BYTES_OBJECT_REF);

    try (Directory dir = new ByteBuffersDirectory()) {
      SegmentInfo si =
          new SegmentInfo(
              dir,
              Version.LATEST,
              Version.LATEST,
              "_acct",
              1,
              false,
              false,
              org.apache.lucene.codecs.Codec.getDefault(),
              Collections.emptyMap(),
              StringHelper.randomId(),
              Collections.emptyMap(),
              null);
      Lucene90CompressingTermVectorsFormat fmt =
          new Lucene90CompressingTermVectorsFormat(
              "Lucene90TermVectorsData", "", CompressionMode.FAST, 1 << 12, 10, 10);
      try (TermVectorsWriter w = fmt.vectorsWriter(dir, si, IOContext.DEFAULT)) {
        System.out.println("tv_writer_fresh=" + w.ramBytesUsed());
        int children = 0;
        for (Accountable ignored : w.getChildResources()) {
          children++;
        }
        System.out.println("tv_writer_children=" + children);
      }
    }
    System.out.println("read_ok=true");
  }
}
