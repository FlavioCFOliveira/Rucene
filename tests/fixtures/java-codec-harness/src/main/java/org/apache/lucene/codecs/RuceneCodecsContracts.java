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
package org.apache.lucene.codecs;

import java.util.function.BiConsumer;

/**
 * Instantiates {@code BlockTermState}, whose constructor is protected, so that
 * its {@code toString()} can be measured. The subclass adds nothing and does not
 * override {@code toString()}.
 */
public final class RuceneCodecsContracts {
  private RuceneCodecsContracts() {}

  private static final class ProbeTermState extends BlockTermState {}

  public static void emit(BiConsumer<String, String> out) {
    ProbeTermState fresh = new ProbeTermState();
    out.accept("BlockTermState.toString/fresh", fresh.toString());

    ProbeTermState filled = new ProbeTermState();
    filled.ord = 42;
    filled.docFreq = 7;
    filled.totalTermFreq = 19;
    filled.termBlockOrd = 3;
    filled.blockFilePointer = 1234;
    out.accept("BlockTermState.toString/filled", filled.toString());
  }
}
