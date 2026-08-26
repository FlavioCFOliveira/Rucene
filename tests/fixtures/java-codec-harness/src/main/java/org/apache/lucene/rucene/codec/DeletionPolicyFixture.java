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

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.AbstractMap;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.TreeSet;

import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.Field;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexCommit;
import org.apache.lucene.index.IndexDeletionPolicy;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.IndexWriterConfig.OpenMode;
import org.apache.lucene.index.KeepLastNCommitsDeletionPolicy;
import org.apache.lucene.index.KeepOnlyLastCommitDeletionPolicy;
import org.apache.lucene.index.NoDeletionPolicy;
import org.apache.lucene.index.PersistentSnapshotDeletionPolicy;
import org.apache.lucene.index.SnapshotDeletionPolicy;
import org.apache.lucene.index.TwoPhaseCommit;
import org.apache.lucene.index.TwoPhaseCommitTool;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FilterDirectory;
import org.apache.lucene.store.FilterIndexOutput;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;

/**
 * Reference fixture for Rucene's {@code IndexCommit} / {@code IndexDeletionPolicy} port.
 *
 * <p>Every shape prints {@code key=value} lines on stdout so that the Rust side can compare
 * behaviour without parsing Lucene internals, and leaves its artefacts on disk so that the
 * generated files can be compared byte-for-byte.
 *
 * <p>Shapes:
 *
 * <ul>
 *   <li>{@code keep-only-last} / {@code keep-last-2} / {@code keep-last-10} / {@code no-deletion} —
 *       runs five commits under the named policy and reports which {@code segments_N} generations
 *       survive in the index directory.
 *   <li>{@code snapshot} — runs {@link SnapshotDeletionPolicy} over five commits, snapshotting two
 *       of them, and reports which generations survive and which are held.
 *   <li>{@code persistent-snapshot} — same, but with {@link PersistentSnapshotDeletionPolicy}
 *       persisting into {@code <outDir>/snapshots}; the resulting {@code snapshots_N} file is the
 *       byte-level reference for the Rust port.
 *   <li>{@code read-snapshots} — loads an existing {@code snapshots_N} file (written by Rucene)
 *       from {@code <outDir>} with Lucene's own reader and reports what it decoded.
 *   <li>{@code reopen-snapshot} — persists a snapshot, closes the writer, reopens the index with a
 *       fresh {@link PersistentSnapshotDeletionPolicy} and reports that the pin survived the
 *       round-trip. This is the only shape whose {@code onInit} receives a non-empty commit list.
 *   <li>{@code release} — snapshots two commits, releases one of them and reports which
 *       generations survive the next commit.
 *   <li>{@code list-commits} — builds an index with every commit retained and reports, for each
 *       commit, its generation, {@code segments_N} name, segment count, user data and full file
 *       list, so the Rust side can compare {@code DirectoryReader.listCommits} output directly.
 *   <li>{@code two-phase-commit} — runs {@link TwoPhaseCommitTool#execute} over recording objects
 *       and reports the exact call trace, plus the exception type and message, for the success,
 *       prepare-failure and commit-failure cases.
 *   <li>{@code persist-close-failure} — makes the {@code snapshots_N} save file fail to close and
 *       reports what Lucene leaves behind and what the next {@code snapshot()} does.
 * </ul>
 *
 * <p>Usage: {@code DeletionPolicyFixture <outDir> <shape>}
 */
public final class DeletionPolicyFixture {

    /** Number of commits every index-building shape performs. */
    private static final int COMMITS = 5;

    private DeletionPolicyFixture() {}

    /** Exposes the protected {@code refCounts} map of the persistent policy. */
    private static final class SnapshotProbe extends PersistentSnapshotDeletionPolicy {
        SnapshotProbe(IndexDeletionPolicy primary, Directory dir, OpenMode mode) throws IOException {
            super(primary, dir, mode);
        }

        Map<Long, Integer> counts() {
            return new TreeMap<>(refCounts);
        }
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            System.err.println("usage: DeletionPolicyFixture <outDir> <shape>");
            System.exit(2);
        }
        Path outDir = Paths.get(args[0]).toAbsolutePath();
        String shape = args[1];
        Files.createDirectories(outDir);

        System.out.println("shape=" + shape);
        System.out.println("version=" + org.apache.lucene.util.Version.LATEST);
        System.out.println("output_dir=" + outDir);

        switch (shape) {
            case "keep-only-last" ->
                    runPolicy(outDir, shape, new KeepOnlyLastCommitDeletionPolicy());
            case "keep-last-2" ->
                    runPolicy(outDir, shape, new KeepLastNCommitsDeletionPolicy(2));
            case "keep-last-10" ->
                    runPolicy(outDir, shape, new KeepLastNCommitsDeletionPolicy(10));
            case "no-deletion" -> runPolicy(outDir, shape, NoDeletionPolicy.INSTANCE);
            case "snapshot" -> runSnapshot(outDir);
            case "persistent-snapshot" -> runPersistentSnapshot(outDir);
            case "read-snapshots" -> readSnapshots(outDir);
            case "reopen-snapshot" -> runReopenSnapshot(outDir);
            case "release" -> runRelease(outDir);
            case "list-commits" -> runListCommits(outDir);
            case "two-phase-commit" -> runTwoPhaseCommit();
            case "persist-close-failure" -> runPersistCloseFailure(outDir);
            default -> {
                System.err.println("unknown shape: " + shape);
                System.exit(2);
            }
        }
    }

    /** Runs {@link #COMMITS} commits under {@code policy} and reports the surviving generations. */
    private static void runPolicy(Path outDir, String shape, IndexDeletionPolicy policy)
            throws IOException {
        Path indexDir = outDir.resolve("index");
        deleteRecursively(indexDir);
        Files.createDirectories(indexDir);

        List<Long> created = new ArrayList<>();
        try (Directory dir = FSDirectory.open(indexDir)) {
            IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
            config.setOpenMode(OpenMode.CREATE);
            config.setIndexDeletionPolicy(policy);
            try (IndexWriter writer = new IndexWriter(dir, config)) {
                for (int i = 0; i < COMMITS; i++) {
                    writer.addDocument(doc(i));
                    writer.commit();
                    created.add(lastGeneration(dir));
                }
            }
            System.out.println("policy=" + policy.getClass().getSimpleName());
            System.out.println("created_generations=" + join(created));
            System.out.println("surviving_generations=" + join(commitGenerations(dir)));
        }
    }

    /** Runs the in-memory snapshot policy and reports which generations it pinned. */
    private static void runSnapshot(Path outDir) throws IOException {
        Path indexDir = outDir.resolve("index");
        deleteRecursively(indexDir);
        Files.createDirectories(indexDir);

        SnapshotDeletionPolicy policy =
                new SnapshotDeletionPolicy(new KeepOnlyLastCommitDeletionPolicy());
        List<Long> created = new ArrayList<>();
        List<Long> snapshotted = new ArrayList<>();

        try (Directory dir = FSDirectory.open(indexDir)) {
            IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
            config.setOpenMode(OpenMode.CREATE);
            config.setIndexDeletionPolicy(policy);
            try (IndexWriter writer = new IndexWriter(dir, config)) {
                SnapshotDeletionPolicy live =
                        (SnapshotDeletionPolicy) writer.getConfig().getIndexDeletionPolicy();
                for (int i = 0; i < COMMITS; i++) {
                    writer.addDocument(doc(i));
                    writer.commit();
                    created.add(lastGeneration(dir));
                    // Snapshot the 2nd and the 4th commit.
                    if (i == 1 || i == 3) {
                        snapshotted.add(live.snapshot().getGeneration());
                    }
                }
                System.out.println("snapshot_count=" + live.getSnapshotCount());
            }
            System.out.println("created_generations=" + join(created));
            System.out.println("snapshotted_generations=" + join(snapshotted));
            System.out.println("surviving_generations=" + join(commitGenerations(dir)));
        }
    }

    /** Runs the persistent snapshot policy, leaving a reference {@code snapshots_N} on disk. */
    private static void runPersistentSnapshot(Path outDir) throws IOException {
        Path indexDir = outDir.resolve("index");
        Path snapshotsDir = outDir.resolve("snapshots");
        deleteRecursively(indexDir);
        deleteRecursively(snapshotsDir);
        Files.createDirectories(indexDir);
        Files.createDirectories(snapshotsDir);

        try (Directory dir = FSDirectory.open(indexDir);
                Directory snapDir = FSDirectory.open(snapshotsDir)) {
            SnapshotProbe policy =
                    new SnapshotProbe(
                            new KeepOnlyLastCommitDeletionPolicy(), snapDir, OpenMode.CREATE);
            IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
            config.setOpenMode(OpenMode.CREATE);
            config.setIndexDeletionPolicy(policy);

            List<Long> created = new ArrayList<>();
            List<Long> snapshotted = new ArrayList<>();
            try (IndexWriter writer = new IndexWriter(dir, config)) {
                SnapshotProbe live =
                        (SnapshotProbe) writer.getConfig().getIndexDeletionPolicy();
                for (int i = 0; i < COMMITS; i++) {
                    writer.addDocument(doc(i));
                    writer.commit();
                    created.add(lastGeneration(dir));
                    if (i == 1 || i == 3) {
                        snapshotted.add(live.snapshot().getGeneration());
                    }
                    // Snapshot the 4th commit twice so a reference count > 1 is persisted.
                    if (i == 3) {
                        snapshotted.add(live.snapshot().getGeneration());
                    }
                }
                System.out.println("snapshot_count=" + live.getSnapshotCount());
                System.out.println("last_save_file=" + live.getLastSaveFile());
                System.out.println("refcounts=" + joinCounts(live.counts()));
            }
            System.out.println("created_generations=" + join(created));
            System.out.println("snapshotted_generations=" + join(snapshotted));
            System.out.println("surviving_generations=" + join(commitGenerations(dir)));
        }
    }

    /** Loads a {@code snapshots_N} file written elsewhere and reports what Lucene decoded. */
    private static void readSnapshots(Path outDir) throws IOException {
        try (Directory snapDir = FSDirectory.open(outDir)) {
            SnapshotProbe probe =
                    new SnapshotProbe(
                            new KeepOnlyLastCommitDeletionPolicy(), snapDir, OpenMode.APPEND);
            System.out.println("last_save_file=" + probe.getLastSaveFile());
            System.out.println("snapshot_count=" + probe.getSnapshotCount());
            System.out.println("refcounts=" + joinCounts(probe.counts()));
        }
    }

    /** Exposes the protected {@code refCounts} map of the in-memory policy. */
    private static final class MemorySnapshotProbe extends SnapshotDeletionPolicy {
        MemorySnapshotProbe(IndexDeletionPolicy primary) {
            super(primary);
        }

        Map<Long, Integer> counts() {
            return new TreeMap<>(refCounts);
        }
    }

    /**
     * Persists a snapshot, closes the writer, then reopens the index with a brand-new policy so
     * that {@code onInit} runs over a non-empty commit list and re-attaches the pinned commit.
     */
    private static void runReopenSnapshot(Path outDir) throws IOException {
        Path indexDir = outDir.resolve("index");
        Path snapshotsDir = outDir.resolve("snapshots");
        deleteRecursively(indexDir);
        deleteRecursively(snapshotsDir);
        Files.createDirectories(indexDir);
        Files.createDirectories(snapshotsDir);

        List<Long> phase1Created = new ArrayList<>();
        long pinned;

        try (Directory dir = FSDirectory.open(indexDir);
                Directory snapDir = FSDirectory.open(snapshotsDir)) {
            // --- Phase 1: create the index and pin its second commit. --------
            SnapshotProbe policy =
                    new SnapshotProbe(
                            new KeepOnlyLastCommitDeletionPolicy(), snapDir, OpenMode.CREATE);
            IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
            config.setOpenMode(OpenMode.CREATE);
            config.setIndexDeletionPolicy(policy);
            try (IndexWriter writer = new IndexWriter(dir, config)) {
                SnapshotProbe live = (SnapshotProbe) writer.getConfig().getIndexDeletionPolicy();
                for (int i = 0; i < 3; i++) {
                    writer.addDocument(doc(i));
                    writer.commit();
                    phase1Created.add(lastGeneration(dir));
                }
                pinned = live.snapshot().getGeneration();
                System.out.println("phase1_snapshotted=" + pinned);
                System.out.println("phase1_last_save_file=" + live.getLastSaveFile());
                System.out.println("phase1_refcounts=" + joinCounts(live.counts()));
            }
            System.out.println("phase1_created_generations=" + join(phase1Created));
            System.out.println("phase1_surviving_generations=" + join(commitGenerations(dir)));

            // --- Phase 2: reopen with a fresh policy over the same save dir. --
            SnapshotProbe reopened =
                    new SnapshotProbe(
                            new KeepOnlyLastCommitDeletionPolicy(), snapDir, OpenMode.APPEND);
            // Before onInit the reference counts are loaded but no commit is attached yet.
            System.out.println("phase2_loaded_refcounts=" + joinCounts(reopened.counts()));
            System.out.println("phase2_loaded_count=" + reopened.getSnapshotCount());
            System.out.println(
                    "phase2_snapshots_before_oninit=" + join(snapshotGenerations(reopened)));

            IndexWriterConfig reopenConfig = new IndexWriterConfig(new WhitespaceAnalyzer());
            reopenConfig.setOpenMode(OpenMode.APPEND);
            reopenConfig.setIndexDeletionPolicy(reopened);
            List<Long> phase2Created = new ArrayList<>();
            try (IndexWriter writer = new IndexWriter(dir, reopenConfig)) {
                SnapshotProbe live = (SnapshotProbe) writer.getConfig().getIndexDeletionPolicy();
                // The writer has just called onInit over the surviving commits.
                System.out.println(
                        "phase2_snapshots_after_oninit=" + join(snapshotGenerations(live)));
                System.out.println(
                        "phase2_index_commit_"
                                + pinned
                                + "_attached="
                                + (live.getIndexCommit(pinned) != null));
                for (int i = 0; i < 2; i++) {
                    writer.addDocument(doc(10 + i));
                    writer.commit();
                    phase2Created.add(lastGeneration(dir));
                }
                System.out.println("phase2_count=" + live.getSnapshotCount());
            }
            System.out.println("phase2_created_generations=" + join(phase2Created));
            System.out.println("phase2_surviving_generations=" + join(commitGenerations(dir)));
        }
    }

    /** Snapshots two commits, releases one, and reports what survives afterwards. */
    private static void runRelease(Path outDir) throws IOException {
        Path indexDir = outDir.resolve("index");
        deleteRecursively(indexDir);
        Files.createDirectories(indexDir);

        MemorySnapshotProbe policy =
                new MemorySnapshotProbe(new KeepOnlyLastCommitDeletionPolicy());
        List<Long> created = new ArrayList<>();
        List<Long> snapshotted = new ArrayList<>();

        try (Directory dir = FSDirectory.open(indexDir)) {
            IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
            config.setOpenMode(OpenMode.CREATE);
            config.setIndexDeletionPolicy(policy);
            try (IndexWriter writer = new IndexWriter(dir, config)) {
                MemorySnapshotProbe live =
                        (MemorySnapshotProbe) writer.getConfig().getIndexDeletionPolicy();
                for (int i = 0; i < COMMITS; i++) {
                    writer.addDocument(doc(i));
                    writer.commit();
                    created.add(lastGeneration(dir));
                    if (i == 1 || i == 3) {
                        snapshotted.add(live.snapshot().getGeneration());
                    }
                }
                System.out.println("count_before_release=" + live.getSnapshotCount());
                System.out.println("refcounts_before_release=" + joinCounts(live.counts()));
                System.out.println(
                        "surviving_before_release=" + join(commitGenerations(dir)));

                // Release the older of the two pins; its files stay until the
                // next checkpoint asks the policy again.
                long released = snapshotted.get(0);
                live.release(live.getIndexCommit(released));
                System.out.println("released=" + released);
                System.out.println("count_after_release=" + live.getSnapshotCount());
                System.out.println("refcounts_after_release=" + joinCounts(live.counts()));
                System.out.println(
                        "snapshots_after_release=" + join(snapshotGenerations(live)));

                writer.addDocument(doc(COMMITS));
                writer.commit();
                created.add(lastGeneration(dir));
            }
            System.out.println("created_generations=" + join(created));
            System.out.println("snapshotted_generations=" + join(snapshotted));
            System.out.println("surviving_generations=" + join(commitGenerations(dir)));
        }
    }

    /** Reports every field of every commit {@code DirectoryReader.listCommits} returns. */
    private static void runListCommits(Path outDir) throws IOException {
        Path indexDir = outDir.resolve("index");
        deleteRecursively(indexDir);
        Files.createDirectories(indexDir);

        List<Long> created = new ArrayList<>();
        try (Directory dir = FSDirectory.open(indexDir)) {
            IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
            config.setOpenMode(OpenMode.CREATE);
            config.setIndexDeletionPolicy(NoDeletionPolicy.INSTANCE);
            try (IndexWriter writer = new IndexWriter(dir, config)) {
                for (int i = 0; i < COMMITS; i++) {
                    writer.addDocument(doc(i));
                    if (i == 2 || i == 4) {
                        // Prove that user data survives the round-trip too.
                        writer.setLiveCommitData(
                                List.of(
                                        new AbstractMap.SimpleEntry<>("round", String.valueOf(i)),
                                        new AbstractMap.SimpleEntry<>("source", "fixture")));
                    }
                    writer.commit();
                    created.add(lastGeneration(dir));
                }
            }
            System.out.println("created_generations=" + join(created));
            for (IndexCommit commit : DirectoryReader.listCommits(dir)) {
                System.out.println(
                        "commit="
                                + commit.getGeneration()
                                + ";"
                                + commit.getSegmentsFileName()
                                + ";"
                                + commit.getSegmentCount()
                                + ";"
                                + joinStrings(new TreeSet<>(commit.getFileNames()))
                                + ";"
                                + joinUserData(commit.getUserData()));
            }
        }
    }

    /** A {@link TwoPhaseCommit} that records every call and can fail on demand. */
    private static final class Recorder implements TwoPhaseCommit {
        private final String name;
        private final List<String> trace;
        private final String failAt;

        Recorder(String name, List<String> trace, String failAt) {
            this.name = name;
            this.trace = trace;
            this.failAt = failAt;
        }

        @Override
        public long prepareCommit() throws IOException {
            trace.add(name + ":prepare");
            if ("prepare".equals(failAt)) {
                throw new IOException("prepare exploded");
            }
            return 1;
        }

        @Override
        public long commit() throws IOException {
            trace.add(name + ":commit");
            if ("commit".equals(failAt)) {
                throw new IOException("commit exploded");
            }
            return 2;
        }

        @Override
        public void rollback() {
            trace.add(name + ":rollback");
        }

        @Override
        public String toString() {
            return "Recorder(" + name + ")";
        }
    }

    /** Runs {@link TwoPhaseCommitTool#execute} three ways and reports the call traces. */
    private static void runTwoPhaseCommit() throws IOException {
        // A `null` element in the middle proves it is skipped by both phases and
        // by the rollback sweep.
        List<String> trace = new ArrayList<>();
        TwoPhaseCommitTool.execute(
                new Recorder("a", trace, null), null, new Recorder("c", trace, null));
        System.out.println("success_trace=" + joinStrings(trace));

        trace = new ArrayList<>();
        String type = "none";
        String message = "";
        try {
            TwoPhaseCommitTool.execute(
                    new Recorder("a", trace, null),
                    new Recorder("b", trace, "prepare"),
                    new Recorder("c", trace, null));
        } catch (IOException e) {
            type = e.getClass().getSimpleName();
            message = e.getMessage();
        }
        System.out.println("prepare_fail_trace=" + joinStrings(trace));
        System.out.println("prepare_fail_type=" + type);
        System.out.println("prepare_fail_message=" + message);

        trace = new ArrayList<>();
        type = "none";
        message = "";
        try {
            TwoPhaseCommitTool.execute(
                    new Recorder("a", trace, null),
                    new Recorder("b", trace, "commit"),
                    new Recorder("c", trace, null));
        } catch (IOException e) {
            type = e.getClass().getSimpleName();
            message = e.getMessage();
        }
        System.out.println("commit_fail_trace=" + joinStrings(trace));
        System.out.println("commit_fail_type=" + type);
        System.out.println("commit_fail_message=" + message);
    }

    /** An {@link IndexOutput} whose {@code close()} fails, as a full disk would. */
    private static final class FailingCloseOutput extends FilterIndexOutput {
        FailingCloseOutput(IndexOutput out) {
            super("FailingCloseOutput(" + out + ")", out.getName(), out);
        }

        @Override
        public void close() throws IOException {
            super.close();
            throw new IOException("no space left on device");
        }
    }

    /** A {@link Directory} whose outputs can be made to fail on {@code close()}. */
    private static final class FailingCloseDirectory extends FilterDirectory {
        private volatile boolean failing;

        FailingCloseDirectory(Directory in) {
            super(in);
        }

        @Override
        public IndexOutput createOutput(String name, IOContext context) throws IOException {
            IndexOutput out = super.createOutput(name, context);
            return failing ? new FailingCloseOutput(out) : out;
        }
    }

    /**
     * Shows what Lucene does when the {@code snapshots_N} save file cannot be closed: the file is
     * kept, and because both implementations create it with {@code CREATE_NEW} semantics the very
     * next {@code snapshot()} fails on the same generation, for good.
     */
    private static void runPersistCloseFailure(Path outDir) throws IOException {
        Path indexDir = outDir.resolve("index");
        Path snapshotsDir = outDir.resolve("snapshots");
        deleteRecursively(indexDir);
        deleteRecursively(snapshotsDir);
        Files.createDirectories(indexDir);
        Files.createDirectories(snapshotsDir);

        try (Directory dir = FSDirectory.open(indexDir)) {
            IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
            config.setOpenMode(OpenMode.CREATE);
            try (IndexWriter writer = new IndexWriter(dir, config)) {
                writer.addDocument(doc(0));
                writer.commit();
            }

            try (Directory raw = FSDirectory.open(snapshotsDir)) {
                FailingCloseDirectory snapDir = new FailingCloseDirectory(raw);
                SnapshotProbe policy =
                        new SnapshotProbe(
                                new KeepOnlyLastCommitDeletionPolicy(), snapDir, OpenMode.CREATE);
                policy.onInit(DirectoryReader.listCommits(dir));

                snapDir.failing = true;
                String firstType = "none";
                try {
                    policy.snapshot();
                } catch (Exception e) {
                    firstType = e.getClass().getSimpleName();
                }
                System.out.println("first_snapshot_error=" + firstType);
                System.out.println("count_after_failure=" + policy.getSnapshotCount());
                System.out.println("last_save_file_after_failure=" + policy.getLastSaveFile());
                System.out.println("files_after_failure=" + joinStrings(snapshotFiles(snapDir)));

                snapDir.failing = false;
                String secondType = "none";
                try {
                    policy.snapshot();
                } catch (Exception e) {
                    secondType = e.getClass().getSimpleName();
                }
                System.out.println("second_snapshot_error=" + secondType);
                System.out.println("count_after_retry=" + policy.getSnapshotCount());
                System.out.println("files_after_retry=" + joinStrings(snapshotFiles(snapDir)));

                // A third attempt shows how many snapshots the failure cost:
                // the second attempt's `finally` cleans the leftover up, so the
                // policy only loses the snapshots taken in between.
                String thirdType = "none";
                try {
                    policy.snapshot();
                } catch (Exception e) {
                    thirdType = e.getClass().getSimpleName();
                }
                System.out.println("third_snapshot_error=" + thirdType);
                System.out.println("count_after_third=" + policy.getSnapshotCount());
                System.out.println("last_save_file_after_third=" + policy.getLastSaveFile());
                System.out.println("files_after_third=" + joinStrings(snapshotFiles(snapDir)));
            }
        }
    }

    /** Returns every {@code snapshots_*} file in {@code dir}, sorted. */
    private static Collection<String> snapshotFiles(Directory dir) throws IOException {
        Collection<String> files = new TreeSet<>();
        for (String file : dir.listAll()) {
            if (file.startsWith(PersistentSnapshotDeletionPolicy.SNAPSHOTS_PREFIX)) {
                files.add(file);
            }
        }
        return files;
    }

    /** Returns the generations held by {@code policy}, sorted. */
    private static List<Long> snapshotGenerations(SnapshotDeletionPolicy policy) {
        List<Long> generations = new ArrayList<>();
        for (IndexCommit commit : policy.getSnapshots()) {
            generations.add(commit.getGeneration());
        }
        generations.sort(Long::compare);
        return generations;
    }

    private static String joinStrings(Iterable<String> values) {
        StringBuilder sb = new StringBuilder();
        boolean first = true;
        for (String value : values) {
            if (!first) {
                sb.append(',');
            }
            first = false;
            sb.append(value);
        }
        return sb.toString();
    }

    private static Document doc(int i) {
        Document document = new Document();
        document.add(new StringField("id", "doc" + i, Field.Store.YES));
        return document;
    }

    /** Returns the generation of the commit the directory currently points at. */
    private static long lastGeneration(Directory dir) throws IOException {
        List<IndexCommit> commits = new ArrayList<>(DirectoryReader.listCommits(dir));
        return commits.get(commits.size() - 1).getGeneration();
    }

    /** Returns every commit generation still present in the directory, oldest first. */
    private static List<Long> commitGenerations(Directory dir) throws IOException {
        List<Long> generations = new ArrayList<>();
        for (IndexCommit commit : DirectoryReader.listCommits(dir)) {
            generations.add(commit.getGeneration());
        }
        return generations;
    }

    private static String join(List<Long> values) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < values.size(); i++) {
            if (i > 0) {
                sb.append(',');
            }
            sb.append(values.get(i));
        }
        return sb.toString();
    }

    /** Renders a string map as {@code key:value,key:value}, sorted by key. */
    private static String joinUserData(Map<String, String> userData) {
        StringBuilder sb = new StringBuilder();
        boolean first = true;
        for (Map.Entry<String, String> entry : new TreeMap<>(userData).entrySet()) {
            if (!first) {
                sb.append(',');
            }
            first = false;
            sb.append(entry.getKey()).append(':').append(entry.getValue());
        }
        return sb.toString();
    }

    private static String joinCounts(Map<Long, Integer> counts) {
        StringBuilder sb = new StringBuilder();
        boolean first = true;
        for (Map.Entry<Long, Integer> entry : counts.entrySet()) {
            if (!first) {
                sb.append(',');
            }
            first = false;
            sb.append(entry.getKey()).append(':').append(entry.getValue());
        }
        return sb.toString();
    }

    private static void deleteRecursively(Path path) throws IOException {
        if (!Files.exists(path)) {
            return;
        }
        try (var stream = Files.walk(path)) {
            for (Path p : stream.sorted((a, b) -> b.getNameCount() - a.getNameCount()).toList()) {
                Files.deleteIfExists(p);
            }
        }
    }
}
