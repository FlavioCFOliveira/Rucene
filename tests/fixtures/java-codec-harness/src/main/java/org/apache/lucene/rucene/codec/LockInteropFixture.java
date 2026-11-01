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

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.nio.file.Paths;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.LockObtainFailedException;

/**
 * Reference tool for {@code write.lock} interoperation between Lucene and Rucene. An {@code
 * IndexWriter} takes {@code write.lock} through {@code FSDirectory.open}'s default {@code
 * NativeFSLockFactory}, whose {@code FileChannel.tryLock()} is a POSIX {@code fcntl} record lock
 * on Unix ({@code NativeFSLockFactory.java:112-113}).
 *
 * <ul>
 *   <li>{@code hold <dir>}: opens a writer, prints {@code held}, waits for a line (or the end) on
 *       standard input, closes the writer and prints {@code released}.
 *   <li>{@code try <dir>}: opens and closes a writer, printing {@code opened}, or {@code refused:
 *       LockObtainFailedException: <message>} when the lock is held.
 * </ul>
 */
public class LockInteropFixture {
  public static void main(String[] args) throws Exception {
    String mode = args[0];
    Path path = Paths.get(args[1]);
    if (mode.equals("hold")) {
      try (Directory dir = FSDirectory.open(path);
          IndexWriter writer = new IndexWriter(dir, new IndexWriterConfig())) {
        System.out.println("held");
        System.out.flush();
        new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8)).readLine();
      }
      System.out.println("released");
    } else if (mode.equals("try")) {
      try (Directory dir = FSDirectory.open(path);
          IndexWriter writer = new IndexWriter(dir, new IndexWriterConfig())) {
        System.out.println("opened");
      } catch (LockObtainFailedException e) {
        System.out.println("refused: LockObtainFailedException: " + e.getMessage());
      }
    } else {
      throw new IllegalArgumentException("unknown mode: " + mode);
    }
  }
}
