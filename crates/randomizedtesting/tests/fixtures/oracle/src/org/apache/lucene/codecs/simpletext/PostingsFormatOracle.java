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

package org.apache.lucene.codecs.simpletext;

import java.io.File;
import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;
import java.util.stream.Stream;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.codecs.KnnVectorsFormat;
import org.apache.lucene.codecs.PostingsFormat;
import org.apache.lucene.codecs.bloom.BloomFilteringPostingsFormat;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.lucene104.Lucene104PostingsFormat;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.TermState;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.NIOFSDirectory;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.StringHelper;
import org.apache.lucene.util.automaton.CompiledAutomaton;
import org.apache.lucene.util.automaton.RegExp;

/**
 * The generic oracle of the {@code lucene/codecs} formats other than {@code SimpleText}: it writes
 * the corpus of {@link SimpleTextOracle} ({@code nocfs}, {@code cfs}, {@code edge}) with {@code
 * Lucene104Codec}, whose per-field formats route the fields to the format under test, and records
 * into {@code <out>/rucene-codecs/}:
 *
 * <ul>
 *   <li>{@code <prefix>_files.tsv}: every file of every index, as {@code SimpleTextOracle.files};
 *   <li>{@code <prefix>_reads.tsv}: what Lucene reads back ({@link SimpleTextReadOracle#dump});
 *   <li>{@code <prefix>_errors.tsv}: the exception a corrupted copy of {@code nocfs} raises on open,
 *       {@code case, class, message}, for three corruptions of each file of the first segment that
 *       carries the format's name and one of the extensions given: {@code truncate} (the last byte
 *       removed), {@code id} (the last byte of the header's segment id flipped) and {@code checksum}
 *       (the last byte of the footer's checksum flipped); the directory is masked as {@code $D},
 *       the segment id as {@code $ID}, the flipped id as {@code $BADID} and the two CRC-32 values
 *       of a checksum mismatch, which cover the segment id, as {@code $CRC};
 *   <li>{@code <prefix>_enum.tsv} ({@code postings} only): the terms-enum calls {@link
 *       SimpleTextReadOracle#dump} does not make ({@link #enumTrace}): ords, term states and
 *       automaton intersections;
 *   <li>{@code <prefix>_spi.tsv}: the service lookups of the format.
 * </ul>
 *
 * <p>Arguments: {@code <out> <prefix> <kind> <format> <extension>...}; {@code kind} is {@code
 * postings} (every field but {@code e_id} uses the format) or {@code knn} (the byte-vector fields
 * {@code vb} and {@code e_vb_*} use it); {@code format} is an SPI name, or {@code
 * BloomFilter(Lucene104)} for {@code new BloomFilteringPostingsFormat(new
 * Lucene104PostingsFormat())}. The blockterms assemblies are {@code lucene-test-framework}'s
 * {@code LuceneFixedGap}, {@code LuceneVarGapFixedInterval} and {@code
 * LuceneVarGapDocFreqInterval}, found by their SPI names; {@code BlockTreeOrds}, whose terms enum
 * supports {@code ord()} and {@code seekExact(long)}, is found by its SPI name too.
 */
public final class PostingsFormatOracle {

  private PostingsFormatOracle() {}

  static PostingsFormat postingsFormat(String format) {
    if (format.equals("BloomFilter(Lucene104)")) {
      return new BloomFilteringPostingsFormat(new Lucene104PostingsFormat());
    }
    return PostingsFormat.forName(format);
  }

  static Codec codec(String kind, String format) {
    if (kind.equals("postings")) {
      PostingsFormat pf = postingsFormat(format);
      return new Lucene104Codec() {
        @Override
        public PostingsFormat getPostingsFormatForField(String field) {
          return field.equals("e_id") ? super.getPostingsFormatForField(field) : pf;
        }
      };
    }
    KnnVectorsFormat kf = KnnVectorsFormat.forName(format);
    return new Lucene104Codec() {
      @Override
      public KnnVectorsFormat getKnnVectorsFormatForField(String field) {
        return field.equals("vb") || field.startsWith("e_vb")
            ? kf
            : super.getKnnVectorsFormatForField(field);
      }
    };
  }

  /** The file of the first segment of {@code dir} with the format's name and {@code ext}. */
  static Path formatFile(Path dir, String name, String ext) throws IOException {
    String segment;
    try (Directory d = new NIOFSDirectory(dir)) {
      segment = SegmentInfos.readLatestCommit(d).info(0).info.name;
    }
    try (Stream<Path> s = Files.list(dir)) {
      List<Path> found =
          s.filter(
                  p -> {
                    String f = p.getFileName().toString();
                    return f.startsWith(segment + "_" + name + "_") && f.endsWith("." + ext);
                  })
              .sorted()
              .toList();
      if (found.isEmpty()) {
        throw new IllegalStateException("no " + name + " ." + ext + " file in " + dir);
      }
      return found.get(0);
    }
  }

  /** The offset of the segment id in a file starting with {@code CodecUtil.writeIndexHeader}. */
  static int idOffset(byte[] bytes) {
    int codecLength = bytes[4] & 0xff;
    return 4 + 1 + codecLength + 4;
  }

  static void corrupt(
      String name, Path source, Path work, String formatName, String ext, String kind,
      PrintWriter w)
      throws IOException {
    String label = ext + "-" + kind;
    Path dir = work.resolve("err-" + label);
    SimpleTextOracle.copy(source, dir);
    String id;
    try (Directory d = new NIOFSDirectory(dir)) {
      id = StringHelper.idToString(SegmentInfos.readLatestCommit(d).info(0).info.getId());
    }
    Path file = formatFile(dir, formatName, ext);
    byte[] bytes = Files.readAllBytes(file);
    String badId = "-";
    switch (kind) {
      case "truncate" -> bytes = Arrays.copyOf(bytes, bytes.length - 1);
      case "id" -> {
        int last = idOffset(bytes) + StringHelper.ID_LENGTH - 1;
        bytes[last] ^= (byte) 0xff;
        badId =
            StringHelper.idToString(
                Arrays.copyOfRange(bytes, idOffset(bytes), idOffset(bytes) + StringHelper.ID_LENGTH));
      }
      case "checksum" -> bytes[bytes.length - 1] ^= 1;
      default -> throw new IllegalArgumentException(kind);
    }
    Files.write(file, bytes);
    String real = dir.toRealPath().toString();
    try (Directory d = new NIOFSDirectory(dir);
        DirectoryReader r = DirectoryReader.open(d)) {
      w.println(label + "\tnone\t-");
    } catch (Throwable t) {
      String message = String.valueOf(t.getMessage()).replace(real, "$D").replace('\n', ' ');
      if (!badId.equals("-")) {
        message = message.replace(badId, "$BADID");
      }
      // The CRC-32 of a file covers its segment id, which differs between two writers.
      message =
          message.replaceAll(
              "expected=[0-9a-f]+ actual=[0-9a-f]+", "expected=\\$CRC actual=\\$CRC");
      w.println(label + "\t" + t.getClass().getName() + "\t" + message.replace(id, "$ID"));
    }
  }

  /** The regular expressions of the intersections of {@link #enumTrace}. */
  static final String[] REGEXPS = {"w1.*", "w[2-4]", ".*7", "t.", "id-00[0-3].", "e.*", ".*"};

  static String hex(BytesRef b) {
    return b == null ? "null" : SimpleTextReadOracle.hex(b);
  }

  /** The first document of the enum's postings. */
  static int firstDoc(TermsEnum te) throws IOException {
    return te.postings(null, PostingsEnum.NONE).nextDoc();
  }

  static String error(Throwable t) {
    return "!" + t.getClass().getSimpleName();
  }

  /**
   * The terms-enum calls the dump does not make, per leaf and field with terms of the format under
   * test (every field but {@code e_id}): {@code ord} (the
   * ord of the first term, then {@code seekExact(ord)} to four ords and {@code next}); term states
   * (every fifth term's {@code termState()}, then, on a fresh enum and in reverse order, {@code
   * seekExact(term, state)}, its stats and first document and {@code next}); and {@code
   * intersect} with each of {@link #REGEXPS} ({@code new CompiledAutomaton(new
   * RegExp(r).toAutomaton(), false, false)}), from the start and from the field's second term.
   */
  static List<String> enumTrace(Path path) throws IOException {
    List<String> out = new java.util.ArrayList<>();
    try (Directory dir = new NIOFSDirectory(path);
        DirectoryReader reader = DirectoryReader.open(dir)) {
      for (LeafReaderContext ctx : reader.leaves()) {
        out.add("leaf " + ctx.ord);
        for (FieldInfo fi : ctx.reader().getFieldInfos()) {
          Terms terms = ctx.reader().terms(fi.name);
          // e_id keeps Lucene104PostingsFormat: its enum is lucene-core's, not the format's.
          if (terms == null || fi.name.equals("e_id")) {
            continue;
          }
          long size = terms.size();
          out.add(" field " + fi.name + " size=" + size);
          StringBuilder sb = new StringBuilder("  ord");
          try {
            TermsEnum te = terms.iterator();
            te.next();
            sb.append(" first=").append(te.ord());
            for (long ord : new long[] {0, size / 3, size / 2, size - 1}) {
              te.seekExact(ord);
              sb.append(" ").append(ord).append("=").append(hex(te.term()));
              sb.append("/").append(te.docFreq()).append("/").append(te.totalTermFreq());
              BytesRef next = te.next();
              sb.append(" next=").append(hex(next));
              if (next != null) {
                sb.append("@").append(te.ord());
              }
            }
          } catch (Throwable t) {
            sb.append(' ').append(error(t));
          }
          out.add(sb.toString());
          List<BytesRef> kept = new java.util.ArrayList<>();
          List<TermState> states = new java.util.ArrayList<>();
          TermsEnum te = terms.iterator();
          int i = 0;
          for (BytesRef t = te.next(); t != null; t = te.next(), i++) {
            if (i % 5 == 0) {
              kept.add(BytesRef.deepCopyOf(t));
              states.add(te.termState());
            }
          }
          sb = new StringBuilder("  state");
          TermsEnum fresh = terms.iterator();
          for (int k = kept.size() - 1; k >= 0; k--) {
            try {
              fresh.seekExact(kept.get(k), states.get(k));
              sb.append(' ').append(hex(fresh.term())).append('/').append(fresh.docFreq());
              sb.append('/').append(fresh.totalTermFreq()).append('/').append(firstDoc(fresh));
              sb.append(" next=").append(hex(fresh.next()));
            } catch (Throwable t) {
              sb.append(' ').append(error(t));
            }
          }
          out.add(sb.toString());
          BytesRef second = null;
          TermsEnum it = terms.iterator();
          if (it.next() != null) {
            second = it.next() == null ? null : BytesRef.deepCopyOf(it.term());
          }
          for (String r : REGEXPS) {
            CompiledAutomaton compiled =
                new CompiledAutomaton(new RegExp(r).toAutomaton(), false, false);
            for (BytesRef start : new BytesRef[] {null, second}) {
              sb = new StringBuilder("  intersect " + r + " from=" + hex(start));
              try {
                TermsEnum ie = terms.intersect(compiled, start);
                for (BytesRef t = ie.next(); t != null; t = ie.next()) {
                  sb.append(' ').append(hex(t)).append('/').append(ie.docFreq());
                  sb.append('/').append(ie.totalTermFreq()).append('/').append(firstDoc(ie));
                }
              } catch (Throwable t) {
                sb.append(' ').append(error(t));
              }
              out.add(sb.toString());
            }
          }
        }
      }
    }
    return out;
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-codecs");
    out.mkdirs();
    String prefix = args[1];
    String kind = args[2];
    String format = args[3];
    List<String> extensions = Arrays.asList(args).subList(4, args.length);
    Codec codec = codec(kind, format);
    String formatName =
        kind.equals("postings")
            ? postingsFormat(format).getName()
            : KnnVectorsFormat.forName(format).getName();
    Path work = Files.createTempDirectory(prefix + "-oracle");
    Path nocfs = work.resolve("nocfs");
    Path cfs = work.resolve("cfs");
    Path edge = work.resolve("edge");
    try (Directory d = new NIOFSDirectory(nocfs)) {
      SimpleTextOracle.writeCorpus(d, false, codec);
    }
    try (Directory d = new NIOFSDirectory(cfs)) {
      SimpleTextOracle.writeCorpus(d, true, codec);
    }
    try (Directory d = new NIOFSDirectory(edge)) {
      SimpleTextOracle.writeEdge(d, codec);
    }
    try (PrintWriter w = new PrintWriter(new File(out, prefix + "_files.tsv"), "UTF-8")) {
      w.println("# case\tfile\tlength\tfnv1a64\tcontent (\\xHH, \\\\)");
      SimpleTextOracle.files("nocfs", nocfs, w);
      SimpleTextOracle.files("cfs", cfs, w);
      SimpleTextOracle.files("edge", edge, w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, prefix + "_reads.tsv"), "UTF-8")) {
      w.println("# case\tline of SimpleTextReadOracle.dump (\\xHH, \\\\)");
      SimpleTextOracle.reads("nocfs", nocfs, w);
      SimpleTextOracle.reads("cfs", cfs, w);
      SimpleTextOracle.reads("edge", edge, w);
    }
    if (kind.equals("postings")) {
      try (PrintWriter w = new PrintWriter(new File(out, prefix + "_enum.tsv"), "UTF-8")) {
        w.println("# case\tline of PostingsFormatOracle.enumTrace");
        for (String c : new String[] {"nocfs", "cfs", "edge"}) {
          for (String line : enumTrace(work.resolve(c))) {
            w.println(c + "\t" + line);
          }
        }
      }
    }
    try (PrintWriter w = new PrintWriter(new File(out, prefix + "_errors.tsv"), "UTF-8")) {
      w.println(
          "# case (extension-corruption)\tclass\tmessage ($D = the index directory, $ID = the"
              + " segment id, $BADID = the flipped id)");
      for (String ext : extensions) {
        for (String corruption : new String[] {"truncate", "id", "checksum"}) {
          corrupt(prefix, nocfs, work, formatName, ext, corruption, w);
        }
      }
    }
    try (PrintWriter w = new PrintWriter(new File(out, prefix + "_spi.tsv"), "UTF-8")) {
      w.println("# lookup\tresult");
      if (kind.equals("postings")) {
        PostingsFormat pf = postingsFormat(format);
        w.println("getName\t" + pf.getName());
        w.println("toString\t" + pf);
        try {
          PostingsFormat found = PostingsFormat.forName(pf.getName());
          w.println("forName.getClass\t" + found.getClass().getName());
          w.println("forName.toString\t" + found);
        } catch (IllegalArgumentException e) {
          w.println("forName\t" + e.getClass().getName());
        }
      } else {
        KnnVectorsFormat kf = KnnVectorsFormat.forName(format);
        w.println("getName\t" + kf.getName());
        w.println("toString\t" + kf);
        w.println("forName.getClass\t" + kf.getClass().getName());
      }
    }
    org.apache.lucene.util.IOUtils.rm(work);
  }
}
