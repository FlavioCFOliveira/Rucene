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

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Collections;
import org.apache.lucene.codecs.CodecUtil;
import org.apache.lucene.codecs.lucene94.Lucene94FieldInfosFormat;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.search.Sort;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.util.StringHelper;
import org.apache.lucene.util.Version;

/**
 * Rucene task #601: writes a {@code .fnm} whose field count is the given value, with a valid index
 * header and footer and nothing in between, and reads it back through {@link
 * Lucene94FieldInfosFormat#read}.
 *
 * <p>Arguments: {@code <baseDir> <count>...}. For each count the file is {@code
 * <baseDir>/<count>/_0.fnm} (segment {@code _0}, id {@code 00..0f}, suffix {@code ""}), and the
 * outcome is printed as {@code RESULT <count> OK} or {@code RESULT <count> <SimpleName>|<message>},
 * followed by one {@code SUPPRESSED <count> <SimpleName>|<message>} line per suppressed exception.
 * The JVM heap bounds what {@code new FieldInfo[count]} may take, so the caller sets it.
 */
public final class HugeFnmCountFixture {

  private HugeFnmCountFixture() {}

  public static void main(String[] args) throws Exception {
    Path base = Paths.get(args[0]);
    byte[] id = new byte[StringHelper.ID_LENGTH];
    for (int i = 0; i < id.length; i++) {
      id[i] = (byte) i;
    }
    for (int a = 1; a < args.length; a++) {
      int count = Integer.parseInt(args[a]);
      Path dir = base.resolve(args[a]);
      Files.createDirectories(dir);
      try (Directory d = FSDirectory.open(dir)) {
        try (IndexOutput out = d.createOutput("_0.fnm", IOContext.DEFAULT)) {
          // FORMAT_CURRENT of Lucene94FieldInfosFormat is 2 (FORMAT_PARENT_FIELD).
          CodecUtil.writeIndexHeader(out, "Lucene94FieldInfos", 2, id, "");
          out.writeVInt(count);
          CodecUtil.writeFooter(out);
        }
        SegmentInfo si =
            new SegmentInfo(
                d,
                Version.LATEST,
                Version.LATEST,
                "_0",
                1,
                false,
                false,
                null,
                Collections.emptyMap(),
                id,
                Collections.emptyMap(),
                (Sort) null);
        try {
          new Lucene94FieldInfosFormat().read(d, si, "", IOContext.DEFAULT);
          System.out.println("RESULT " + count + " OK");
        } catch (Throwable t) {
          System.out.println("RESULT " + count + " " + describe(t));
          for (Throwable s : t.getSuppressed()) {
            System.out.println("SUPPRESSED " + count + " " + describe(s));
          }
        }
      }
    }
  }

  private static String describe(Throwable t) {
    return t.getClass().getSimpleName() + "|" + t.getMessage();
  }
}
