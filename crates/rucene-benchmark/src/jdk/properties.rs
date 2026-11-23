//! `java.util.Properties`, with the iteration order of the
//! `ConcurrentHashMap` that backs it in JDK 25.
//!
//! The order is observable in the benchmark: `DocMaker` adds one document
//! field per property in `entrySet()` order, which fixes the field numbers
//! the index records. The map reproduces everything that decides that order
//! for a single writer: `new ConcurrentHashMap<>(8)` (a table of 16 built at
//! the first insertion), the spreading `(h ^ (h >>> 16)) & HASH_BITS` over
//! `String.hashCode()`, tail insertion in a list bin, head insertion in a
//! tree bin, `treeifyBin`'s pre-sizing (`tryPresize(n << 1)`) below 64 bins,
//! the resize threshold `n - (n >>> 2)`, and `transfer`'s split of each bin
//! (the `lastRun` tail kept, the nodes before it prepended in reverse; tree
//! bins split in order).

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::java_string_hash_code;

/// `ConcurrentHashMap.TREEIFY_THRESHOLD`.
const TREEIFY_THRESHOLD: usize = 8;
/// `ConcurrentHashMap.UNTREEIFY_THRESHOLD`.
const UNTREEIFY_THRESHOLD: usize = 6;
/// `ConcurrentHashMap.MIN_TREEIFY_CAPACITY`.
const MIN_TREEIFY_CAPACITY: usize = 64;
/// `ConcurrentHashMap.HASH_BITS`.
const HASH_BITS: i32 = 0x7fff_ffff;

/// One node: `(hash, key, value)`.
#[derive(Clone, Debug)]
struct Node {
    hash: i32,
    key: String,
    value: String,
}

/// A bin: a linked list (`tree == false`) or a `TreeBin` (`tree == true`),
/// whose `first` list is `nodes`.
#[derive(Clone, Debug, Default)]
struct Bin {
    nodes: Vec<Node>,
    tree: bool,
}

/// Equivalent to `java.util.Properties` holding `String` keys and values.
#[derive(Clone, Debug)]
pub struct Properties {
    table: Vec<Bin>,
    /// `sizeCtl`: the initial capacity before the table exists, the resize
    /// threshold after.
    size_ctl: usize,
    count: usize,
}

impl Default for Properties {
    fn default() -> Self {
        Self::new()
    }
}

/// `ConcurrentHashMap.spread(h)`.
fn spread(h: i32) -> i32 {
    (h ^ ((h as u32) >> 16) as i32) & HASH_BITS
}

/// `ConcurrentHashMap.tableSizeFor(c)`.
fn table_size_for(c: usize) -> usize {
    c.max(1).next_power_of_two()
}

impl Properties {
    /// `new Properties()`: `new ConcurrentHashMap<>(8)`, whose `sizeCtl` is
    /// `tableSizeFor((int) (1.0 + 8 / 0.75f))` = 16.
    pub fn new() -> Self {
        Properties {
            table: Vec::new(),
            size_ctl: table_size_for((1.0 + 8.0 / 0.75f32 as f64) as usize),
            count: 0,
        }
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.count
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// `getProperty(key)`.
    pub fn get_property(&self, key: &str) -> Option<&str> {
        if self.table.is_empty() {
            return None;
        }
        let hash = spread(java_string_hash_code(key));
        let n = self.table.len();
        self.table[(hash as usize) & (n - 1)]
            .nodes
            .iter()
            .find(|node| node.hash == hash && node.key == key)
            .map(|node| node.value.as_str())
    }

    /// `getProperty(key, defaultValue)`.
    pub fn get_property_or<'a>(
        &'a self,
        key: &str,
        default_value: Option<&'a str>,
    ) -> Option<&'a str> {
        self.get_property(key).or(default_value)
    }

    /// `get(key) != null`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get_property(key).is_some()
    }

    /// `setProperty(key, value)` (`put`): the previous value, if any.
    pub fn set_property(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Option<String> {
        let key = key.into();
        let value = value.into();
        let hash = spread(java_string_hash_code(&key));
        if self.table.is_empty() {
            // initTable
            let n = self.size_ctl.max(1);
            self.table = vec![Bin::default(); n];
            self.size_ctl = n - (n >> 2);
        }
        let n = self.table.len();
        let i = (hash as usize) & (n - 1);
        let bin = &mut self.table[i];
        let mut bin_count = 0usize;
        if bin.nodes.is_empty() {
            bin.nodes.push(Node { hash, key, value });
        } else if bin.tree {
            if let Some(node) = bin
                .nodes
                .iter_mut()
                .find(|node| node.hash == hash && node.key == key)
            {
                return Some(std::mem::replace(&mut node.value, value));
            }
            // putTreeVal: `first = x = new TreeNode(h, k, v, f, xp)`.
            bin.nodes.insert(0, Node { hash, key, value });
            bin_count = 2;
        } else {
            if let Some(node) = bin
                .nodes
                .iter_mut()
                .find(|node| node.hash == hash && node.key == key)
            {
                return Some(std::mem::replace(&mut node.value, value));
            }
            bin_count = bin.nodes.len();
            bin.nodes.push(Node { hash, key, value });
        }
        if bin_count >= TREEIFY_THRESHOLD {
            self.treeify_bin(i);
        }
        self.add_count();
        None
    }

    /// `treeifyBin(tab, index)`.
    fn treeify_bin(&mut self, index: usize) {
        let n = self.table.len();
        if n < MIN_TREEIFY_CAPACITY {
            self.try_presize(n << 1);
        } else if !self.table[index].nodes.is_empty() && !self.table[index].tree {
            self.table[index].tree = true;
        }
    }

    /// `tryPresize(size)` with the table already built.
    fn try_presize(&mut self, size: usize) {
        let c = table_size_for(size + (size >> 1) + 1);
        loop {
            let sc = self.size_ctl;
            if c <= sc {
                break;
            }
            self.transfer();
        }
    }

    /// `addCount(1, check)`.
    fn add_count(&mut self) {
        self.count += 1;
        while self.count >= self.size_ctl {
            self.transfer();
        }
    }

    /// `transfer(tab, null)` by a single thread.
    fn transfer(&mut self) {
        let n = self.table.len();
        let mut next = vec![Bin::default(); n << 1];
        let old = std::mem::take(&mut self.table);
        for (i, bin) in old.into_iter().enumerate().rev() {
            if bin.nodes.is_empty() {
                continue;
            }
            if bin.tree {
                let mut lo = Vec::new();
                let mut hi = Vec::new();
                for node in &bin.nodes {
                    if (node.hash as usize) & n == 0 {
                        lo.push(node.clone());
                    } else {
                        hi.push(node.clone());
                    }
                }
                let lo_tree = if lo.len() <= UNTREEIFY_THRESHOLD {
                    false
                } else {
                    !hi.is_empty() || bin.tree
                };
                let hi_tree = if hi.len() <= UNTREEIFY_THRESHOLD {
                    false
                } else {
                    !lo.is_empty() || bin.tree
                };
                next[i] = Bin {
                    nodes: lo,
                    tree: lo_tree,
                };
                next[i + n] = Bin {
                    nodes: hi,
                    tree: hi_tree,
                };
            } else {
                let nodes = bin.nodes;
                let mut run_bit = (nodes[0].hash as usize) & n;
                let mut last_run = 0;
                for (p, node) in nodes.iter().enumerate().skip(1) {
                    let b = (node.hash as usize) & n;
                    if b != run_bit {
                        run_bit = b;
                        last_run = p;
                    }
                }
                let tail: Vec<Node> = nodes[last_run..].to_vec();
                let (mut ln, mut hn) = if run_bit == 0 {
                    (tail, Vec::new())
                } else {
                    (Vec::new(), tail)
                };
                for node in &nodes[..last_run] {
                    if (node.hash as usize) & n == 0 {
                        ln.insert(0, node.clone());
                    } else {
                        hn.insert(0, node.clone());
                    }
                }
                next[i] = Bin {
                    nodes: ln,
                    tree: false,
                };
                next[i + n] = Bin {
                    nodes: hn,
                    tree: false,
                };
            }
        }
        self.table = next;
        let n2 = n << 1;
        self.size_ctl = (n << 1) - (n >> 1);
        debug_assert_eq!(self.size_ctl, n2 - (n2 >> 2));
    }

    /// `entrySet()` in iteration order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.table
            .iter()
            .flat_map(|bin| bin.nodes.iter())
            .map(|node| (node.key.as_str(), node.value.as_str()))
    }

    /// `keySet()` in iteration order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries().map(|(k, _)| k)
    }

    /// `load(Reader)`: the `key=value` lines of `text`, each `put` in
    /// order.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed `\uxxxx` escape.
    pub fn load(&mut self, text: &str) -> Result<()> {
        let units: Vec<u16> = text.encode_utf16().collect();
        for (key, value) in load_pairs(&units)? {
            self.set_property(key, value);
        }
        Ok(())
    }
}

/// `Properties.load0`: the key-value pairs of a `.properties` text, in
/// the order Java `put`s them.
///
/// The line reader and `loadConvert` are copied from
/// `rucene_queryparser::jdk::resource_bundle`, which keeps them private.
fn load_pairs(text: &[u16]) -> Result<Vec<(String, String)>> {
    let mut map = Vec::new();
    let mut reader = LineReader { text, off: 0 };
    while let Some(line) = reader.read_line() {
        let limit = line.len();
        let mut key_len = 0usize;
        let mut value_start = limit;
        let mut has_sep = false;
        let mut preceding_backslash = false;
        while key_len < limit {
            let c = line[key_len];
            // need check if escaped.
            if (c == u16::from(b'=') || c == u16::from(b':')) && !preceding_backslash {
                value_start = key_len + 1;
                has_sep = true;
                break;
            } else if (c == u16::from(b' ') || c == u16::from(b'\t') || c == 0x0C)
                && !preceding_backslash
            {
                value_start = key_len + 1;
                break;
            }
            if c == u16::from(b'\\') {
                preceding_backslash = !preceding_backslash;
            } else {
                preceding_backslash = false;
            }
            key_len += 1;
        }
        while value_start < limit {
            let c = line[value_start];
            if c != u16::from(b' ') && c != u16::from(b'\t') && c != 0x0C {
                if !has_sep && (c == u16::from(b'=') || c == u16::from(b':')) {
                    has_sep = true;
                } else {
                    break;
                }
            }
            value_start += 1;
        }
        let key = load_convert(&line[..key_len])?;
        let value = load_convert(&line[value_start..limit])?;
        map.push((key, value));
    }
    Ok(map)
}

/// `Properties.LineReader` over a decoded text.
struct LineReader<'a> {
    text: &'a [u16],
    off: usize,
}

impl LineReader<'_> {
    /// `readLine()`: the next logical line, with its leading whitespace,
    /// comment lines and line continuations removed; `None` at the end.
    fn read_line(&mut self) -> Option<Vec<u16>> {
        let mut line: Vec<u16> = Vec::new();
        let mut skip_white_space = true;
        let mut appended_line_begin = false;
        let mut preceding_backslash = false;
        loop {
            if self.off >= self.text.len() {
                if line.is_empty() {
                    return None;
                }
                if preceding_backslash {
                    line.pop();
                }
                return Some(line);
            }
            let c = self.text[self.off];
            self.off += 1;
            if skip_white_space {
                if c == u16::from(b' ') || c == u16::from(b'\t') || c == 0x0C {
                    continue;
                }
                if !appended_line_begin && (c == u16::from(b'\r') || c == u16::from(b'\n')) {
                    continue;
                }
                skip_white_space = false;
                appended_line_begin = false;
            }
            if line.is_empty() && (c == u16::from(b'#') || c == u16::from(b'!')) {
                // Comment, quickly consume the rest of the line, resume on
                // line-break and backslash.
                loop {
                    if self.off >= self.text.len() {
                        return None;
                    }
                    let c = self.text[self.off];
                    self.off += 1;
                    if c == u16::from(b'\r') || c == u16::from(b'\n') {
                        break;
                    }
                }
                skip_white_space = true;
                continue;
            }
            if c != u16::from(b'\n') && c != u16::from(b'\r') {
                line.push(c);
                preceding_backslash = if c == u16::from(b'\\') {
                    !preceding_backslash
                } else {
                    false
                };
            } else {
                // reached EOL
                if line.is_empty() {
                    skip_white_space = true;
                    continue;
                }
                if self.off >= self.text.len() {
                    if preceding_backslash {
                        line.pop();
                    }
                    return Some(line);
                }
                if preceding_backslash {
                    // backslash at EOL is not part of the line
                    line.pop();
                    // skip leading whitespace characters in the following
                    // line
                    skip_white_space = true;
                    appended_line_begin = true;
                    preceding_backslash = false;
                    // take care not to include any subsequent \n
                    if c == u16::from(b'\r') && self.text[self.off] == u16::from(b'\n') {
                        self.off += 1;
                    }
                } else {
                    return Some(line);
                }
            }
        }
    }
}

/// `Properties.loadConvert`: decodes `\uxxxx` escapes and the escaped
/// special characters.
fn load_convert(input: &[u16]) -> Result<String> {
    let malformed = || LuceneError::illegal_argument("Malformed \\uxxxx encoding.");
    let mut out: Vec<u16> = Vec::with_capacity(input.len());
    let end = input.len();
    let mut off = 0usize;
    while off < end {
        let mut a_char = input[off];
        off += 1;
        if a_char == u16::from(b'\\') {
            // `LineReader::readLine` excludes unescaped backslashes at the
            // end of the line.
            a_char = input[off];
            off += 1;
            if a_char == u16::from(b'u') {
                // Read the xxxx
                if off + 4 > end {
                    return Err(malformed());
                }
                let mut value: u32 = 0;
                for _ in 0..4 {
                    let c = input[off];
                    off += 1;
                    let digit = match c {
                        0x30..=0x39 => u32::from(c) - 0x30,
                        0x61..=0x66 => 10 + u32::from(c) - 0x61,
                        0x41..=0x46 => 10 + u32::from(c) - 0x41,
                        _ => return Err(malformed()),
                    };
                    value = (value << 4) + digit;
                }
                out.push(value as u16);
            } else {
                let converted = match a_char {
                    0x74 => u16::from(b'\t'),
                    0x72 => u16::from(b'\r'),
                    0x6E => u16::from(b'\n'),
                    0x66 => 0x0C,
                    other => other,
                };
                out.push(converted);
            }
        } else {
            out.push(a_char);
        }
    }
    Ok(String::from_utf16_lossy(&out))
}
