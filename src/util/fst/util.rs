//! Port of `org.apache.lucene.util.fst.Util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`Util`] | `Util` |
//! | [`FSTPath`] | `Util.FSTPath<T>` |
//! | [`TopNSearcher`] | `Util.TopNSearcher<T>` |
//! | [`PathResult`] | `Util.Result<T>` |
//! | [`TopResults`] | `Util.TopResults<T>` |

use std::cmp::Ordering;
use std::collections::HashSet;
use std::io::Write;
use std::rc::Rc;

use crate::error::{LuceneError, Result};
use crate::util::{BytesRef, BytesRefBuilder, IntsRef, IntsRefBuilder};

use super::fst::{
    target_has_arcs, Arc, BitTable, BytesReader, ARCS_FOR_BINARY_SEARCH, ARCS_FOR_CONTINUOUS,
    ARCS_FOR_DIRECT_ADDRESSING, BIT_TARGET_NEXT, END_LABEL, FST,
};
use super::fst_compiler::ints_ref_compare;
use super::outputs::Outputs;

/// Comparator over FST outputs.
///
/// Equivalent to the `Comparator<T>` parameter of `Util.TopNSearcher`.
pub type OutputComparator<T> = Rc<dyn Fn(&T, &T) -> Ordering>;

/// Comparator over paths.
///
/// Equivalent to the `Comparator<FSTPath<T>>` parameter of
/// `Util.TopNSearcher`.
pub type PathComparator<T> = Rc<dyn Fn(&FSTPath<T>, &FSTPath<T>) -> Ordering>;

/// Represents a path in [`TopNSearcher`].
///
/// Equivalent to `Util.FSTPath<T>`.
#[derive(Debug, Clone)]
pub struct FSTPath<T> {
    /// Holds the last arc appended to this path.
    pub arc: Arc<T>,
    /// Holds the cost plus any usage-specific output.
    pub output: T,
    /// The input consumed so far.
    pub input: IntsRefBuilder,
    /// A usage-specific boost.
    pub boost: f32,
    /// A usage-specific context.
    pub context: Option<String>,
    /// Custom payload for consumers; the NRT suggester uses it to record
    /// whether a path has already enumerated a surface form.
    pub payload: i32,
}

impl<T: Clone + Default> FSTPath<T> {
    /// Creates a path ending at a copy of `arc`.
    ///
    /// Equivalent to the package-private `FSTPath` constructor.
    pub fn new(
        output: T,
        arc: &Arc<T>,
        input: IntsRefBuilder,
        boost: f32,
        context: Option<String>,
        payload: i32,
    ) -> Self {
        let mut copy = Arc::default();
        copy.copy_from(arc);
        Self {
            arc: copy,
            output,
            input,
            boost,
            context,
            payload,
        }
    }

    /// Creates a new path that extends this one.
    ///
    /// Equivalent to `FSTPath.newPath`.
    pub fn new_path(&self, output: T, input: IntsRefBuilder) -> Self {
        Self::new(
            output,
            &self.arc,
            input,
            self.boost,
            self.context.clone(),
            self.payload,
        )
    }
}

/// Hooks a caller can use to filter the paths a [`TopNSearcher`] considers.
///
/// Equivalent to the three `protected` methods of `Util.TopNSearcher` that
/// subclasses override: `acceptPartialPath`, `acceptResult(FSTPath)` and
/// `acceptResult(IntsRef, T)`.
pub trait TopNSearcherHooks<T> {
    /// Prevents a path from being considered before it is complete.
    ///
    /// Equivalent to `TopNSearcher.acceptPartialPath`.
    fn accept_partial_path(&mut self, _path: &FSTPath<T>) -> bool {
        true
    }

    /// Accepts or rejects a completed result.
    ///
    /// Equivalent to `TopNSearcher.acceptResult(IntsRef, T)`.
    fn accept_result(&mut self, _input: &IntsRef, _output: &T) -> bool {
        true
    }

    /// Accepts or rejects a completed path.
    ///
    /// Equivalent to `TopNSearcher.acceptResult(FSTPath)`, whose default
    /// implementation forwards to
    /// [`TopNSearcherHooks::accept_result`].
    fn accept_result_path(&mut self, path: &FSTPath<T>) -> bool {
        let input = path.input.get();
        self.accept_result(&input, &path.output)
    }
}

/// The hooks that accept every path, matching the base `TopNSearcher`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultTopNSearcherHooks;

impl<T> TopNSearcherHooks<T> for DefaultTopNSearcherHooks {}

/// Holds a single input ([`IntsRef`]) and output, as returned by
/// [`Util::shortest_paths`].
///
/// Equivalent to the record `Util.Result<T>`, renamed here so that it does not
/// shadow [`crate::error::Result`]. A Java record derives `equals`; this port
/// cannot, because `crate::util::IntsRef` does not implement [`PartialEq`] and
/// is defined outside this module.
#[derive(Debug, Clone)]
pub struct PathResult<T> {
    /// The term.
    pub input: IntsRef,
    /// The output the FST maps the term to.
    pub output: T,
}

/// Holds the results of a top-N search.
///
/// Equivalent to `Util.TopResults<T>`.
#[derive(Debug, Clone)]
pub struct TopResults<T> {
    /// `true` when this is a complete result, that is, when the queue size was
    /// large enough to find the complete list of results. It is `false` when
    /// the [`TopNSearcher`] rejected too many results.
    pub is_complete: bool,
    /// The top results.
    pub top_n: Vec<PathResult<T>>,
}

impl<T> TopResults<T> {
    /// Iterates over the top results.
    ///
    /// Equivalent to `TopResults.iterator`.
    pub fn iter(&self) -> std::slice::Iter<'_, PathResult<T>> {
        self.top_n.iter()
    }
}

impl<'r, T> IntoIterator for &'r TopResults<T> {
    type Item = &'r PathResult<T>;
    type IntoIter = std::slice::Iter<'r, PathResult<T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.top_n.iter()
    }
}

/// Finds the top N shortest paths from a starting point.
///
/// Equivalent to `Util.TopNSearcher<T>`.
///
/// # Java to Rust adaptations
///
/// * The `TreeSet<FSTPath<T>>` queue becomes a `Vec` kept sorted with the same
///   comparator. `TreeSet` is a *set*, so a path that compares equal to one
///   already queued is dropped; the sorted vector reproduces that, and
///   `pollFirst`, `pollLast` and `last` map to removing the first element,
///   popping the last one, and reading the last one.
/// * The three `protected` accept methods become the [`TopNSearcherHooks`]
///   trait, so that callers can filter paths without subclassing.
pub struct TopNSearcher<'a, O: Outputs, H = DefaultTopNSearcherHooks>
where
    H: TopNSearcherHooks<O::Output>,
{
    fst: &'a FST<O>,
    bytes_reader: Box<dyn BytesReader + 'a>,
    top_n: usize,
    max_queue_depth: usize,
    scratch_arc: Arc<O::Output>,
    comparator: OutputComparator<O::Output>,
    path_comparator: PathComparator<O::Output>,
    queue: Option<Vec<FSTPath<O::Output>>>,
    hooks: H,
}

/// Builds the default path comparator: compare with `comparator` first, then
/// break ties on the path input.
///
/// Equivalent to the record `Util.TieBreakByInputComparator<T>`.
fn tie_break_by_input_comparator<T: Clone + Default + 'static>(
    comparator: OutputComparator<T>,
) -> PathComparator<T> {
    Rc::new(move |a: &FSTPath<T>, b: &FSTPath<T>| {
        let cmp = comparator(&a.output, &b.output);
        if cmp == Ordering::Equal {
            ints_ref_compare(&a.input.get(), &b.input.get()).cmp(&0)
        } else {
            cmp
        }
    })
}

impl<'a, O: Outputs> TopNSearcher<'a, O, DefaultTopNSearcherHooks>
where
    O::Output: 'static,
{
    /// Creates a searcher that ties results by input order.
    ///
    /// Equivalent to
    /// `TopNSearcher(FST, int, int, Comparator)`.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while opening the FST's byte reader.
    pub fn new(
        fst: &'a FST<O>,
        top_n: usize,
        max_queue_depth: usize,
        comparator: OutputComparator<O::Output>,
    ) -> Result<Self> {
        let path_comparator = tie_break_by_input_comparator(Rc::clone(&comparator));
        Self::with_path_comparator(
            fst,
            top_n,
            max_queue_depth,
            comparator,
            path_comparator,
            DefaultTopNSearcherHooks,
        )
    }
}

impl<'a, O: Outputs, H: TopNSearcherHooks<O::Output>> TopNSearcher<'a, O, H> {
    /// Creates a searcher with an explicit path comparator and filtering hooks.
    ///
    /// Equivalent to
    /// `TopNSearcher(FST, int, int, Comparator, Comparator)`, plus the
    /// subclass that would override the accept methods.
    ///
    /// # Errors
    ///
    /// Propagates the error raised while opening the FST's byte reader.
    pub fn with_path_comparator(
        fst: &'a FST<O>,
        top_n: usize,
        max_queue_depth: usize,
        comparator: OutputComparator<O::Output>,
        path_comparator: PathComparator<O::Output>,
        hooks: H,
    ) -> Result<Self> {
        Ok(Self {
            fst,
            bytes_reader: fst.get_bytes_reader()?,
            top_n,
            max_queue_depth,
            scratch_arc: Arc::default(),
            comparator,
            path_comparator,
            queue: Some(Vec::new()),
            hooks,
        })
    }

    /// Adds `path` to the queue when it competes with what is already queued.
    ///
    /// Equivalent to the protected `TopNSearcher.addIfCompetitive`.
    fn add_if_competitive(&mut self, path: &mut FSTPath<O::Output>) {
        let queue = match &self.queue {
            Some(queue) => queue,
            None => return,
        };

        let output = self.fst.outputs().add(&path.output, path.arc.output());

        if queue.len() == self.max_queue_depth {
            let bottom = queue
                .last()
                .expect("INVARIANT: max_queue_depth is only reached with a non-empty queue");
            let comp = (self.path_comparator)(path, bottom);
            if comp == Ordering::Greater {
                // Does not compete.
                return;
            } else if comp == Ordering::Equal {
                // Tie break by alphabetical sort on the input.
                path.input.append(path.arc.label());
                let cmp = ints_ref_compare(&bottom.input.get(), &path.input.get());
                path.input.set_length(path.input.length() - 1);

                // Duplicates should never be seen.
                debug_assert_ne!(cmp, 0);

                if cmp < 0 {
                    // Does not compete.
                    return;
                }
            }
            // Competes.
        }
        // Otherwise the queue is not full yet, so any path that is hit competes.

        // Copy the current input to the new input and append the arc label.
        let mut new_input = IntsRefBuilder::new();
        new_input.copy_ints_ref(&path.input.get());
        new_input.append(path.arc.label());

        let new_path = path.new_path(output, new_input);
        if self.hooks.accept_partial_path(&new_path) {
            self.queue_add(new_path);
            let max_queue_depth = self.max_queue_depth;
            if let Some(queue) = &mut self.queue {
                if queue.len() == max_queue_depth + 1 {
                    queue.pop();
                }
            }
        }
    }

    /// Inserts `path` in sorted position, dropping it when an equal path is
    /// already queued.
    ///
    /// Equivalent to `TreeSet.add`, which is a no-op for an element that
    /// compares equal to an existing one.
    fn queue_add(&mut self, path: FSTPath<O::Output>) {
        let path_comparator = Rc::clone(&self.path_comparator);
        if let Some(queue) = &mut self.queue {
            match queue.binary_search_by(|probe| path_comparator(probe, &path)) {
                Ok(_) => {}
                Err(pos) => queue.insert(pos, path),
            }
        }
    }

    /// Adds every arc leaving `node`, including the "finished" arc when the
    /// node is final, to the queue.
    ///
    /// Equivalent to `TopNSearcher.addStartPaths(Arc, T, boolean,
    /// IntsRefBuilder)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn add_start_paths(
        &mut self,
        node: &Arc<O::Output>,
        start_output: O::Output,
        allow_empty_string: bool,
        input: IntsRefBuilder,
    ) -> Result<()> {
        self.add_start_paths_full(node, start_output, allow_empty_string, input, 0.0, None, -1)
    }

    /// Adds every arc leaving `node` to the queue, with a boost, a context and
    /// a payload.
    ///
    /// Equivalent to `TopNSearcher.addStartPaths(Arc, T, boolean,
    /// IntsRefBuilder, float, CharSequence, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    // Mirrors Lucene's seven-argument `addStartPaths` overload.
    #[allow(clippy::too_many_arguments)]
    pub fn add_start_paths_full(
        &mut self,
        node: &Arc<O::Output>,
        start_output: O::Output,
        allow_empty_string: bool,
        input: IntsRefBuilder,
        boost: f32,
        context: Option<String>,
        payload: i32,
    ) -> Result<()> {
        let mut path = FSTPath::new(start_output, node, input, boost, context, payload);
        self.fst
            .read_first_target_arc(node, &mut path.arc, &mut *self.bytes_reader)?;

        // Bootstrap: find the minimum starting arc.
        loop {
            if allow_empty_string || path.arc.label() != END_LABEL {
                self.add_if_competitive(&mut path);
            }
            if path.arc.is_last() {
                break;
            }
            self.fst
                .read_next_arc(&mut path.arc, &mut *self.bytes_reader)?;
        }
        Ok(())
    }

    /// Runs the search and returns the top results.
    ///
    /// Equivalent to `TopNSearcher.search`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn search(&mut self) -> Result<TopResults<O::Output>> {
        let mut results: Vec<PathResult<O::Output>> = Vec::new();

        let fst = self.fst;
        let mut fst_reader = fst.get_bytes_reader()?;
        let no_output = fst.outputs().no_output();

        let mut reject_count = 0usize;

        // For each of the top N paths.
        while results.len() < self.top_n {
            let path = match &mut self.queue {
                // Ran out of paths.
                None => break,
                Some(queue) => {
                    if queue.is_empty() {
                        // There were fewer than top_n paths available.
                        break;
                    }
                    // Remove the top path, which is now going to be pursued.
                    queue.remove(0)
                }
            };
            let mut path = path;

            if !self.hooks.accept_partial_path(&path) {
                continue;
            }

            if path.arc.label() == END_LABEL {
                // Empty string.
                path.input.set_length(path.input.length() - 1);
                results.push(PathResult {
                    input: path.input.get(),
                    output: path.output,
                });
                continue;
            }

            if results.len() + 1 == self.top_n && self.max_queue_depth == self.top_n {
                // Last path: the queue is no longer needed.
                self.queue = None;
            }

            // Take the path and find its "0 output completion", that is, keep
            // traversing the first arc with NO_OUTPUT that can be found, since
            // that must lead to the minimum path completing from path.arc.

            // For each input letter.
            loop {
                fst.read_first_target_arc_in_place(&mut path.arc, &mut *fst_reader)?;

                // For each arc leaving this node.
                let mut found_zero = false;
                let mut arc_copy_is_pending = false;
                loop {
                    // Tricky: instead of comparing output == 0, this must be
                    // expressed as compare(output, 0) == 0.
                    if (self.comparator)(&no_output, path.arc.output()) == Ordering::Equal {
                        if self.queue.is_none() {
                            found_zero = true;
                            break;
                        } else if !found_zero {
                            arc_copy_is_pending = true;
                            found_zero = true;
                        } else {
                            self.add_if_competitive(&mut path);
                        }
                    } else if self.queue.is_some() {
                        self.add_if_competitive(&mut path);
                    }
                    if path.arc.is_last() {
                        break;
                    }
                    if arc_copy_is_pending {
                        self.scratch_arc.copy_from(&path.arc);
                        arc_copy_is_pending = false;
                    }
                    fst.read_next_arc(&mut path.arc, &mut *fst_reader)?;
                }

                debug_assert!(found_zero);

                if self.queue.is_some() && !arc_copy_is_pending {
                    path.arc.copy_from(&self.scratch_arc);
                }

                if path.arc.label() == END_LABEL {
                    // Add the final output.
                    path.output = fst.outputs().add(&path.output, path.arc.output());
                    if self.hooks.accept_result_path(&path) {
                        results.push(PathResult {
                            input: path.input.get(),
                            output: path.output.clone(),
                        });
                    } else {
                        reject_count += 1;
                    }
                    break;
                }
                path.input.append(path.arc.label());
                path.output = fst.outputs().add(&path.output, path.arc.output());
                if !self.hooks.accept_partial_path(&path) {
                    break;
                }
            }
        }
        Ok(TopResults {
            is_complete: reject_count + self.top_n <= self.max_queue_depth,
            top_n: results,
        })
    }
}

/// Static helper methods for FSTs.
///
/// Equivalent to `org.apache.lucene.util.fst.Util`. Its package-private static
/// `binarySearch` stays crate-visible as the module-level
/// [`binary_search`](self::binary_search) function, matching Lucene's
/// visibility.
pub struct Util;

impl Util {
    /// Looks up the output for `input`, or `None` when the input is not
    /// accepted.
    ///
    /// Equivalent to `Util.get(FST, IntsRef)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn get<O: Outputs>(fst: &FST<O>, input: &IntsRef) -> Result<Option<O::Output>> {
        let mut arc = Arc::default();
        fst.get_first_arc(&mut arc);

        let mut fst_reader = fst.get_bytes_reader()?;

        // Accumulate the output as the input is consumed.
        let mut output = fst.outputs().no_output();
        for i in 0..input.length {
            let label = input.ints[input.offset + i];
            if !fst.find_target_arc_in_place(label, &mut arc, &mut *fst_reader)? {
                return Ok(None);
            }
            output = fst.outputs().add(&output, arc.output());
        }

        if arc.is_final() {
            Ok(Some(fst.outputs().add(&output, arc.next_final_output())))
        } else {
            Ok(None)
        }
    }

    /// Looks up the output for `input`, or `None` when the input is not
    /// accepted.
    ///
    /// Equivalent to `Util.get(FST, BytesRef)`, the `BYTE1` overload; Rust has
    /// no overloading, hence the distinct name.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn get_bytes<O: Outputs>(fst: &FST<O>, input: &BytesRef) -> Result<Option<O::Output>> {
        debug_assert_eq!(fst.metadata().input_type(), super::fst::InputType::Byte1);

        let mut fst_reader = fst.get_bytes_reader()?;

        let mut arc = Arc::default();
        fst.get_first_arc(&mut arc);

        // Accumulate the output as the input is consumed.
        let mut output = fst.outputs().no_output();
        for i in 0..input.length {
            let label = i32::from(input.bytes[i + input.offset]);
            if !fst.find_target_arc_in_place(label, &mut arc, &mut *fst_reader)? {
                return Ok(None);
            }
            output = fst.outputs().add(&output, arc.output());
        }

        if arc.is_final() {
            Ok(Some(fst.outputs().add(&output, arc.next_final_output())))
        } else {
            Ok(None)
        }
    }

    /// Starting from `from_node`, finds the top N minimum-cost completions to a
    /// final node.
    ///
    /// Equivalent to `Util.shortestPaths`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn shortest_paths<O: Outputs>(
        fst: &FST<O>,
        from_node: &Arc<O::Output>,
        start_output: O::Output,
        comparator: OutputComparator<O::Output>,
        top_n: usize,
        allow_empty_string: bool,
    ) -> Result<TopResults<O::Output>>
    where
        O::Output: 'static,
    {
        // All paths are kept, so top_n can be passed for max_queue_depth and
        // the pruning is admissible.
        let mut searcher = TopNSearcher::new(fst, top_n, top_n, comparator)?;

        // Since this search is initialised with a single start node, it is fine
        // to start with an empty input path.
        searcher.add_start_paths(
            from_node,
            start_output,
            allow_empty_string,
            IntsRefBuilder::new(),
        )?;
        searcher.search()
    }

    /// Dumps an FST to GraphViz's `dot` language, for visualization.
    ///
    /// Equivalent to `Util.toDot`. Larger FSTs, of a few thousand nodes, will
    /// not even render.
    ///
    /// `same_rank` tries to order states in layers of breadth-first traversal,
    /// which may mess up arcs but makes the structure clearer. `label_states`
    /// gives states labels equal to their offsets in the binary format, which
    /// expands the graph considerably.
    ///
    /// # Errors
    ///
    /// Propagates reader and I/O errors.
    pub fn to_dot<O: Outputs>(
        fst: &FST<O>,
        out: &mut dyn Write,
        same_rank: bool,
        label_states: bool,
    ) -> Result<()> {
        let expanded_node_color = "blue";

        // The start arc of the automaton, from the epsilon state to the first
        // state with outgoing transitions.
        let mut start_arc = Arc::default();
        fst.get_first_arc(&mut start_arc);

        // Transitions to consider for the next level.
        let mut this_level_queue: Vec<Arc<O::Output>> = Vec::new();
        let mut next_level_queue: Vec<Arc<O::Output>> = vec![start_arc.clone()];

        // States on the same level, for ranking.
        let mut same_level_states: Vec<i64> = Vec::new();

        // Already seen states, by target offset.
        let mut seen: HashSet<i64> = HashSet::new();
        seen.insert(start_arc.target());

        let state_shape = "circle";
        let final_state_shape = "doublecircle";

        writeln!(out, "digraph FST {{")?;
        writeln!(
            out,
            "  rankdir = LR; splines=true; concentrate=true; ordering=out; ranksep=2.5; "
        )?;

        if !label_states {
            writeln!(
                out,
                "  node [shape=circle, width=.2, height=.2, style=filled]"
            )?;
        }

        emit_dot_state(out, "initial", Some("point"), Some("white"), "")?;

        let no_output = fst.outputs().no_output();
        let mut r = fst.get_bytes_reader()?;

        {
            let state_color = if fst.is_expanded_target(&start_arc, &mut *r)? {
                Some(expanded_node_color)
            } else {
                None
            };

            let (is_final, final_output) = if start_arc.is_final() {
                let output = if fst
                    .outputs()
                    .equals(start_arc.next_final_output(), &no_output)
                {
                    None
                } else {
                    Some(start_arc.next_final_output().clone())
                };
                (true, output)
            } else {
                (false, None)
            };

            emit_dot_state(
                out,
                &start_arc.target().to_string(),
                Some(if is_final {
                    final_state_shape
                } else {
                    state_shape
                }),
                state_color,
                &match &final_output {
                    None => String::new(),
                    Some(output) => fst.outputs().output_to_string(output),
                },
            )?;
        }

        writeln!(out, "  initial -> {}", start_arc.target())?;

        let mut level = 0u64;

        while !next_level_queue.is_empty() {
            this_level_queue.append(&mut next_level_queue);

            level += 1;
            writeln!(out, "\n  // Transitions and states at level: {level}")?;
            while let Some(mut arc) = this_level_queue.pop() {
                if target_has_arcs(&arc) {
                    // Scan all target arcs.
                    let node = arc.target();

                    fst.read_first_real_target_arc(arc.target(), &mut arc, &mut *r)?;

                    loop {
                        // Emit the unseen state and add it to the queue for the
                        // next level.
                        if arc.target() >= 0 && !seen.contains(&arc.target()) {
                            let state_color = if fst.is_expanded_target(&arc, &mut *r)? {
                                Some(expanded_node_color)
                            } else {
                                None
                            };

                            let final_output =
                                if !fst.outputs().equals(arc.next_final_output(), &no_output) {
                                    fst.outputs().output_to_string(arc.next_final_output())
                                } else {
                                    String::new()
                                };

                            emit_dot_state(
                                out,
                                &arc.target().to_string(),
                                Some(state_shape),
                                state_color,
                                &final_output,
                            )?;
                            seen.insert(arc.target());
                            next_level_queue.push(arc.clone());
                            same_level_states.push(arc.target());
                        }

                        let mut outs = if !fst.outputs().equals(arc.output(), &no_output) {
                            format!("/{}", fst.outputs().output_to_string(arc.output()))
                        } else {
                            String::new()
                        };

                        if !target_has_arcs(&arc)
                            && arc.is_final()
                            && !fst.outputs().equals(arc.next_final_output(), &no_output)
                        {
                            // Tricky special case: due to pruning, the builder
                            // can produce an FST with an arc into the final end
                            // state (-1) that also has a next final output; in
                            // that case the output is pulled up onto this arc.
                            outs = format!(
                                "{outs}/[{}]",
                                fst.outputs().output_to_string(arc.next_final_output())
                            );
                        }

                        let arc_color = if arc.flag(BIT_TARGET_NEXT) {
                            "red"
                        } else {
                            "black"
                        };

                        debug_assert_ne!(arc.label(), END_LABEL);
                        writeln!(
                            out,
                            "  {} -> {} [label=\"{}{}\"{} color=\"{}\"]",
                            node,
                            arc.target(),
                            printable_label(arc.label()),
                            outs,
                            if arc.is_final() {
                                " style=\"bold\""
                            } else {
                                ""
                            },
                            arc_color
                        )?;

                        // Break the loop on the last arc of this state.
                        if arc.is_last() {
                            break;
                        }
                        fst.read_next_real_arc(&mut arc, &mut *r)?;
                    }
                }
            }

            // Emit state ranking information.
            if same_rank && same_level_states.len() > 1 {
                write!(out, "  {{rank=same; ")?;
                for state in &same_level_states {
                    write!(out, "{state}; ")?;
                }
                writeln!(out, " }}")?;
            }
            same_level_states.clear();
        }

        // Emit the terminating state, which is always there anyway.
        writeln!(
            out,
            "  -1 [style=filled, color=black, shape=doublecircle, label=\"\"]\n"
        )?;
        writeln!(out, "  {{rank=sink; -1 }}")?;

        writeln!(out, "}}")?;
        out.flush()?;
        Ok(())
    }

    /// Maps each UTF-16 code unit of `s` to an int of `scratch`.
    ///
    /// Equivalent to `Util.toUTF16`.
    pub fn to_utf16(s: &str, scratch: &mut IntsRefBuilder) -> IntsRef {
        let units: Vec<u16> = s.encode_utf16().collect();
        scratch.set_length(units.len());
        scratch.grow_no_copy(units.len());
        for (idx, unit) in units.iter().enumerate() {
            scratch.set_int_at(idx, i32::from(*unit));
        }
        scratch.get()
    }

    /// Decodes the Unicode code points of `s` into `scratch`.
    ///
    /// Equivalent to `Util.toUTF32(CharSequence, IntsRefBuilder)`.
    pub fn to_utf32(s: &str, scratch: &mut IntsRefBuilder) -> IntsRef {
        let mut int_idx = 0usize;
        for c in s.chars() {
            scratch.grow(int_idx + 1);
            scratch.set_int_at(int_idx, c as i32);
            int_idx += 1;
        }
        scratch.set_length(int_idx);
        scratch.get()
    }

    /// Decodes the Unicode code points of `s[offset..offset + length]` into
    /// `scratch`.
    ///
    /// Equivalent to `Util.toUTF32(char[], int, int, IntsRefBuilder)`; this
    /// port takes Unicode scalar values, since that is what a Rust `char` is.
    pub fn to_utf32_chars(
        s: &[char],
        offset: usize,
        length: usize,
        scratch: &mut IntsRefBuilder,
    ) -> IntsRef {
        let mut int_idx = 0usize;
        for c in &s[offset..offset + length] {
            scratch.grow(int_idx + 1);
            scratch.set_int_at(int_idx, *c as i32);
            int_idx += 1;
        }
        scratch.set_length(int_idx);
        scratch.get()
    }

    /// Takes the unsigned byte values of `input` and converts them into ints.
    ///
    /// Equivalent to `Util.toIntsRef`.
    pub fn to_ints_ref(input: &BytesRef, scratch: &mut IntsRefBuilder) -> IntsRef {
        scratch.grow_no_copy(input.length);
        for i in 0..input.length {
            scratch.set_int_at(i, i32::from(input.bytes[i + input.offset]));
        }
        scratch.set_length(input.length);
        scratch.get()
    }

    /// Converts an [`IntsRef`] to a [`BytesRef`]; the caller must ensure every
    /// value fits into a byte.
    ///
    /// Equivalent to `Util.toBytesRef`. Lucene allows `-128` to `255`, which
    /// this port reproduces.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a value does not fit into
    /// a byte; Lucene fails the same case with an assertion.
    pub fn to_bytes_ref(input: &IntsRef, scratch: &mut BytesRefBuilder) -> Result<BytesRef> {
        scratch.grow_no_copy(input.length);
        for i in 0..input.length {
            let value = input.ints[i + input.offset];
            if !(-128..=255).contains(&value) {
                return Err(LuceneError::IllegalArgument(format!(
                    "value {value} doesn't fit into byte"
                )));
            }
            scratch.set_byte_at(i, value as u8);
        }
        scratch.set_length(input.length);
        Ok(scratch.get())
    }

    /// Reads the first arc greater than or equal to `label` into `arc` and
    /// returns whether one was found.
    ///
    /// Equivalent to `Util.readCeilArc`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn read_ceil_arc<O: Outputs>(
        label: i32,
        fst: &FST<O>,
        follow: &Arc<O::Output>,
        arc: &mut Arc<O::Output>,
        input: &mut dyn BytesReader,
    ) -> Result<bool> {
        if label == END_LABEL {
            return Ok(FST::<O>::read_end_arc(follow, arc));
        }
        if !target_has_arcs(follow) {
            return Ok(false);
        }
        fst.read_first_target_arc(follow, arc, input)?;
        if arc.bytes_per_arc() != 0 && arc.label() != END_LABEL {
            if arc.node_flags() == ARCS_FOR_DIRECT_ADDRESSING {
                // Fixed length arcs in a direct addressing node.
                let target_index = label - arc.label();
                if target_index >= arc.num_arcs() {
                    return Ok(false);
                } else if target_index < 0 {
                    return Ok(true);
                } else {
                    if BitTable::is_bit_set(target_index, arc, input)? {
                        fst.read_arc_by_direct_addressing(arc, input, target_index)?;
                    } else {
                        let ceil_index = BitTable::next_bit_set(target_index, arc, input)?;
                        debug_assert_ne!(ceil_index, -1);
                        fst.read_arc_by_direct_addressing(arc, input, ceil_index)?;
                    }
                    return Ok(true);
                }
            } else if arc.node_flags() == ARCS_FOR_CONTINUOUS {
                let target_index = label - arc.label();
                if target_index >= arc.num_arcs() {
                    return Ok(false);
                } else if target_index < 0 {
                    return Ok(true);
                } else {
                    fst.read_arc_by_continuous(arc, input, target_index)?;
                    return Ok(true);
                }
            }
            // Fixed length arcs in a binary search node.
            let mut idx = binary_search(fst, arc, label)?;
            if idx >= 0 {
                fst.read_arc_by_index(arc, input, idx)?;
                return Ok(true);
            }
            idx = -1 - idx;
            if idx == arc.num_arcs() {
                // Dead end.
                return Ok(false);
            }
            fst.read_arc_by_index(arc, input, idx)?;
            return Ok(true);
        }

        // Variable length arcs in a linear scan list, or a special arc with
        // label == END_LABEL.
        fst.read_first_real_target_arc(follow.target(), arc, input)?;

        loop {
            if arc.label() >= label {
                return Ok(true);
            } else if arc.is_last() {
                return Ok(false);
            }
            fst.read_next_real_arc(arc, input)?;
        }
    }
}

/// Performs a binary search over arcs encoded as a packed array.
///
/// Equivalent to the package-private static `Util.binarySearch`. Returns the
/// index of the arc carrying `target_label`, or `-1 - idx` where `idx` is the
/// index of the arc with the next highest label, or the total number of arcs
/// when the target label exceeds the maximum.
pub(crate) fn binary_search<O: Outputs>(
    fst: &FST<O>,
    arc: &Arc<O::Output>,
    target_label: i32,
) -> Result<i32> {
    debug_assert_eq!(arc.node_flags(), ARCS_FOR_BINARY_SEARCH);
    let mut input = fst.get_bytes_reader()?;
    let mut low = arc.arc_idx();
    let mut high = arc.num_arcs() - 1;
    while low <= high {
        let mid = ((low as u32 + high as u32) >> 1) as i32;
        input.set_position(arc.pos_arcs_start());
        input.skip_bytes(i64::from(arc.bytes_per_arc()) * i64::from(mid) + 1)?;
        let mid_label = fst.read_label(input.as_data_input())?;
        let cmp = mid_label - target_label;
        if cmp < 0 {
            low = mid + 1;
        } else if cmp > 0 {
            high = mid - 1;
        } else {
            return Ok(mid);
        }
    }
    Ok(-1 - low)
}

/// Emits a single state in the `dot` language.
///
/// Equivalent to the private `Util.emitDotState`.
fn emit_dot_state(
    out: &mut dyn Write,
    name: &str,
    shape: Option<&str>,
    color: Option<&str>,
    label: &str,
) -> Result<()> {
    writeln!(
        out,
        "  {} [{} {} label=\"{}\" ]",
        name,
        match shape {
            Some(shape) => format!("shape={shape}"),
            None => String::new(),
        },
        match color {
            Some(color) => format!("color={color}"),
            None => String::new(),
        },
        label
    )?;
    Ok(())
}

/// Ensures an arc's label is printable, since `dot` uses US-ASCII.
///
/// Equivalent to the private `Util.printableLabel`.
fn printable_label(label: i32) -> String {
    // Any ordinary ASCII character, except for `"` and `\`, is printed as the
    // character; everything else as a hexadecimal string.
    if (0x20..=0x7d).contains(&label) && label != 0x22 && label != 0x5c {
        char::from_u32(label as u32)
            .map(String::from)
            .unwrap_or_else(|| format!("0x{label:x}"))
    } else {
        format!("0x{label:x}")
    }
}
