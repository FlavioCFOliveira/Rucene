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
package org.apache.lucene.tests.util;

import com.carrotsearch.randomizedtesting.annotations.TestGroup;
import java.io.File;
import java.io.InputStream;
import java.io.PrintWriter;
import java.lang.annotation.Annotation;
import java.lang.annotation.Documented;
import java.lang.annotation.Inherited;
import java.lang.annotation.Retention;
import java.lang.annotation.Target;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;

/**
 * Oracle of {@code crates/rucene-test-framework/tests/test_framework_types_oracle.rs} (rmp #445):
 * the observable behaviour of {@code LuceneTestCase.Nightly}, {@code
 * LuceneTestCase.SuppressSysoutChecks} and {@code StringMockResourceLoader} on Lucene 10.5.0 and
 * JDK 25. ({@code English} has its own oracle, {@code TestUtilOracle}.)
 *
 * <p>Writes {@code <out>/rucene-test-framework/test_framework_types.tsv}: {@code key<TAB>value}
 * rows. Strings are written as hex UTF-16 code units joined by {@code .} ({@code -} when empty);
 * byte arrays as hex bytes joined by {@code .} ({@code -} when empty).
 */
public final class TestFrameworkTypesOracle {

  private TestFrameworkTypesOracle() {}

  @LuceneTestCase.Nightly
  static final class NightlySuite {}

  /** Inherits {@code @Nightly} ({@code @Inherited}). */
  static final class NightlyChild extends NightlyBase {}

  @LuceneTestCase.Nightly
  static class NightlyBase {}

  @LuceneTestCase.SuppressSysoutChecks(bugUrl = "https://issues.apache.org/jira/browse/LUCENE-1")
  static class SysoutSuite {}

  @LuceneTestCase.SuppressSysoutChecks(bugUrl = "a\"b\\cé\n\ttab")
  static final class SysoutEscapes {}

  static final class SysoutChild extends SysoutSuite {}

  /** The bug URLs of {@link SysoutSuite} and {@link SysoutEscapes}, in order. */
  static final String[] BUG_URLS = {
    "https://issues.apache.org/jira/browse/LUCENE-1", "a\"b\\cé\n\ttab"
  };

  /** The texts given to {@code StringMockResourceLoader}, in order. */
  static final String[] TEXTS = {
    "", "a", "stopwords\nthe\nof\n", "café ß 中文 😀", "\u0000\u007f\u0080"
  };

  static String units(String s) {
    if (s.isEmpty()) {
      return "-";
    }
    StringBuilder b = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      if (i > 0) {
        b.append('.');
      }
      b.append(String.format("%04x", (int) s.charAt(i)));
    }
    return b.toString();
  }

  static String bytes(byte[] bytes) {
    if (bytes.length == 0) {
      return "-";
    }
    StringBuilder b = new StringBuilder();
    for (int i = 0; i < bytes.length; i++) {
      if (i > 0) {
        b.append('.');
      }
      b.append(String.format("%02x", bytes[i] & 0xff));
    }
    return b.toString();
  }

  static void type(PrintWriter out, String key, Class<? extends Annotation> type) {
    out.println(key + ".name\t" + units(type.getName()));
    out.println(key + ".simpleName\t" + units(type.getSimpleName()));
    out.println(key + ".inherited\t" + type.isAnnotationPresent(Inherited.class));
    out.println(key + ".documented\t" + type.isAnnotationPresent(Documented.class));
    out.println(key + ".retention\t" + type.getAnnotation(Retention.class).value());
    Target target = type.getAnnotation(Target.class);
    out.println(key + ".target\t" + (target == null ? "null" : Arrays.toString(target.value())));
    out.println(key + ".members\t" + type.getDeclaredMethods().length);
    TestGroup group = type.getAnnotation(TestGroup.class);
    out.println(key + ".testGroup\t" + (group != null));
    if (group != null) {
      out.println(key + ".testGroup.name\t" + units(group.name()));
      out.println(key + ".testGroup.sysProperty\t" + units(group.sysProperty()));
      out.println(key + ".testGroup.enabled\t" + group.enabled());
      out.println(key + ".groupName\t" + units(TestGroup.Utilities.getGroupName(type)));
      out.println(key + ".groupSysProperty\t" + units(TestGroup.Utilities.getSysProperty(type)));
    }
  }

  public static void main(String[] args) throws Exception {
    File dir = new File(args[0], "rucene-test-framework");
    dir.mkdirs();
    try (PrintWriter out =
        new PrintWriter(new File(dir, "test_framework_types.tsv"), StandardCharsets.UTF_8)) {
      out.println("# TestFrameworkTypesOracle: LuceneTestCase.Nightly, SuppressSysoutChecks,");
      out.println("# StringMockResourceLoader (Lucene 10.5.0, JDK 25). key<TAB>value.");
      type(out, "nightly", LuceneTestCase.Nightly.class);
      out.println(
          "nightly.toString\t"
              + units(NightlySuite.class.getAnnotation(LuceneTestCase.Nightly.class).toString()));
      out.println(
          "nightly.inheritedBySubclass\t"
              + NightlyChild.class.isAnnotationPresent(LuceneTestCase.Nightly.class));
      type(out, "sysout", LuceneTestCase.SuppressSysoutChecks.class);
      Class<?>[] sysoutSuites = {SysoutSuite.class, SysoutEscapes.class};
      for (int i = 0; i < sysoutSuites.length; i++) {
        LuceneTestCase.SuppressSysoutChecks a =
            sysoutSuites[i].getAnnotation(LuceneTestCase.SuppressSysoutChecks.class);
        out.println("sysout.bugUrl." + i + "\t" + units(a.bugUrl()));
        out.println("sysout.toString." + i + "\t" + units(a.toString()));
      }
      out.println(
          "sysout.inheritedBySubclass\t"
              + SysoutChild.class.isAnnotationPresent(LuceneTestCase.SuppressSysoutChecks.class));
      for (int i = 0; i < TEXTS.length; i++) {
        StringMockResourceLoader loader = new StringMockResourceLoader(TEXTS[i]);
        out.println("smrl.text." + i + "\t" + units(TEXTS[i]));
        // Every resource name opens the same bytes; a fresh stream per call.
        for (String name : new String[] {"stopwords.txt", "", "/absolute/path.txt"}) {
          try (InputStream in = loader.openResource(name)) {
            out.println("smrl.open." + i + "." + units(name) + "\t" + bytes(in.readAllBytes()));
          }
        }
      }
    }
  }
}
