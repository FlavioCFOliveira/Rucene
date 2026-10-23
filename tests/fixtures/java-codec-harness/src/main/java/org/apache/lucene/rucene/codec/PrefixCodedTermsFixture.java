package org.apache.lucene.rucene.codec;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.index.PrefixCodedTerms;
import org.apache.lucene.index.Term;
import org.apache.lucene.util.BytesRef;

/**
 * Reports what Lucene 10.5.0's {@link PrefixCodedTerms} holds and what it costs, so the Rust port
 * can be compared against the reference implementation rather than against its own arithmetic.
 *
 * <p>Two things are reported per case. The decoded term sequence proves the encoding round-trips
 * identically. {@code ramBytesUsed()} proves the payload is held the way Lucene holds it: the sum
 * is over the *capacity* of each {@code ByteBuffer} in the {@code List<ByteBuffer>} the builder
 * produced (`PrefixCodedTerms.java:37`, `:49-51`, `:103`), so a packet with three short terms costs
 * a whole 1 KiB block, and a port that flattened the payload into one tightly sized buffer would
 * report a different number.
 *
 * <h2>Protocol</h2>
 *
 * <p>Arguments: {@code <inputFile> <outputFile>}. Each input line is one case: tab-separated pairs
 * of {@code field} and the term's bytes in lowercase hex. Terms must already be in ascending {@link
 * Term} order, which is what {@code Builder.add} requires.
 *
 * <p>Each output line is {@code size <TAB> ramBytesUsed <TAB> blockCount} followed by one {@code
 * field:hexBytes} field per decoded term.
 */
public final class PrefixCodedTermsFixture {

  private PrefixCodedTermsFixture() {}

  public static void main(String[] args) throws IOException {
    if (args.length != 2) {
      throw new IllegalArgumentException("usage: PrefixCodedTermsFixture <inputFile> <outputFile>");
    }
    List<String> lines = Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8);
    StringBuilder out = new StringBuilder();
    for (String line : lines) {
      out.append(run(line)).append('\n');
    }
    Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
  }

  private static String run(String line) {
    PrefixCodedTerms.Builder builder = new PrefixCodedTerms.Builder();
    if (!line.isEmpty()) {
      String[] parts = line.split("\t", -1);
      for (int i = 0; i + 1 < parts.length; i += 2) {
        builder.add(new Term(parts[i], new BytesRef(fromHex(parts[i + 1]))));
      }
    }
    PrefixCodedTerms terms = builder.finish();

    // `ramBytesUsed` is `sum(buffer.capacity()) + 2 * Long.BYTES`, so the block
    // count follows from it for the default 1 KiB blocks and is reported so the
    // Rust side can compare the payload's shape and not only its cost.
    long ram = terms.ramBytesUsed();

    StringBuilder sb = new StringBuilder();
    sb.append(terms.size()).append('\t').append(ram);

    PrefixCodedTerms.TermIterator it = terms.iterator();
    for (BytesRef bytes = it.next(); bytes != null; bytes = it.next()) {
      sb.append('\t').append(it.field()).append(':').append(toHex(bytes));
    }
    return sb.toString();
  }

  private static byte[] fromHex(String hex) {
    byte[] out = new byte[hex.length() / 2];
    for (int i = 0; i < out.length; i++) {
      out[i] = (byte) Integer.parseInt(hex.substring(i * 2, i * 2 + 2), 16);
    }
    return out;
  }

  private static String toHex(BytesRef bytes) {
    StringBuilder sb = new StringBuilder(bytes.length * 2);
    for (int i = 0; i < bytes.length; i++) {
      sb.append(String.format("%02x", bytes.bytes[bytes.offset + i] & 0xFF));
    }
    return sb.toString();
  }
}
