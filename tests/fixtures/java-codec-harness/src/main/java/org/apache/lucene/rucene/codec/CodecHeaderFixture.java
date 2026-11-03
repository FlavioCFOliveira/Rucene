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

import org.apache.lucene.codecs.CodecUtil;
import org.apache.lucene.index.CorruptIndexException;
import org.apache.lucene.index.IndexFormatTooNewException;
import org.apache.lucene.index.IndexFormatTooOldException;
import java.nio.file.Paths;
import org.apache.lucene.store.ByteBuffersDataOutput;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.ChecksumIndexInput;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.store.NIOFSDirectory;

/**
 * Reference tool for the exceptions {@code CodecUtil.checkHeaderNoMagic} raises
 * ({@code CodecUtil.java:201-218}): a {@code TestCodec} header of version 5 ({@code plain.bin})
 * and an index header of version 5 ({@code index.bin}) are read through an {@code
 * NIOFSDirectory}, with a plain and a checksum input, against a version range below the file's,
 * above it, and under another codec name. The tool prints each exception's class and message,
 * and the resource and version triple of the two format exceptions.
 *
 * <p>It also reads, through the same two inputs, a file without the codec magic ({@code
 * nomagic.bin}, {@code CodecUtil.java:182-194}), the index header against another id and against
 * an id of the wrong length ({@code CodecUtil.java:363-375}), an index header whose suffix is
 * not valid UTF-8 ({@code badsuffix.bin}, {@code CodecUtil.java:378-389}), and a header followed
 * by one mebibyte in a {@code ByteBuffersDirectory}, whose inputs describe themselves with the
 * data input's {@code toString()} ({@code ByteBuffersDirectory.java:51-78}). Characters outside
 * printable ASCII are printed as {@code \\uXXXX} UTF-16 escapes.
 *
 * <p>{@code verifyAndCopyIndexHeader} ({@code CodecUtil.java:274-314}) reads, through the same
 * two inputs, a file too small for a header and a footer, a file of 32 bytes without the codec
 * magic ({@code nomagiclong.bin}), the index header against another id and against its own;
 * {@code readIndexHeader} ({@code CodecUtil.java:320-339}) reads the file without the magic, and
 * the index header through the plain input. A header whose codec name is not valid UTF-8 ({@code
 * badcodec.bin}) is read by {@code checkHeader}, whose {@code readString} decodes it leniently
 * ({@code DataInput.java:230-235}).
 *
 * <p>The footer checks read files that are too short for a footer ({@code tiny.bin}), a valid
 * footer ({@code footer.bin}) against a wrong expected length and from a misplaced position, and
 * footers whose magic ({@code badfooter.bin}), algorithm id ({@code badalgo.bin}), CRC bits
 * ({@code badcrc.bin}) or checksum ({@code badsum.bin}) is wrong: {@code readFooter} ({@code
 * CodecUtil.java:345-360}), {@code retrieveChecksum} ({@code CodecUtil.java:525-558}), {@code
 * validateFooter} ({@code CodecUtil.java:560-598}), {@code checksumEntireFile} ({@code
 * CodecUtil.java:606-621}), {@code readCRC} ({@code CodecUtil.java:629-635}) and {@code
 * checkFooter} ({@code CodecUtil.java:432-445}), each naming its input as the resource.
 *
 * <p>{@code tests/codec_header_exceptions.rs} writes the two files, has this tool read them, and
 * reads them with Rucene. {@code write <dir>} writes them with Lucene instead.
 *
 * <p>Command-line usage: {@code java -cp lucene-core-10.5.0.jar CodecHeaderFixture.java
 * [write|read] <dir>}.
 */
public final class CodecHeaderFixture {

  private CodecHeaderFixture() {}

  static final byte[] ID = new byte[16];

  /** Another id, and an id of the wrong length. */
  static final byte[] OTHER_ID = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16};

  static final byte[] SHORT_ID = new byte[15];

  /** Bytes that are not valid UTF-8: a stray byte, truncated sequences, an encoded surrogate. */
  static final byte[] BAD_SUFFIX = {
    0x61, (byte) 0xFF, (byte) 0xC3, 0x28, (byte) 0xE2, (byte) 0x82, 0x41, (byte) 0xF0,
    (byte) 0x9F, (byte) 0x98, (byte) 0x80, (byte) 0xED, (byte) 0xA0, (byte) 0x80
  };

  /** {@code s} with every character outside printable ASCII as a UTF-16 escape. */
  static String escaped(String s) {
    StringBuilder out = new StringBuilder();
    for (char c : s.toCharArray()) {
      if (c < 0x20 || c > 0x7E) {
        out.append(String.format("\\u%04X", (int) c));
      } else {
        out.append(c);
      }
    }
    return out.toString();
  }

  interface Check {
    void run(IndexInput in) throws Exception;
  }

  static void report(Directory dir, String file, boolean checksum, String label, Check check)
      throws Exception {
    try (IndexInput in =
        checksum ? dir.openChecksumInput(file) : dir.openInput(file, IOContext.DEFAULT)) {
      try {
        check.run(in);
        System.out.println(label + ": ok");
      } catch (IndexFormatTooOldException e) {
        System.out.println(label + ": IndexFormatTooOldException: " + e.getMessage());
        System.out.println(
            label
                + "  resource="
                + e.getResourceDescription()
                + " version="
                + e.getVersion()
                + " min="
                + e.getMinVersion()
                + " max="
                + e.getMaxVersion()
                + " reason="
                + e.getReason());
      } catch (IndexFormatTooNewException e) {
        System.out.println(label + ": IndexFormatTooNewException: " + e.getMessage());
        System.out.println(
            label
                + "  resource="
                + e.getResourceDescription()
                + " version="
                + e.getVersion()
                + " min="
                + e.getMinVersion()
                + " max="
                + e.getMaxVersion());
      } catch (CorruptIndexException e) {
        System.out.println(label + ": CorruptIndexException: " + escaped(e.getMessage()));
      }
    }
  }

  /** Writes or reads the two files in {@code args[1]}, as {@code args[0]} says. */
  public static void main(String[] args) throws Exception {
    try (Directory dir = new NIOFSDirectory(Paths.get(args[1]))) {
      if (args[0].equals("write")) {
        try (IndexOutput out = dir.createOutput("plain.bin", IOContext.DEFAULT)) {
          CodecUtil.writeHeader(out, "TestCodec", 5);
        }
        try (IndexOutput out = dir.createOutput("index.bin", IOContext.DEFAULT)) {
          CodecUtil.writeIndexHeader(out, "TestCodec", 5, ID, "sfx");
        }
        writeExtra(dir);
      } else {
        read(dir);
      }
    }
  }

  /**
   * The files beyond the two headers: no magic (4 and 32 bytes), an index header with a bad suffix,
   * and a header whose codec name is the bad suffix's bytes.
   */
  static void writeExtra(Directory dir) throws Exception {
    try (IndexOutput out = dir.createOutput("nomagic.bin", IOContext.DEFAULT)) {
      out.writeBytes(new byte[] {0x12, 0x34, 0x56, 0x78}, 4);
    }
    try (IndexOutput out = dir.createOutput("nomagiclong.bin", IOContext.DEFAULT)) {
      out.writeBytes(new byte[] {0x12, 0x34, 0x56, 0x78}, 4);
      out.writeBytes(new byte[28], 28);
    }
    try (IndexOutput out = dir.createOutput("badcodec.bin", IOContext.DEFAULT)) {
      CodecUtil.writeBEInt(out, CodecUtil.CODEC_MAGIC);
      out.writeVInt(BAD_SUFFIX.length);
      out.writeBytes(BAD_SUFFIX, BAD_SUFFIX.length);
      CodecUtil.writeBEInt(out, 5);
    }
    try (IndexOutput out = dir.createOutput("badsuffix.bin", IOContext.DEFAULT)) {
      CodecUtil.writeHeader(out, "TestCodec", 5);
      out.writeBytes(ID, ID.length);
      out.writeByte((byte) BAD_SUFFIX.length);
      out.writeBytes(BAD_SUFFIX, BAD_SUFFIX.length);
    }
    writeFooters(dir);
  }

  /** The files the footer checks read. */
  static void writeFooters(Directory dir) throws Exception {
    try (IndexOutput out = dir.createOutput("tiny.bin", IOContext.DEFAULT)) {
      out.writeBytes(new byte[] {1, 2, 3, 4}, 4);
    }
    try (IndexOutput out = dir.createOutput("footer.bin", IOContext.DEFAULT)) {
      CodecUtil.writeHeader(out, "TestCodec", 5);
      out.writeBytes(new byte[] {7, 7, 7}, 3);
      CodecUtil.writeFooter(out);
    }
    try (IndexOutput out = dir.createOutput("badfooter.bin", IOContext.DEFAULT)) {
      out.writeBytes(new byte[20], 20);
    }
    try (IndexOutput out = dir.createOutput("badalgo.bin", IOContext.DEFAULT)) {
      out.writeBytes(new byte[4], 4);
      CodecUtil.writeBEInt(out, CodecUtil.FOOTER_MAGIC);
      CodecUtil.writeBEInt(out, 1);
      CodecUtil.writeBELong(out, 0L);
    }
    try (IndexOutput out = dir.createOutput("badcrc.bin", IOContext.DEFAULT)) {
      out.writeBytes(new byte[4], 4);
      CodecUtil.writeBEInt(out, CodecUtil.FOOTER_MAGIC);
      CodecUtil.writeBEInt(out, 0);
      CodecUtil.writeBELong(out, 0x100000000L);
    }
    try (IndexOutput out = dir.createOutput("badsum.bin", IOContext.DEFAULT)) {
      out.writeBytes(new byte[4], 4);
      CodecUtil.writeBEInt(out, CodecUtil.FOOTER_MAGIC);
      CodecUtil.writeBEInt(out, 0);
      CodecUtil.writeBELong(out, 12345L);
    }
  }

  /**
   * The footer checks. A checksum input seeks only forwards and cannot be cloned, so every check
   * but {@code checkFooter} reads through the plain input.
   */
  static void readFooters(Directory dir) throws Exception {
    report(dir, "tiny.bin", false, "read footer short", in -> CodecUtil.readFooter(in));
    report(dir, "tiny.bin", false, "retrieve short", in -> CodecUtil.retrieveChecksum(in));
    report(dir, "tiny.bin", false, "entire short", in -> CodecUtil.checksumEntireFile(in));
    report(
        dir,
        "footer.bin",
        false,
        "expected longer",
        in -> CodecUtil.retrieveChecksum(in, in.length() + 1));
    report(
        dir,
        "footer.bin",
        false,
        "expected shorter",
        in -> CodecUtil.retrieveChecksum(in, in.length() - 1));
    report(dir, "badfooter.bin", false, "footer magic", in -> CodecUtil.retrieveChecksum(in));
    report(dir, "badalgo.bin", false, "footer algorithm", in -> CodecUtil.readFooter(in));
    report(dir, "badcrc.bin", false, "footer crc", in -> CodecUtil.retrieveChecksum(in));
    report(dir, "badsum.bin", false, "entire checksum", in -> CodecUtil.checksumEntireFile(in));
    report(dir, "footer.bin", true, "check footer extended", in -> CodecUtil.checkFooter((ChecksumIndexInput) in));
    report(
        dir,
        "footer.bin",
        true,
        "check footer truncated",
        in -> {
          in.seek(in.length() - 10);
          CodecUtil.checkFooter((ChecksumIndexInput) in);
        });
  }

  static void read(Directory dir) throws Exception {
    readExtra(dir);
    try (Directory memory = new ByteBuffersDirectory()) {
      try (IndexOutput out = memory.createOutput("big.bin", IOContext.DEFAULT)) {
        CodecUtil.writeHeader(out, "TestCodec", 5);
        out.writeBytes(new byte[1 << 20], 1 << 20);
      }
      for (boolean checksum : new boolean[] {false, true}) {
        String p = checksum ? "memory checksum " : "memory plain ";
        report(memory, "big.bin", checksum, p + "too-old", in -> CodecUtil.checkHeader(in, "TestCodec", 6, 10));
        report(memory, "big.bin", checksum, p + "codec", in -> CodecUtil.checkHeader(in, "OtherCodec", 1, 10));
      }
    }
    readFooters(dir);
    for (boolean checksum : new boolean[] {false, true}) {
      String p = checksum ? "checksum " : "plain ";
      report(dir, "plain.bin", checksum, p + "too-old", in -> CodecUtil.checkHeader(in, "TestCodec", 6, 10));
      report(dir, "plain.bin", checksum, p + "too-new", in -> CodecUtil.checkHeader(in, "TestCodec", 1, 4));
      report(dir, "plain.bin", checksum, p + "codec", in -> CodecUtil.checkHeader(in, "OtherCodec", 1, 10));
      report(dir, "plain.bin", checksum, p + "in-range", in -> CodecUtil.checkHeader(in, "TestCodec", 5, 5));
      report(
          dir,
          "index.bin",
          checksum,
          p + "index too-new",
          in -> CodecUtil.checkIndexHeader(in, "TestCodec", 1, 4, ID, "sfx"));
      report(
          dir,
          "index.bin",
          checksum,
          p + "index codec",
          in -> CodecUtil.checkIndexHeader(in, "OtherCodec", 1, 10, ID, "sfx"));
    }
  }

  static void readExtra(Directory dir) throws Exception {
    for (boolean checksum : new boolean[] {false, true}) {
      String p = checksum ? "checksum " : "plain ";
      report(dir, "nomagic.bin", checksum, p + "magic", in -> CodecUtil.checkHeader(in, "TestCodec", 1, 10));
      report(
          dir,
          "index.bin",
          checksum,
          p + "index id",
          in -> CodecUtil.checkIndexHeader(in, "TestCodec", 1, 10, OTHER_ID, "sfx"));
      report(
          dir,
          "index.bin",
          checksum,
          p + "index short id",
          in -> CodecUtil.checkIndexHeader(in, "TestCodec", 1, 10, SHORT_ID, "sfx"));
      report(
          dir,
          "index.bin",
          checksum,
          p + "index suffix",
          in -> CodecUtil.checkIndexHeader(in, "TestCodec", 1, 10, ID, "xfs"));
      report(
          dir,
          "badsuffix.bin",
          checksum,
          p + "index bad suffix",
          in -> CodecUtil.checkIndexHeader(in, "TestCodec", 1, 10, ID, "sfx"));
      report(
          dir,
          "nomagic.bin",
          checksum,
          p + "copy small",
          in -> CodecUtil.verifyAndCopyIndexHeader(in, new ByteBuffersDataOutput(), ID));
      report(
          dir,
          "nomagiclong.bin",
          checksum,
          p + "copy magic",
          in -> CodecUtil.verifyAndCopyIndexHeader(in, new ByteBuffersDataOutput(), ID));
      report(
          dir,
          "index.bin",
          checksum,
          p + "copy id",
          in -> CodecUtil.verifyAndCopyIndexHeader(in, new ByteBuffersDataOutput(), OTHER_ID));
      report(
          dir,
          "index.bin",
          checksum,
          p + "copy ok",
          in -> CodecUtil.verifyAndCopyIndexHeader(in, new ByteBuffersDataOutput(), ID));
      report(dir, "nomagic.bin", checksum, p + "read header magic", CodecUtil::readIndexHeader);
      report(dir, "badcodec.bin", checksum, p + "bad codec", in -> CodecUtil.checkHeader(in, "TestCodec", 1, 10));
    }
    // A checksum input cannot seek back to the start once the header was read
    // (ChecksumIndexInput.java:57-65).
    report(dir, "index.bin", false, "plain read header", CodecUtil::readIndexHeader);
  }
}
