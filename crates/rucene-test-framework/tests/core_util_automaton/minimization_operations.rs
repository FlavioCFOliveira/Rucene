//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/MinimizationOperations.java`:
//! operations for minimizing automata (Hopcroft's algorithm), a test helper
//! of the `org.apache.lucene.util.automaton` tests.
//!
//! `StateList` and `StateListNode` form a doubly linked list whose nodes
//! Java compares by identity (`sn.sl == active[j][c]`); the port keeps them
//! in arenas and compares their indices. Index 0 of the list arena is
//! `StateList.EMPTY`.

use std::collections::VecDeque;

use crate::core_support::det;
use rucene::internal::hppc::{IntArrayList, IntHashSet};
use rucene::util::automaton::{
    Automaton, Operations, Transition, TransitionAccessor, MAX_CODE_POINT, MIN_CODE_POINT,
};

/// `MinimizationOperations`.
pub struct MinimizationOperations;

/// `StateList.EMPTY`: the list every `active` slot starts on.
const EMPTY: usize = 0;

/// `StateList`.
#[derive(Default)]
struct StateList {
    size: i32,
    first: Option<usize>,
    last: Option<usize>,
}

/// `StateListNode`.
struct StateListNode {
    q: i32,
    next: Option<usize>,
    prev: Option<usize>,
    sl: usize,
}

/// The arenas of the `StateList`s and `StateListNode`s of one `minimize`.
struct Lists {
    lists: Vec<StateList>,
    nodes: Vec<StateListNode>,
}

impl Lists {
    /// `new StateList()`.
    fn new_list(&mut self) -> usize {
        self.lists.push(StateList::default());
        self.lists.len() - 1
    }

    /// `StateList.add(q)`: `new StateListNode(q, this)`.
    fn add(&mut self, sl: usize, q: i32) -> usize {
        assert!(sl != EMPTY);
        let node = self.nodes.len();
        let list = &mut self.lists[sl];
        let prev = if list.size == 0 {
            list.first = Some(node);
            list.last = Some(node);
            None
        } else {
            let last = list.last.expect("a non-empty list has a last node");
            list.last = Some(node);
            Some(last)
        };
        list.size += 1;
        if let Some(prev) = prev {
            self.nodes[prev].next = Some(node);
        }
        self.nodes.push(StateListNode {
            q,
            next: None,
            prev,
            sl,
        });
        node
    }

    /// `StateListNode.remove()`.
    fn remove(&mut self, node: usize) {
        let (sl, next, prev) = {
            let n = &self.nodes[node];
            (n.sl, n.next, n.prev)
        };
        self.lists[sl].size -= 1;
        if self.lists[sl].first == Some(node) {
            self.lists[sl].first = next;
        } else {
            self.nodes[prev.expect("a non-first node has a prev")].next = next;
        }
        if self.lists[sl].last == Some(node) {
            self.lists[sl].last = prev;
        } else {
            self.nodes[next.expect("a non-last node has a next")].prev = prev;
        }
    }
}

impl MinimizationOperations {
    /// `minimize(a, determinizeWorkLimit)`: minimizes (and determinizes if
    /// not already deterministic) the given automaton using Hopcroft's
    /// algorithm.
    ///
    /// # Panics
    ///
    /// `TooComplexToDeterminizeException` when determinizing needs more than
    /// `determinize_work_limit` effort.
    pub fn minimize(a: &Automaton, determinize_work_limit: i32) -> Automaton {
        if a.get_num_states() == 0 || (!a.is_accept(0) && a.get_num_transitions(0) == 0) {
            // Fastmatch for common case
            return Automaton::new();
        }
        let a = det(Operations::determinize(a, determinize_work_limit));
        // a.writeDot("adet");
        if a.get_num_transitions(0) == 1 {
            let mut t = Transition::new();
            a.get_transition(0, 0, &mut t);
            if t.dest == 0 && t.min == MIN_CODE_POINT && t.max == MAX_CODE_POINT {
                // Accepts all strings
                return a;
            }
        }
        let a = Operations::totalize(&a);
        // a.writeDot("atot");

        // initialize data structures
        let sigma = a.get_start_points();
        let sigma_len = sigma.len();
        let states_len = a.get_num_states() as usize;
        let mut reverse: Vec<Vec<Option<IntArrayList>>> = (0..states_len)
            .map(|_| (0..sigma_len).map(|_| None).collect())
            .collect();
        let mut partition: Vec<IntHashSet> = Vec::with_capacity(states_len);
        let mut splitblock: Vec<IntArrayList> = Vec::with_capacity(states_len);
        let mut block = vec![0i32; states_len];
        let mut lists = Lists {
            lists: vec![StateList::default()],
            nodes: Vec::new(),
        };
        let mut active = vec![vec![EMPTY; sigma_len]; states_len];
        let mut active2: Vec<Vec<Option<usize>>> = vec![vec![None; sigma_len]; states_len];
        let mut pending: VecDeque<(i32, i32)> = VecDeque::new();
        let mut pending2 = vec![false; sigma_len * states_len];
        let mut split = vec![false; states_len];
        let mut refine = vec![false; states_len];
        let mut refine2 = vec![false; states_len];
        for _ in 0..states_len {
            splitblock.push(IntArrayList::new());
            partition.push(IntHashSet::new());
        }

        // find initial partition and reverse edges
        let mut transition = Transition::new();
        for q in 0..states_len {
            let j = if a.is_accept(q as i32) { 0 } else { 1 };
            partition[j].add(q as i32);
            block[q] = j as i32;
            transition.source = q as i32;
            transition.transition_upto = -1;
            for (x, &label) in sigma.iter().enumerate() {
                let dest = a.next(&mut transition, label) as usize;
                let r = &mut reverse[dest];
                if r[x].is_none() {
                    r[x] = Some(IntArrayList::new());
                }
                r[x].as_mut().expect("just set").add(q as i32);
            }
        }

        // initialize active sets
        for j in 0..=1usize {
            for x in 0..sigma_len {
                let members: Vec<i32> = partition[j].iter().map(|c| c.value).collect();
                for q in members {
                    if reverse[q as usize][x].is_some() {
                        let mut state_list = active[j][x];
                        if state_list == EMPTY {
                            state_list = lists.new_list();
                            active[j][x] = state_list;
                        }
                        active2[q as usize][x] = Some(lists.add(state_list, q));
                    }
                }
            }
        }

        // initialize pending
        for x in 0..sigma_len {
            let j = if lists.lists[active[0][x]].size <= lists.lists[active[1][x]].size {
                0
            } else {
                1
            };
            pending.push_back((j, x as i32));
            pending2[x * states_len + j as usize] = true;
        }

        // process pending until fixed point
        let mut k = 2usize;
        // System.out.println("start min");
        while let Some((p, x)) = pending.pop_front() {
            let (p, x) = (p as usize, x as usize);
            // System.out.println("    pop n1=" + ip.n1 + " n2=" + ip.n2);
            pending2[x * states_len + p] = false;
            // find states that need to be split off their blocks
            let mut m = lists.lists[active[p][x]].first;
            while let Some(node) = m {
                let mq = lists.nodes[node].q as usize;
                if let Some(r) = &reverse[mq][x] {
                    for i_cursor in r.iter() {
                        let i = i_cursor.value as usize;
                        if !split[i] {
                            split[i] = true;
                            let j = block[i] as usize;
                            splitblock[j].add(i as i32);
                            if !refine2[j] {
                                refine2[j] = true;
                                refine[j] = true;
                            }
                        }
                    }
                }
                m = lists.nodes[node].next;
            }

            // refine blocks
            for j in 0..states_len {
                if !refine[j] {
                    continue;
                }
                let sb: Vec<i32> = splitblock[j].iter().map(|c| c.value).collect();
                if (sb.len() as i32) < partition[j].size() {
                    for &s in &sb {
                        partition[j].remove(s);
                        partition[k].add(s);
                        block[s as usize] = k as i32;
                        for c in 0..sigma_len {
                            if let Some(sn) = active2[s as usize][c] {
                                if lists.nodes[sn].sl == active[j][c] {
                                    lists.remove(sn);
                                    let mut state_list = active[k][c];
                                    if state_list == EMPTY {
                                        state_list = lists.new_list();
                                        active[k][c] = state_list;
                                    }
                                    active2[s as usize][c] = Some(lists.add(state_list, s));
                                }
                            }
                        }
                    }
                    // update pending
                    for c in 0..sigma_len {
                        let aj = lists.lists[active[j][c]].size;
                        let ak = lists.lists[active[k][c]].size;
                        let ofs = c * states_len;
                        if !pending2[ofs + j] && 0 < aj && aj <= ak {
                            pending2[ofs + j] = true;
                            pending.push_back((j as i32, c as i32));
                        } else {
                            pending2[ofs + k] = true;
                            pending.push_back((k as i32, c as i32));
                        }
                    }
                    k += 1;
                }
                refine2[j] = false;
                for &s in &sb {
                    split[s as usize] = false;
                }
                splitblock[j].clear();
            }
            refine.iter_mut().for_each(|b| *b = false);
        }

        let mut result = Automaton::new();
        let mut t = Transition::new();
        // System.out.println("  k=" + k);

        // make a new state for each equivalence class, set initial state
        let mut state_map = vec![0i32; states_len];
        let mut state_rep = vec![0i32; k];

        result.create_state();

        // System.out.println("min: k=" + k);
        for n in 0..k {
            // System.out.println("    n=" + n);
            let is_initial = partition[n].contains(0);
            let new_state = if is_initial {
                // System.out.println("    isInitial!");
                0
            } else {
                result.create_state()
            };
            // System.out.println("  newState=" + newState);
            let members: Vec<i32> = partition[n].iter().map(|c| c.value).collect();
            for q in members {
                state_map[q as usize] = new_state;
                // System.out.println("      q=" + q + " isAccept?=" + a.isAccept(q));
                result.set_accept(new_state, a.is_accept(q));
                state_rep[new_state as usize] = q; // select representative
            }
        }

        // build transitions and set acceptance
        for n in 0..k {
            let num_transitions = a.init_transition(state_rep[n], &mut t);
            for _ in 0..num_transitions {
                a.get_next_transition(&mut t);
                // System.out.println("  add trans");
                result.add_transition_range(n as i32, state_map[t.dest as usize], t.min, t.max);
            }
        }
        result.finish_state();
        // System.out.println(result.getNumStates() + " states");

        Operations::remove_dead_states(&result)
    }
}
