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
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.BufferedWriter;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.LineNumberReader;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.nio.charset.CharsetDecoder;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.Base64;
import java.util.List;
import java.util.Random;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;
import java.util.zip.ZipOutputStream;

/**
 * The JDK side of the oracle of rucene-analysis-common's JDK I/O emulations (rmp #453):
 * {@code jdk/io.rs} (UTF-8 {@code InputStreamReader}, {@code BufferedReader}, {@code
 * LineNumberReader}), {@code jdk/base64.rs} ({@code Base64.getDecoder()}) and {@code
 * jdk/zip_input_stream.rs} ({@code ZipInputStream}). Inputs and outputs are hex.
 *
 * <ul>
 *   <li>{@code utf8 <file>}: lines {@code <R|P> <bytes>}: decoded with REPLACE or REPORT through
 *       {@code BufferedReader.readLine} and {@code LineNumberReader.readLine}; prints the lines
 *       (UTF-16 hex, joined by '/') and the final line number, or the failure.
 *   <li>{@code b64 <file>}: lines {@code <B|S> <hex>}: {@code decode(byte[])} of the bytes, or
 *       {@code decode(String)} of the UTF-16 units.
 *   <li>{@code zipgen <seed> <count> <out>}: writes ZIP streams (stored and deflated entries,
 *       directories, empty entries, UTF-8 names, then corrupted and truncated copies), one hex
 *       line each.
 *   <li>{@code zip <file>}: reads each ZIP stream; prints each entry's name and content.
 * </ul>
 *
 * A failure prints {@code E<binary class name>:<message>}.
 */
public class JdkIoOracle {
  static byte[] unhex(String s) {
    if (s.equals("-")) return new byte[0];
    byte[] b = new byte[s.length() / 2];
    for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
    return b;
  }

  static String hex(byte[] b) {
    if (b.length == 0) return "-";
    StringBuilder s = new StringBuilder();
    for (byte x : b) s.append(String.format("%02x", x & 0xFF));
    return s.toString();
  }

  static String hexUnits(String s) {
    StringBuilder b = new StringBuilder();
    for (int i = 0; i < s.length(); i++) b.append(String.format("%04x", (int) s.charAt(i)));
    return b.toString();
  }

  static String failure(Throwable e) {
    return "E" + e.getClass().getName() + ":" + e.getMessage();
  }

  static CharsetDecoder decoder(String action) {
    CodingErrorAction a = action.equals("R") ? CodingErrorAction.REPLACE : CodingErrorAction.REPORT;
    return StandardCharsets.UTF_8.newDecoder().onMalformedInput(a).onUnmappableCharacter(a);
  }

  static String utf8(String action, byte[] bytes) {
    StringBuilder out = new StringBuilder();
    try (BufferedReader r =
        new BufferedReader(new InputStreamReader(new ByteArrayInputStream(bytes), decoder(action)))) {
      StringBuilder lines = new StringBuilder("B");
      try {
        for (String line = r.readLine(); line != null; line = r.readLine()) {
          lines.append('/').append(hexUnits(line));
        }
      } catch (Exception e) {
        lines.append('/').append(failure(e));
      }
      out.append(lines);
    } catch (Exception e) {
      out.append(failure(e));
    }
    out.append(' ');
    try (LineNumberReader r =
        new LineNumberReader(
            new InputStreamReader(new ByteArrayInputStream(bytes), decoder(action)))) {
      StringBuilder lines = new StringBuilder("L");
      try {
        for (String line = r.readLine(); line != null; line = r.readLine()) {
          lines.append('/').append(r.getLineNumber()).append('=').append(hexUnits(line));
        }
        lines.append("/n=").append(r.getLineNumber());
      } catch (Exception e) {
        lines.append('/').append(failure(e));
      }
      out.append(lines);
    } catch (Exception e) {
      out.append(failure(e));
    }
    return out.toString();
  }

  static String b64(String kind, String arg) {
    try {
      if (kind.equals("B")) return hex(Base64.getDecoder().decode(unhex(arg)));
      StringBuilder s = new StringBuilder();
      if (arg.equals("-")) arg = "";
      for (int i = 0; i < arg.length(); i += 4) s.append((char) Integer.parseInt(arg.substring(i, i + 4), 16));
      return hex(Base64.getDecoder().decode(s.toString()));
    } catch (Throwable e) {
      return failure(e);
    }
  }

  static String zip(byte[] bytes) {
    StringBuilder out = new StringBuilder("Z");
    try (ZipInputStream in = new ZipInputStream(new ByteArrayInputStream(bytes))) {
      try {
        for (ZipEntry e = in.getNextEntry(); e != null; e = in.getNextEntry()) {
          out.append('/').append(hexUnits(e.getName())).append('=');
          out.append(hex(readAll(in)));
          out.append(";a=").append(in.available());
        }
        out.append("/a=").append(in.available());
      } catch (Throwable e) {
        out.append('/').append(failure(e));
      }
    } catch (Throwable e) {
      out.append('/').append(failure(e));
    }
    return out.toString();
  }

  static byte[] readAll(InputStream in) throws Exception {
    ByteArrayOutputStream b = new ByteArrayOutputStream();
    byte[] buf = new byte[1 + 37];
    for (int n = in.read(buf); n != -1; n = in.read(buf)) b.write(buf, 0, n);
    return b.toByteArray();
  }

  static void zipgen(long seed, int count, String outFile) throws Exception {
    Random r = new Random(seed);
    StringBuilder all = new StringBuilder();
    String[] names = {"a.txt", "dir/", "dir/b.bin", "syn0.txt", "config.json", "café.txt", "中.txt", "e"};
    for (int n = 0; n < count; n++) {
      ByteArrayOutputStream bo = new ByteArrayOutputStream();
      try (ZipOutputStream z = new ZipOutputStream(bo)) {
        z.setLevel(r.nextInt(10));
        int entries = r.nextInt(5);
        for (int i = 0; i < entries; i++) {
          String name = names[r.nextInt(names.length)] + (i == 0 ? "" : Integer.toString(i));
          ZipEntry e = new ZipEntry(name);
          byte[] data = new byte[name.endsWith("/") ? 0 : r.nextInt(3) == 0 ? 0 : r.nextInt(5000)];
          int mode = r.nextInt(3);
          for (int j = 0; j < data.length; j++) data[j] = (byte) (mode == 0 ? r.nextInt(256) : 'a' + r.nextInt(mode == 1 ? 3 : 26));
          if (r.nextBoolean()) {
            e.setMethod(ZipEntry.STORED);
            java.util.zip.CRC32 crc = new java.util.zip.CRC32();
            crc.update(data);
            e.setSize(data.length);
            e.setCompressedSize(data.length);
            e.setCrc(crc.getValue());
          }
          z.putNextEntry(e);
          z.write(data);
          z.closeEntry();
        }
      }
      byte[] zip = bo.toByteArray();
      all.append(hex(zip)).append('\n');
      // a corrupted copy and a truncated copy
      if (zip.length > 0) {
        byte[] c = zip.clone();
        for (int k = 0; k < 1 + r.nextInt(3); k++) c[r.nextInt(c.length)] ^= (byte) (1 + r.nextInt(255));
        all.append(hex(c)).append('\n');
        byte[] t = java.util.Arrays.copyOf(zip, r.nextInt(zip.length));
        all.append(hex(t)).append('\n');
      }
    }
    Files.writeString(Paths.get(outFile), all.toString());
  }

  public static void main(String[] args) throws Exception {
    if (args[0].equals("zipgen")) {
      zipgen(Long.parseLong(args[1]), Integer.parseInt(args[2]), args[3]);
      return;
    }
    PrintWriter out =
        new PrintWriter(
            new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8), 1 << 16));
    List<String> lines = Files.readAllLines(Paths.get(args[1]), StandardCharsets.UTF_8);
    for (String line : lines) {
      if (line.isEmpty()) continue;
      switch (args[0]) {
        case "utf8":
          {
            String[] f = line.split(" ");
            out.println(utf8(f[0], unhex(f[1])));
            break;
          }
        case "b64":
          {
            String[] f = line.split(" ");
            out.println(b64(f[0], f[1]));
            break;
          }
        case "zip":
          out.println(zip(unhex(line)));
          break;
        default:
          throw new IllegalArgumentException(args[0]);
      }
    }
    out.flush();
  }
}
