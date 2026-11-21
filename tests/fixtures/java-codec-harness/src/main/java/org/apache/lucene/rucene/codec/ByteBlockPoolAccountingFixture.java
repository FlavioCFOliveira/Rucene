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

import org.apache.lucene.util.ByteBlockPool;
import org.apache.lucene.util.Counter;
import org.apache.lucene.util.RamUsageEstimator;

/**
 * Prints the two different numbers Lucene 10.5.0 reports about the memory of a
 * {@link ByteBlockPool}, so the Rust port asserts against measured values
 * rather than against its own arithmetic.
 *
 * <p>The two numbers are routinely conflated and are not the same thing:
 *
 * <ul>
 *   <li><b>the heap estimate</b> — {@code ByteBlockPool.ramBytesUsed()}, which
 *       is {@code shallowSizeOfInstance(ByteBlockPool.class)} plus
 *       {@code shallowSizeOf(buffers)} — the whole {@code byte[][]} reference
 *       array, including the slots that hold no block — plus
 *       {@code sizeOfObject(buf)} for each block, which adds a 16-byte array
 *       header to each one. It therefore never equals a whole number of blocks;
 *   <li><b>the allocator's charge</b> — what
 *       {@code ByteBlockPool.DirectTrackingAllocator} adds to the shared
 *       {@code Counter}, which is exactly {@code BYTE_BLOCK_SIZE} per block and
 *       nothing else.
 * </ul>
 *
 * <p>The reference array starts at {@code new byte[10][]} and grows through
 * {@code ArrayUtil.oversize(buffers.length + 1, NUM_BYTES_OBJECT_REF)}; it is
 * never shrunk, so {@code reset(false, false)} — which nulls every slot and
 * frees every block — leaves the grown array behind and the estimate does not
 * fall back to the value it had when the pool was new. The {@code ram_reset}
 * and {@code charge_reset} rows pin that.
 *
 * <p>Usage: {@code java ... ByteBlockPoolAccountingFixture}.
 */
public final class ByteBlockPoolAccountingFixture {

  /** Block counts to measure: 0 to 12 crosses the first growth of the reference array. */
  private static final int[] BLOCK_COUNTS = {0, 1, 2, 3, 9, 10, 11, 12, 20, 30};

  private ByteBlockPoolAccountingFixture() {}

  public static void main(String[] args) {
    System.out.println("base=" + RamUsageEstimator.shallowSizeOfInstance(ByteBlockPool.class));
    System.out.println("block_size=" + ByteBlockPool.BYTE_BLOCK_SIZE);
    System.out.println(
        "size_of_block=" + RamUsageEstimator.sizeOf(new byte[ByteBlockPool.BYTE_BLOCK_SIZE]));
    System.out.println("num_bytes_object_ref=" + RamUsageEstimator.NUM_BYTES_OBJECT_REF);
    System.out.println("shallow_of_ten_refs=" + RamUsageEstimator.shallowSizeOf(new byte[10][]));

    for (int blocks : BLOCK_COUNTS) {
      Counter counter = Counter.newCounter();
      ByteBlockPool pool = new ByteBlockPool(new ByteBlockPool.DirectTrackingAllocator(counter));
      for (int i = 0; i < blocks; i++) {
        pool.nextBuffer();
      }
      System.out.println("ram=" + blocks + ":" + pool.ramBytesUsed());
      System.out.println("charge=" + blocks + ":" + counter.get());

      // reset(zeroFillBuffers=false, reuseFirst=false): every block is released
      // to the allocator, but the reference array keeps the length it grew to.
      pool.reset(false, false);
      System.out.println("ram_reset=" + blocks + ":" + pool.ramBytesUsed());
      System.out.println("charge_reset=" + blocks + ":" + counter.get());
    }
    System.out.println("ok=true");
  }
}
