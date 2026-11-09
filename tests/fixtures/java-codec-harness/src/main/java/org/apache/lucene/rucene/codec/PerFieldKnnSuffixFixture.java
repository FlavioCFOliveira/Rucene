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
import java.util.Arrays;
import org.apache.lucene.codecs.KnnVectorsFormat;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsFormat;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.store.FSDirectory;

/**
 * rmp #228: {@code PerFieldKnnVectorsFormat.FieldsWriter} keys its writers by format instance, so
 * a codec answering a new {@code Lucene99HnswVectorsFormat} for each of four vector fields writes
 * four suffixes, {@code _0} to {@code _3}. Writes one segment (no compound file) of one document
 * with fields {@code v0..v3} and prints the sorted file names.
 *
 * <p>Usage: {@code java -cp lucene-core-10.5.0.jar PerFieldKnnSuffixFixture.java <dir> <shared>}
 * where {@code shared} is {@code true} for one instance shared by every field.
 */
public class PerFieldKnnSuffixFixture {
  public static void main(String[] args) throws Exception {
    Path path = Path.of(args[0]);
    Files.createDirectories(path);
    boolean shared = Boolean.parseBoolean(args[1]);
    KnnVectorsFormat one = new Lucene99HnswVectorsFormat();
    IndexWriterConfig iwc =
        new IndexWriterConfig()
            .setUseCompoundFile(false)
            .setMergePolicy(NoMergePolicy.INSTANCE)
            .setCodec(
                new Lucene104Codec() {
                  @Override
                  public KnnVectorsFormat getKnnVectorsFormatForField(String field) {
                    return shared ? one : new Lucene99HnswVectorsFormat();
                  }
                });
    try (FSDirectory dir = FSDirectory.open(path);
        IndexWriter w = new IndexWriter(dir, iwc)) {
      Document doc = new Document();
      for (int i = 0; i < 4; i++) {
        doc.add(new KnnFloatVectorField("v" + i, new float[] {i, i + 1}));
      }
      w.addDocument(doc);
      w.commit();
      String[] files = dir.listAll();
      Arrays.sort(files);
      for (String f : files) {
        if (f.startsWith("_0_")) {
          System.out.println(f);
        }
      }
    }
  }
}
