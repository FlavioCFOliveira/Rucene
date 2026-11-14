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

//! The types declared in `TernaryTree.java`, of the Java package
//! `org.apache.lucene.analysis.compound.hyphenation`.

use std::io::Write;

use super::char_vector::CharVector;

/// allocation size for arrays
///
/// Equivalent to `TernaryTree.BLOCK_SIZE`.
pub(crate) const BLOCK_SIZE: usize = 2048;

/// Ternary Search Tree.
///
/// A ternary search tree is a hybrid between a binary tree and a digital
/// search tree (trie). Keys are limited to strings. A data value of type char
/// is stored in each leaf node. It can be used as an index (or pointer) to
/// the data. Branches that only contain one key are compressed to one node by
/// storing a pointer to the trailer substring of the key. This class is
/// intended to serve as base class or helper class to implement Dictionary
/// collections or the like. Ternary trees have some nice properties as the
/// following: the tree can be traversed in sorted order, partial matches
/// (wildcard) can be implemented, retrieval of all keys within a given
/// distance from the target, etc. The storage requirements are higher than a
/// binary tree but a lot less than a trie. Performance is comparable with a
/// hash table, sometimes it outperforms a hash function (most of the time can
/// determine a miss faster than a hash).
///
/// The main purpose of this java port is to serve as a base for implementing
/// TeX's hyphenation algorithm (see The TeXBook, appendix H). Each language
/// requires from 5000 to 15000 hyphenation patterns which will be keys in
/// this tree. The strings patterns are usually small (from 2 to 5
/// characters), but each char in the tree is stored in a node. Thus memory
/// usage is the main concern. We will sacrifice 'elegance' to keep memory
/// requirements to the minimum. Using java's char type as pointer (yes, I
/// know pointer it is a forbidden word in java) we can keep the size of the
/// node to be just 8 bytes (3 pointers and the data char). This gives room
/// for about 65000 nodes. In my tests the english patterns took 7694 nodes
/// and the german patterns 10055 nodes, so I think we are safe.
///
/// All said, this is a map with strings as keys and char as value. Pretty
/// limited!. It can be extended to a general map by using the string
/// representation of an object and using the char value as an index to an
/// array that contains the object values. This class has been taken from the
/// Apache FOP project (<http://xmlgraphics.apache.org/fop/>). They have been
/// slightly modified.
///
/// Keys are Java `String`s and `char[]`s, held as UTF-16 units; the `char`
/// node pointers keep Java's 16-bit wrap-around. An index past the end of an
/// array panics where Java throws `ArrayIndexOutOfBoundsException`.
///
/// Equivalent to `org.apache.lucene.analysis.compound.hyphenation.TernaryTree`;
/// [`Clone`] is its `clone()` (copies of the four node arrays and of `kv`,
/// the same `root`, `freenode` and `length`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TernaryTree {
    // We use 4 arrays to represent a node. I guess I should have created a
    // proper node class, but somehow Knuth's pascal code made me forget we
    // now have a portable language with virtual memory management and
    // automatic garbage collection! And now is kind of late, furthermore, if
    // it ain't broken, don't fix it.
    /// Pointer to low branch and to rest of the key when it is stored
    /// directly in this node, we don't have unions in java!
    pub(crate) lo: Vec<u16>,
    /// Pointer to high branch.
    pub(crate) hi: Vec<u16>,
    /// Pointer to equal branch and to data when this node is a string
    /// terminator.
    pub(crate) eq: Vec<u16>,
    /// The character stored in this node: splitchar. Two special values are
    /// reserved: 0x0000 as string terminator, 0xFFFF to indicate that the
    /// branch starting at this node is compressed.
    ///
    /// This shouldn't be a problem if we give the usual semantics to strings
    /// since 0xFFFF is guaranteed not to be an Unicode character.
    pub(crate) sc: Vec<u16>,
    /// This vector holds the trailing of the keys when the branch is
    /// compressed.
    pub(crate) kv: CharVector,
    pub(crate) root: u16,
    pub(crate) freenode: u16,
    /// number of items in tree
    pub(crate) length: i32,
}

impl TernaryTree {
    /// Equivalent to the package-private `TernaryTree()`.
    pub(crate) fn new() -> Self {
        let mut tree = Self {
            lo: Vec::new(),
            hi: Vec::new(),
            eq: Vec::new(),
            sc: Vec::new(),
            kv: CharVector::new(),
            root: 0,
            freenode: 0,
            length: 0,
        };
        tree.init();
        tree
    }

    /// Equivalent to the protected `TernaryTree.init()`.
    pub(crate) fn init(&mut self) {
        self.root = 0;
        self.freenode = 1;
        self.length = 0;
        self.lo = vec![0; BLOCK_SIZE];
        self.hi = vec![0; BLOCK_SIZE];
        self.eq = vec![0; BLOCK_SIZE];
        self.sc = vec![0; BLOCK_SIZE];
        self.kv = CharVector::new();
    }

    /// Branches are initially compressed, needing one node per key plus the
    /// size of the string key. They are decompressed as needed when another
    /// key with same prefix is inserted. This saves a lot of space, specially
    /// for long keys.
    ///
    /// Equivalent to `TernaryTree.insert(String key, char val)`.
    pub fn insert(&mut self, key: &[u16], val: u16) {
        // make sure we have enough room in the arrays
        let mut len = key.len() + 1; // maximum number of nodes that may be generated
        if self.freenode as usize + len > self.eq.len() {
            self.redim_node_arrays(self.eq.len() + BLOCK_SIZE);
        }
        let mut strkey = vec![0u16; len];
        len -= 1;
        strkey[..len].copy_from_slice(&key[..len]);
        strkey[len] = 0;
        self.root = self.insert_node(self.root, &strkey, 0, val);
    }

    /// Equivalent to `TernaryTree.insert(char[] key, int start, char val)`:
    /// as in Java, the room check measures `key` from index 0, not `start`.
    pub fn insert_chars(&mut self, key: &[u16], start: usize, val: u16) {
        let len = Self::strlen(key) + 1;
        if self.freenode as usize + len > self.eq.len() {
            self.redim_node_arrays(self.eq.len() + BLOCK_SIZE);
        }
        self.root = self.insert_node(self.root, key, start, val);
    }

    /// The actual insertion function, recursive version.
    ///
    /// Equivalent to the private `TernaryTree.insert(char p, char[] key, int start, char val)`.
    fn insert_node(&mut self, mut p: u16, key: &[u16], start: usize, val: u16) -> u16 {
        let len = Self::strlen_from(key, start);
        if p == 0 {
            // this means there is no branch, this node will start a new branch.
            // Instead of doing that, we store the key somewhere else and create
            // only one node with a pointer to the key
            p = self.freenode;
            self.freenode = self.freenode.wrapping_add(1);
            let pi = p as usize;
            self.eq[pi] = val; // holds data
            self.length += 1;
            self.hi[pi] = 0;
            if len > 0 {
                self.sc[pi] = 0xFFFF; // indicates branch is compressed
                self.lo[pi] = self.kv.alloc(len + 1) as u16; // use 'lo' to hold pointer to key
                Self::strcpy(self.kv.get_array_mut(), self.lo[pi] as usize, key, start);
            } else {
                self.sc[pi] = 0;
                self.lo[pi] = 0;
            }
            return p;
        }

        let pi = p as usize;
        if self.sc[pi] == 0xFFFF {
            // branch is compressed: need to decompress
            // this will generate garbage in the external key array
            // but we can do some garbage collection later
            let pp = self.freenode;
            self.freenode = self.freenode.wrapping_add(1);
            let ppi = pp as usize;
            self.lo[ppi] = self.lo[pi]; // previous pointer to key
            self.eq[ppi] = self.eq[pi]; // previous pointer to data
            self.lo[pi] = 0;
            if len > 0 {
                self.sc[pi] = self.kv.get(self.lo[ppi] as usize);
                self.eq[pi] = pp;
                self.lo[ppi] = self.lo[ppi].wrapping_add(1);
                if self.kv.get(self.lo[ppi] as usize) == 0 {
                    // key completly decompressed leaving garbage in key array
                    self.lo[ppi] = 0;
                    self.sc[ppi] = 0;
                    self.hi[ppi] = 0;
                } else {
                    // we only got first char of key, rest is still there
                    self.sc[ppi] = 0xFFFF;
                }
            } else {
                // In this case we can save a node by swapping the new node
                // with the compressed node
                self.sc[ppi] = 0xFFFF;
                self.hi[pi] = pp;
                self.sc[pi] = 0;
                self.eq[pi] = val;
                self.length += 1;
                return p;
            }
        }
        let s = key[start];
        if s < self.sc[pi] {
            self.lo[pi] = self.insert_node(self.lo[pi], key, start, val);
        } else if s == self.sc[pi] {
            if s != 0 {
                self.eq[pi] = self.insert_node(self.eq[pi], key, start + 1, val);
            } else {
                // key already in tree, overwrite data
                self.eq[pi] = val;
            }
        } else {
            self.hi[pi] = self.insert_node(self.hi[pi], key, start, val);
        }
        p
    }

    /// Compares 2 null terminated char arrays
    ///
    /// Equivalent to `TernaryTree.strcmp(char[] a, int startA, char[] b, int startB)`.
    pub fn strcmp(a: &[u16], mut start_a: usize, b: &[u16], mut start_b: usize) -> i32 {
        while a[start_a] == b[start_b] {
            if a[start_a] == 0 {
                return 0;
            }
            start_a += 1;
            start_b += 1;
        }
        a[start_a] as i32 - b[start_b] as i32
    }

    /// Compares a string with null terminated char array
    ///
    /// Equivalent to `TernaryTree.strcmp(String str, char[] a, int start)`.
    pub fn strcmp_str(str: &[u16], a: &[u16], start: usize) -> i32 {
        let len = str.len();
        let mut i = 0;
        while i < len {
            let d = str[i] as i32 - a[start + i] as i32;
            if d != 0 {
                return d;
            }
            if a[start + i] == 0 {
                return d;
            }
            i += 1;
        }
        if a[start + i] != 0 {
            return -(a[start + i] as i32);
        }
        0
    }

    /// Equivalent to `TernaryTree.strcpy(char[] dst, int di, char[] src, int si)`.
    pub fn strcpy(dst: &mut [u16], mut di: usize, src: &[u16], mut si: usize) {
        while src[si] != 0 {
            dst[di] = src[si];
            di += 1;
            si += 1;
        }
        dst[di] = 0;
    }

    /// Equivalent to `TernaryTree.strlen(char[] a, int start)`.
    pub fn strlen_from(a: &[u16], start: usize) -> usize {
        let mut len = 0;
        let mut i = start;
        while i < a.len() && a[i] != 0 {
            len += 1;
            i += 1;
        }
        len
    }

    /// Equivalent to `TernaryTree.strlen(char[] a)`.
    pub fn strlen(a: &[u16]) -> usize {
        Self::strlen_from(a, 0)
    }

    /// Equivalent to `TernaryTree.find(String key)`: the value of `key`, or
    /// -1.
    pub fn find(&self, key: &[u16]) -> i32 {
        let len = key.len();
        let mut strkey = vec![0u16; len + 1];
        strkey[..len].copy_from_slice(key);
        strkey[len] = 0;

        self.find_chars(&strkey, 0)
    }

    /// Equivalent to `TernaryTree.find(char[] key, int start)`.
    pub fn find_chars(&self, key: &[u16], start: usize) -> i32 {
        let mut p = self.root;
        let mut i = start;

        while p != 0 {
            let pi = p as usize;
            if self.sc[pi] == 0xFFFF {
                if Self::strcmp(key, i, self.kv.get_array(), self.lo[pi] as usize) == 0 {
                    return self.eq[pi] as i32;
                } else {
                    return -1;
                }
            }
            let c = key[i];
            let d = c as i32 - self.sc[pi] as i32;
            if d == 0 {
                if c == 0 {
                    return self.eq[pi] as i32;
                }
                i += 1;
                p = self.eq[pi];
            } else if d < 0 {
                p = self.lo[pi];
            } else {
                p = self.hi[pi];
            }
        }
        -1
    }

    /// Equivalent to `TernaryTree.knows(String key)`.
    pub fn knows(&self, key: &[u16]) -> bool {
        self.find(key) >= 0
    }

    /// redimension the arrays
    ///
    /// Equivalent to the private `TernaryTree.redimNodeArrays(int)`.
    fn redim_node_arrays(&mut self, newsize: usize) {
        let len = newsize.min(self.lo.len());
        for array in [&mut self.lo, &mut self.hi, &mut self.eq, &mut self.sc] {
            let mut na = vec![0u16; newsize];
            na[..len].copy_from_slice(&array[..len]);
            *array = na;
        }
    }

    /// Equivalent to `TernaryTree.size()`.
    pub fn size(&self) -> i32 {
        self.length
    }

    /// Recursively insert the median first and then the median of the lower
    /// and upper halves, and so on in order to get a balanced tree. The array
    /// of keys is assumed to be sorted in ascending order.
    ///
    /// Equivalent to the protected `TernaryTree.insertBalanced(String[], char[], int, int)`;
    /// a `None` key panics where Java throws `NullPointerException`.
    pub(crate) fn insert_balanced(
        &mut self,
        k: &[Option<Vec<u16>>],
        v: &[u16],
        offset: usize,
        n: i32,
    ) {
        if n < 1 {
            return;
        }
        let m = (n >> 1) as usize;

        let key = k[m + offset]
            .as_ref()
            .expect("INVARIANT: Java's insert(null) throws NullPointerException");
        self.insert(key, v[m + offset]);
        self.insert_balanced(k, v, offset, m as i32);

        self.insert_balanced(k, v, offset + m + 1, n - m as i32 - 1);
    }

    /// Balance the tree for best search performance
    ///
    /// Equivalent to `TernaryTree.balance()`.
    pub fn balance(&mut self) {
        let n = self.length;
        let mut k: Vec<Option<Vec<u16>>> = vec![None; n.max(0) as usize];
        let mut v = vec![0u16; n.max(0) as usize];
        {
            let mut iter = TernaryTreeIterator::new(self);
            let mut i = 0;
            while iter.has_more_elements() {
                v[i] = iter.get_value();
                k[i] = iter.next_element();
                i += 1;
            }
        }
        self.init();
        self.insert_balanced(&k, &v, 0, n);

        // With uniform letter distribution sc[root] should be around 'm'
    }

    /// Each node stores a character (splitchar) which is part of some
    /// key(s). In a compressed branch (one that only contain a single string
    /// key) the trailer of the key which is not already in nodes is stored
    /// externally in the kv array. As items are inserted, key substrings
    /// decrease. Some substrings may completely disappear when the whole
    /// branch is totally decompressed. The tree is traversed to find the key
    /// substrings actually used. In addition, duplicate substrings are
    /// removed using a map (implemented with a TernaryTree!).
    ///
    /// Equivalent to `TernaryTree.trimToSize()`.
    pub fn trim_to_size(&mut self) {
        // first balance the tree for best performance
        self.balance();

        // redimension the node arrays
        self.redim_node_arrays(self.freenode as usize);

        // ok, compact kv array
        let mut kx = CharVector::new();
        kx.alloc(1);
        let mut map = TernaryTree::new();
        self.compact(&mut kx, &mut map, self.root);
        self.kv = kx;
        self.kv.trim_to_size();
    }

    /// Equivalent to the private `TernaryTree.compact(CharVector, TernaryTree, char)`.
    fn compact(&mut self, kx: &mut CharVector, map: &mut TernaryTree, p: u16) {
        if p == 0 {
            return;
        }
        let pi = p as usize;
        if self.sc[pi] == 0xFFFF {
            let mut k = map.find_chars(self.kv.get_array(), self.lo[pi] as usize);
            if k < 0 {
                k = kx.alloc(Self::strlen_from(self.kv.get_array(), self.lo[pi] as usize) + 1)
                    as i32;
                Self::strcpy(
                    kx.get_array_mut(),
                    k as usize,
                    self.kv.get_array(),
                    self.lo[pi] as usize,
                );
                map.insert_chars(kx.get_array(), k as usize, k as u16);
            }
            self.lo[pi] = k as u16;
        } else {
            self.compact(kx, map, self.lo[pi]);
            if self.sc[pi] != 0 {
                self.compact(kx, map, self.eq[pi]);
            }
            self.compact(kx, map, self.hi[pi]);
        }
    }

    /// Equivalent to `TernaryTree.keys()`.
    pub fn keys(&self) -> TernaryTreeIterator<'_> {
        TernaryTreeIterator::new(self)
    }

    /// Equivalent to `TernaryTree.printStats(PrintStream)`; like a
    /// `PrintStream`, it ignores the errors of `out`.
    pub fn print_stats(&self, out: &mut dyn Write) {
        let _ = writeln!(out, "Number of keys = {}", self.length);
        let _ = writeln!(out, "Node count = {}", self.freenode);
        let _ = writeln!(out, "Key Array length = {}", self.kv.length());
    }
}

/// Equivalent to the private class
/// `org.apache.lucene.analysis.compound.hyphenation.TernaryTree$Iterator$Item`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Item {
    parent: u16,
    child: u16,
}

impl Item {
    /// Equivalent to `Item(char p, char c)`.
    fn new(p: u16, c: u16) -> Self {
        Self {
            parent: p,
            child: c,
        }
    }
}

/// Enumeration over TST keys
///
/// Equivalent to the inner class
/// `org.apache.lucene.analysis.compound.hyphenation.TernaryTree$Iterator`
/// (`@lucene.internal`), over the tree it borrows; its node stack, a Java
/// `ArrayDeque` used with `push` and `pop`, is a `Vec` used as a stack.
/// [`Iterator`] is `hasMoreElements()` then `nextElement()`.
#[derive(Debug, Clone)]
pub struct TernaryTreeIterator<'a> {
    tree: &'a TernaryTree,
    /// current node index
    cur: i32,
    /// current key
    curkey: Option<Vec<u16>>,
    /// Node stack
    ns: Vec<Item>,
    /// key stack implemented with a StringBuilder
    ks: Vec<u16>,
}

impl<'a> TernaryTreeIterator<'a> {
    /// Equivalent to `TernaryTree.this.new Iterator()`.
    pub fn new(tree: &'a TernaryTree) -> Self {
        let mut iter = Self {
            tree,
            cur: -1,
            curkey: None,
            ns: Vec::new(),
            ks: Vec::new(),
        };
        iter.rewind();
        iter
    }

    /// Equivalent to `Iterator.rewind()`.
    pub fn rewind(&mut self) {
        self.ns.clear();
        self.ks.clear();
        self.cur = self.tree.root as i32;
        self.run();
    }

    /// Equivalent to `Iterator.nextElement()`: the current key (`None` for
    /// Java's `null`), then a move to the next one.
    pub fn next_element(&mut self) -> Option<Vec<u16>> {
        let res = self.curkey.clone();
        self.cur = self.up();
        self.run();
        res
    }

    /// Equivalent to `Iterator.getValue()`.
    pub fn get_value(&self) -> u16 {
        if self.cur >= 0 {
            return self.tree.eq[self.cur as usize];
        }
        0
    }

    /// Equivalent to `Iterator.hasMoreElements()`.
    pub fn has_more_elements(&self) -> bool {
        self.cur != -1
    }

    /// traverse upwards
    ///
    /// Equivalent to the private `Iterator.up()`.
    fn up(&mut self) -> i32 {
        let t = self.tree;
        let mut res = 0;

        if self.ns.is_empty() {
            return -1;
        }

        if self.cur != 0 && t.sc[self.cur as usize] == 0 {
            return t.lo[self.cur as usize] as i32;
        }

        let mut climb = true;

        while climb {
            let mut i = self
                .ns
                .pop()
                .expect("INVARIANT: the stack is checked non-empty before each pop");
            i.child = i.child.wrapping_add(1);
            let parent = i.parent as usize;
            match i.child {
                1 => {
                    if t.sc[parent] != 0 {
                        res = t.eq[parent] as i32;
                        self.ns.push(i);
                        self.ks.push(t.sc[parent]);
                    } else {
                        i.child = i.child.wrapping_add(1);
                        self.ns.push(i);
                        res = t.hi[parent] as i32;
                    }
                    climb = false;
                }
                2 => {
                    res = t.hi[parent] as i32;
                    self.ns.push(i);
                    if !self.ks.is_empty() {
                        self.ks.pop(); // pop
                    }
                    climb = false;
                }
                _ => {
                    if self.ns.is_empty() {
                        return -1;
                    }
                    climb = true;
                }
            }
        }
        res
    }

    /// traverse the tree to find next key
    ///
    /// Equivalent to the private `Iterator.run()`.
    fn run(&mut self) -> i32 {
        let t = self.tree;
        if self.cur == -1 {
            return -1;
        }

        let mut leaf = false;
        loop {
            // first go down on low branch until leaf or compressed branch
            while self.cur != 0 {
                let cur = self.cur as usize;
                if t.sc[cur] == 0xFFFF {
                    leaf = true;
                    break;
                }
                self.ns.push(Item::new(self.cur as u16, 0));
                if t.sc[cur] == 0 {
                    leaf = true;
                    break;
                }
                self.cur = t.lo[cur] as i32;
            }
            if leaf {
                break;
            }
            // nothing found, go up one node and try again
            self.cur = self.up();
            if self.cur == -1 {
                return -1;
            }
        }
        // The current node should be a data node and
        // the key should be in the key stack (at least partially)
        let mut buf = self.ks.clone();
        let cur = self.cur as usize;
        if t.sc[cur] == 0xFFFF {
            let mut p = t.lo[cur] as usize;
            while t.kv.get(p) != 0 {
                buf.push(t.kv.get(p));
                p += 1;
            }
        }
        self.curkey = Some(buf);
        0
    }
}

impl Iterator for TernaryTreeIterator<'_> {
    type Item = Vec<u16>;

    /// `hasMoreElements()` then `nextElement()`.
    fn next(&mut self) -> Option<Vec<u16>> {
        if self.has_more_elements() {
            self.next_element()
        } else {
            None
        }
    }
}
