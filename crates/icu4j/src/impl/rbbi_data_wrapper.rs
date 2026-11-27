// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.RBBIDataWrapper`: the compiled break-iterator rule data
//! (`.brk`), with its nested classes `RBBIStateTable` and `RBBIDataHeader`.

use rucene::error::{LuceneError, Result};
use rucene::util::print_stream::PrintStream;

use crate::jdk::{ByteBuffer, DataOutputStream};
use crate::r#impl::icu_binary::ICUBinary;
use crate::text::rule_based_break_iterator;
use crate::util::code_point_map::CodePointMap;
use crate::util::code_point_trie::{CodePointTrie, Type as TrieType};

/// `java.io.IOException(message)`.
fn io_exception(message: &str) -> LuceneError {
    LuceneError::from(std::io::Error::other(message.to_string()))
}

/// A state table of the rule data: the forward table or the safe reverse
/// table.
///
/// Equivalent to the public static nested class
/// `RBBIDataWrapper.RBBIStateTable`.
#[derive(Debug, Clone, Default)]
pub struct RBBIStateTable {
    /// `fNumStates`: number of states (rows) in this table.
    pub f_num_states: i32,
    /// `fRowLen`: length of a table row in bytes. Note mismatch with table
    /// data, which is `char[]`.
    pub f_row_len: i32,
    /// `fDictCategoriesStart`: char category number of the first dictionary
    /// char class, or the the largest category number + 1 if there are no
    /// dictionary categories.
    pub f_dict_categories_start: i32,
    /// `fLookAheadResultsSize`: size of run-time array required for holding
    /// look-ahead results; the number of look-ahead rules plus 1 (slot 0 is
    /// unused).
    pub f_look_ahead_results_size: i32,
    /// `fFlags`: option flags for this state table.
    pub f_flags: i32,
    /// `fTable`: linear array of next state values, accessed as
    /// `array[state, char_class]`.
    pub f_table: Vec<u16>,
}

impl RBBIStateTable {
    /// `fHeaderSize`: size of the table header, in bytes.
    ///
    /// Java declares it `public static int` without `final`; ICU never
    /// assigns it, and the port makes it a constant.
    pub const F_HEADER_SIZE: i32 = 20;

    /// `RBBIStateTable()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `get(ByteBuffer bytes, int length)`: reads a state table of `length`
    /// bytes; `None` for length 0.
    ///
    /// # Errors
    ///
    /// `IOException("Invalid RBBI state table length.")` for a length
    /// shorter than the header, and the buffer's `BufferUnderflowException`.
    pub fn get(bytes: &mut ByteBuffer, length: i32) -> Result<Option<RBBIStateTable>> {
        if length == 0 {
            return Ok(None);
        }
        if length < Self::F_HEADER_SIZE {
            return Err(io_exception("Invalid RBBI state table length."));
        }
        let mut this = RBBIStateTable::new();
        this.f_num_states = bytes.get_int()?;
        this.f_row_len = bytes.get_int()?;
        this.f_dict_categories_start = bytes.get_int()?;
        this.f_look_ahead_results_size = bytes.get_int()?;
        this.f_flags = bytes.get_int()?;
        let length_of_table = length - Self::F_HEADER_SIZE; // length in bytes.
        let use_8_bits =
            (this.f_flags & RBBIDataWrapper::RBBI_8BITS_ROWS) == RBBIDataWrapper::RBBI_8BITS_ROWS;
        if use_8_bits {
            let mut table = vec![0u16; length_of_table as usize];
            for t in table.iter_mut() {
                let b = bytes.get()?;
                *t = (0xff & b as i32) as u16;
            }
            this.f_table = table;
            ICUBinary::skip_bytes(bytes, length_of_table & 1)?;
        } else {
            this.f_table = ICUBinary::get_chars(bytes, length_of_table / 2, length_of_table & 1)?;
        }
        Ok(Some(this))
    }

    /// `put(DataOutputStream bytes)`: writes the table, padded to a
    /// multiple of 8 bytes; returns the bytes written.
    ///
    /// # Errors
    ///
    /// The `IOException` of the stream.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` when `fTable` is shorter than its
    /// declared dimensions.
    pub fn put(&self, bytes: &mut DataOutputStream<'_>) -> Result<i32> {
        bytes.write_int(self.f_num_states)?;
        bytes.write_int(self.f_row_len)?;
        bytes.write_int(self.f_dict_categories_start)?;
        bytes.write_int(self.f_look_ahead_results_size)?;
        bytes.write_int(self.f_flags)?;
        if (self.f_flags & RBBIDataWrapper::RBBI_8BITS_ROWS) == RBBIDataWrapper::RBBI_8BITS_ROWS {
            let table_len = self.f_row_len * self.f_num_states; // fRowLen is bytes.
            for i in 0..table_len {
                let b = (self.f_table[i as usize] & 0x00ff) as i8;
                bytes.write_byte(b as i32)?;
            }
        } else {
            let table_len = self.f_row_len * self.f_num_states / 2; // fRowLen is bytes.
            for i in 0..table_len {
                bytes.write_char(self.f_table[i as usize] as i32)?;
            }
        }
        let mut bytes_written = Self::F_HEADER_SIZE + self.f_row_len * self.f_num_states; // total bytes written,
                                                                                          // including the header.
        while bytes_written % 8 != 0 {
            bytes.write_byte(0)?;
            bytes_written += 1;
        }
        Ok(bytes_written)
    }
}

/// `RBBIStateTable.equals(Object other)`: equal headers and tables.
impl PartialEq for RBBIStateTable {
    fn eq(&self, other_st: &Self) -> bool {
        if std::ptr::eq(self, other_st) {
            return true;
        }
        if self.f_num_states != other_st.f_num_states {
            return false;
        }
        if self.f_row_len != other_st.f_row_len {
            return false;
        }
        if self.f_dict_categories_start != other_st.f_dict_categories_start {
            return false;
        }
        if self.f_look_ahead_results_size != other_st.f_look_ahead_results_size {
            return false;
        }
        if self.f_flags != other_st.f_flags {
            return false;
        }
        self.f_table == other_st.f_table
    }
}

/// The header of the rule data.
///
/// Equivalent to the public static nested class
/// `RBBIDataWrapper.RBBIDataHeader`.
#[derive(Debug, Clone, Default)]
pub struct RBBIDataHeader {
    pub(crate) f_magic: i32,
    pub(crate) f_format_version: [i8; 4],
    pub(crate) f_length: i32,
    /// `fCatCount`: the number of character categories.
    pub f_cat_count: i32,
    pub(crate) f_f_table: i32,
    pub(crate) f_f_table_len: i32,
    pub(crate) f_r_table: i32,
    pub(crate) f_r_table_len: i32,
    pub(crate) f_trie: i32,
    pub(crate) f_trie_len: i32,
    pub(crate) f_rule_source: i32,
    pub(crate) f_rule_source_len: i32,
    pub(crate) f_status_table: i32,
    pub(crate) f_status_table_len: i32,
}

impl RBBIDataHeader {
    /// `RBBIDataHeader()`: `fMagic = 0`, `fFormatVersion = new byte[4]`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// The compiled rule data of a `RuleBasedBreakIterator`.
///
/// Equivalent to the final class `com.ibm.icu.impl.RBBIDataWrapper`.
#[derive(Debug, Clone)]
pub struct RBBIDataWrapper {
    /// `fHeader`.
    pub f_header: RBBIDataHeader,
    /// `fFTable`: the forward state table, `null` when absent.
    pub f_f_table: Option<RBBIStateTable>,
    /// `fRTable`: the safe reverse state table, `null` when absent.
    pub f_r_table: Option<RBBIStateTable>,
    /// `fTrie`: the character-category trie.
    pub f_trie: CodePointTrie,
    /// `fRuleSource`: the (stripped) rule source.
    pub f_rule_source: Vec<u16>,
    /// `fStatusTable`: the rule status values.
    pub f_status_table: Vec<i32>,
}

impl RBBIDataWrapper {
    /// `DATA_FORMAT`: `"Brk "`.
    pub const DATA_FORMAT: i32 = 0x4272_6b20;
    /// `FORMAT_VERSION`: 6.0.0.0.
    pub const FORMAT_VERSION: i32 = 0x0600_0000;

    //
    // These fields are the ready-to-use compiled rule data, as
    //   read from the file.
    //
    /// `DH_SIZE`: size of the data header, in `int`s.
    pub const DH_SIZE: i32 = 20;
    /// `DH_MAGIC`.
    pub const DH_MAGIC: i32 = 0;
    /// `DH_FORMATVERSION`.
    pub const DH_FORMATVERSION: i32 = 1;
    /// `DH_LENGTH`.
    pub const DH_LENGTH: i32 = 2;
    /// `DH_CATCOUNT`.
    pub const DH_CATCOUNT: i32 = 3;
    /// `DH_FTABLE`.
    pub const DH_FTABLE: i32 = 4;
    /// `DH_FTABLELEN`.
    pub const DH_FTABLELEN: i32 = 5;
    /// `DH_RTABLE`.
    pub const DH_RTABLE: i32 = 6;
    /// `DH_RTABLELEN`.
    pub const DH_RTABLELEN: i32 = 7;
    /// `DH_TRIE`.
    pub const DH_TRIE: i32 = 8;
    /// `DH_TRIELEN`.
    pub const DH_TRIELEN: i32 = 9;
    /// `DH_RULESOURCE`.
    pub const DH_RULESOURCE: i32 = 10;
    /// `DH_RULESOURCELEN`.
    pub const DH_RULESOURCELEN: i32 = 11;
    /// `DH_STATUSTABLE`.
    pub const DH_STATUSTABLE: i32 = 12;
    /// `DH_STATUSTABLELEN`.
    pub const DH_STATUSTABLELEN: i32 = 13;

    // Index offsets to the fields in a state table row.
    //    Corresponds to struct RBBIStateTableRow in the C version.
    //
    /// `ACCEPTING`: state number to be returned when this state is accepting.
    pub const ACCEPTING: i32 = 0;
    /// `LOOKAHEAD`: look-ahead match backup.
    pub const LOOKAHEAD: i32 = 1;
    /// `TAGSIDX`: index of the rule status values.
    pub const TAGSIDX: i32 = 2;
    /// `NEXTSTATES`: index of the first next-state value.
    pub const NEXTSTATES: i32 = 3;

    /// `ACCEPTING_UNCONDITIONAL`: value constant for the `ACCEPTING` field of
    /// a state table row.
    pub const ACCEPTING_UNCONDITIONAL: i32 = 1;

    // Bit selectors for the "FLAGS" field of the state table header
    //     enum RBBIStateTableFlags in the C version.
    //
    /// `RBBI_LOOKAHEAD_HARD_BREAK`.
    pub const RBBI_LOOKAHEAD_HARD_BREAK: i32 = 1;
    /// `RBBI_BOF_REQUIRED`.
    pub const RBBI_BOF_REQUIRED: i32 = 2;
    /// `RBBI_8BITS_ROWS`.
    pub const RBBI_8BITS_ROWS: i32 = 4;

    /// `IsAcceptable.isDataVersionAcceptable(byte[] version)`: the format
    /// version must be 6.0.0.0 (Java's signed-byte arithmetic kept).
    fn is_data_version_acceptable(version: &[i8; 4]) -> bool {
        let int_version = ((version[0] as i32) << 24)
            .wrapping_add((version[1] as i32) << 16)
            .wrapping_add((version[2] as i32) << 8)
            .wrapping_add(version[3] as i32);
        int_version == Self::FORMAT_VERSION
    }

    /// `IS_ACCEPTABLE` as an `ICUBinary.Authenticate`.
    fn is_acceptable(version: &[u8; 4]) -> bool {
        Self::is_data_version_acceptable(&version.map(|b| b as i8))
    }

    /// `equals(RBBIStateTable left, RBBIStateTable right)`: both absent, or
    /// equal.
    pub fn equals(left: Option<&RBBIStateTable>, right: Option<&RBBIStateTable>) -> bool {
        match (left, right) {
            (None, None) => true,
            (Some(l), Some(r)) => std::ptr::eq(l, r) || l == r,
            _ => false,
        }
    }

    /// `getRowIndex(int state)`: the index in a state table of the row of
    /// `state`.
    pub fn get_row_index(&self, state: i32) -> i32 {
        state * (self.f_header.f_cat_count + Self::NEXTSTATES)
    }

    /// `fFTable`, dereferenced as Java does on each use.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the data has no forward table.
    pub fn f_table(&self) -> &RBBIStateTable {
        self.f_f_table
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: fFTable"))
    }

    /// `fRTable`, dereferenced as Java does on each use.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the data has no reverse table.
    pub fn r_table(&self) -> &RBBIStateTable {
        self.f_r_table
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: fRTable"))
    }

    /// `get(ByteBuffer bytes)`: reads compiled break-iterator rule data.
    ///
    /// # Errors
    ///
    /// `IOException` for a bad ICU header, a wrong magic number or format
    /// version, or corrupt offsets; the buffer's exceptions.
    pub fn get(mut bytes: ByteBuffer) -> Result<RBBIDataWrapper> {
        ICUBinary::read_header(&mut bytes, Self::DATA_FORMAT, Some(&Self::is_acceptable))?;

        // Read in the RBBI data header...
        let mut h = RBBIDataHeader::new();
        h.f_magic = bytes.get_int()?;
        h.f_format_version[0] = bytes.get()?;
        h.f_format_version[1] = bytes.get()?;
        h.f_format_version[2] = bytes.get()?;
        h.f_format_version[3] = bytes.get()?;
        h.f_length = bytes.get_int()?;
        h.f_cat_count = bytes.get_int()?;
        h.f_f_table = bytes.get_int()?;
        h.f_f_table_len = bytes.get_int()?;
        h.f_r_table = bytes.get_int()?;
        h.f_r_table_len = bytes.get_int()?;
        h.f_trie = bytes.get_int()?;
        h.f_trie_len = bytes.get_int()?;
        h.f_rule_source = bytes.get_int()?;
        h.f_rule_source_len = bytes.get_int()?;
        h.f_status_table = bytes.get_int()?;
        h.f_status_table_len = bytes.get_int()?;
        ICUBinary::skip_bytes(&mut bytes, 6 * 4)?; // uint32_t  fReserved[6];

        if h.f_magic != 0xb1a0 || !Self::is_data_version_acceptable(&h.f_format_version) {
            return Err(io_exception(
                "Break Iterator Rule Data Magic Number Incorrect, or unsupported data version.",
            ));
        }

        // Current position in the buffer.
        let mut pos = Self::DH_SIZE * 4; // offset of end of header, which has DH_SIZE fields, all int32_t (4 bytes)

        //
        // Read in the Forward state transition table as an array of shorts.
        //

        //   Quick-and-dirty sanity check that the data seems to be OK
        if h.f_f_table < pos || h.f_f_table > h.f_length {
            return Err(io_exception("Break iterator Rule data corrupt"));
        }

        //    Skip over any padding preceding this table
        ICUBinary::skip_bytes(&mut bytes, h.f_f_table - pos)?;
        pos = h.f_f_table;

        let f_f_table = RBBIStateTable::get(&mut bytes, h.f_f_table_len)?;
        pos += h.f_f_table_len;

        //
        // Read in the Reverse state table
        //

        // Skip over any padding in the file
        ICUBinary::skip_bytes(&mut bytes, h.f_r_table - pos)?;
        pos = h.f_r_table;

        // Create & fill the table itself.
        let f_r_table = RBBIStateTable::get(&mut bytes, h.f_r_table_len)?;
        pos += h.f_r_table_len;

        //
        // Unserialize the Character categories TRIE
        //     Because we can't be absolutely certain where the Trie deserialize will
        //     leave the buffer, leave position unchanged.
        //     The seek to the start of the next item following the TRIE will get us
        //     back in sync.
        //
        ICUBinary::skip_bytes(&mut bytes, h.f_trie - pos)?; // seek buffer from end of
        pos = h.f_trie; // previous section to the start of the trie

        bytes.mark(); // Mark position of start of TRIE in the input
                      //  and tell Java to keep the mark valid so long
                      //  as we don't go more than 100 bytes past the
                      //  past the end of the TRIE.

        let f_trie = CodePointTrie::from_binary(Some(TrieType::Fast), None, &mut bytes)?; // Deserialize the TRIE, leaving buffer
                                                                                          //  at an unknown position, preceding the
                                                                                          //  padding between TRIE and following section.

        bytes.reset()?; // Move buffer back to marked position at
                        //   the start of the serialized TRIE.  Now our
                        //   "pos" variable and the buffer are in
                        //   agreement.

        //
        // Read the Rule Status Table
        //
        if pos > h.f_status_table {
            return Err(io_exception("Break iterator Rule data corrupt"));
        }
        ICUBinary::skip_bytes(&mut bytes, h.f_status_table - pos)?;
        pos = h.f_status_table;
        let f_status_table = ICUBinary::get_ints(
            &mut bytes,
            h.f_status_table_len / 4,
            h.f_status_table_len & 3,
        )?;
        pos += h.f_status_table_len;

        //
        // Put the break rule source into a String
        //
        if pos > h.f_rule_source {
            return Err(io_exception("Break iterator Rule data corrupt"));
        }
        ICUBinary::skip_bytes(&mut bytes, h.f_rule_source - pos)?;
        let rule_bytes = ICUBinary::get_bytes(&mut bytes, h.f_rule_source_len, 0)?;
        let rule_bytes: Vec<u8> = rule_bytes.into_iter().map(|b| b as u8).collect();
        let f_rule_source: Vec<u16> = String::from_utf8_lossy(&rule_bytes)
            .encode_utf16()
            .collect();

        let this = RBBIDataWrapper {
            f_header: h,
            f_f_table,
            f_r_table,
            f_trie,
            f_rule_source,
            f_status_table,
        };
        if let Some(env) = rule_based_break_iterator::f_debug_env() {
            if env.contains("data") {
                let out = rucene::util::print_stream::WriterPrintStream::stdout();
                this.dump(&out)?;
            }
        }
        Ok(this)
    }

    /// `dump(java.io.PrintStream out)`: prints the tables, the character
    /// categories and the rule source.
    ///
    /// # Errors
    ///
    /// `NullPointerException` when the data has no forward table.
    pub fn dump(&self, out: &dyn PrintStream) -> Result<()> {
        if self.f_f_table.is_none() {
            // There is no table. Fail early for testing purposes.
            return Err(LuceneError::null_pointer());
        }
        out.println("RBBI Data Wrapper dump ...");
        out.println("");
        out.println("Forward State Table");
        self.dump_table(out, self.f_f_table.as_ref());
        out.println("Reverse State Table");
        self.dump_table(out, self.f_r_table.as_ref());

        self.dump_char_categories(out);
        out.println(&format!(
            "Source Rules: {}",
            String::from_utf16_lossy(&self.f_rule_source)
        ));
        Ok(())
    }

    /// `intToString(int n, int width)`: `n` right-aligned in `width`
    /// characters.
    pub fn int_to_string(n: i32, width: i32) -> String {
        let mut dest = n.to_string();
        while (dest.encode_utf16().count() as i32) < width {
            dest.insert(0, ' ');
        }
        dest
    }

    /// `charToString(char n, int width)`: `n` right-aligned in `width`
    /// characters.
    pub fn char_to_string(n: u16, width: i32) -> String {
        let mut dest = String::from_utf16_lossy(&[n]);
        let mut len = 1;
        while len < width {
            dest.insert(0, ' ');
            len += 1;
        }
        dest
    }

    /// `intToHexString(int n, int width)`: `Integer.toHexString(n)`
    /// right-aligned in `width` characters.
    pub fn int_to_hex_string(n: i32, width: i32) -> String {
        let mut dest = format!("{:x}", n as u32);
        while (dest.len() as i32) < width {
            dest.insert(0, ' ');
        }
        dest
    }

    /// `dumpTable(java.io.PrintStream out, RBBIStateTable table)`.
    fn dump_table(&self, out: &dyn PrintStream, table: Option<&RBBIStateTable>) {
        match table {
            Some(table) if !table.f_table.is_empty() => {
                let mut header = String::from(" Row  Acc Look  Tag");
                for n in 0..self.f_header.f_cat_count {
                    header.push_str(&Self::int_to_string(n, 5));
                }
                out.println(&header);
                for _ in 0..header.len() {
                    out.print("-");
                }
                out.println("");
                for state in 0..table.f_num_states {
                    self.dump_row(out, table, state);
                }
                out.println("");
            }
            _ => out.println("  -- null -- "),
        }
    }

    /// `dumpRow(java.io.PrintStream out, RBBIStateTable table, char state)`.
    fn dump_row(&self, out: &dyn PrintStream, table: &RBBIStateTable, state: i32) {
        let mut dest = String::with_capacity((self.f_header.f_cat_count * 5 + 20) as usize);
        dest.push_str(&Self::int_to_string(state, 4));
        let row = self.get_row_index(state) as usize;
        let t = &table.f_table;
        if t[row + Self::ACCEPTING as usize] != 0 {
            dest.push_str(&Self::int_to_string(
                t[row + Self::ACCEPTING as usize] as i32,
                5,
            ));
        } else {
            dest.push_str("     ");
        }
        if t[row + Self::LOOKAHEAD as usize] != 0 {
            dest.push_str(&Self::int_to_string(
                t[row + Self::LOOKAHEAD as usize] as i32,
                5,
            ));
        } else {
            dest.push_str("     ");
        }
        dest.push_str(&Self::int_to_string(
            t[row + Self::TAGSIDX as usize] as i32,
            5,
        ));

        for col in 0..self.f_header.f_cat_count as usize {
            dest.push_str(&Self::int_to_string(
                t[row + Self::NEXTSTATES as usize + col] as i32,
                5,
            ));
        }

        out.println(&dest);
    }

    /// `dumpCharCategories(java.io.PrintStream out)`.
    fn dump_char_categories(&self, out: &dyn PrintStream) {
        let n = self.f_header.f_cat_count;
        let mut cat_strings: Vec<String> = vec![String::new(); (n + 1) as usize];
        let mut range_start = 0;
        let mut range_end = 0;
        let mut last_cat: i32 = -1;
        let mut last_newline = vec![0usize; (n + 1) as usize];

        out.println("\nCharacter Categories");
        out.println("--------------------");
        for char32 in 0..=0x10ffff {
            let category = self.f_trie.get(char32);
            if category < 0 || category > self.f_header.f_cat_count {
                out.println(&format!(
                    "Error, bad category {:x} for char {:x}",
                    category as u32, char32
                ));
                break;
            }
            if category == last_cat {
                range_end = char32;
            } else {
                if last_cat >= 0 {
                    let lc = last_cat as usize;
                    if cat_strings[lc].len() > last_newline[lc] + 70 {
                        last_newline[lc] = cat_strings[lc].len() + 10;
                        cat_strings[lc].push_str("\n       ");
                    }

                    cat_strings[lc].push_str(&format!(" {:x}", range_start));
                    if range_end != range_start {
                        cat_strings[lc].push_str(&format!("-{:x}", range_end));
                    }
                }
                last_cat = category;
                range_start = char32;
                range_end = char32;
            }
        }
        // `catStrings[lastCat]` with `lastCat == -1` throws in Java.
        let lc = usize::try_from(last_cat)
            .unwrap_or_else(|_| crate::jdk::array_index_out_of_bounds(last_cat));
        cat_strings[lc].push_str(&format!(" {:x}", range_start));
        if range_end != range_start {
            cat_strings[lc].push_str(&format!("-{:x}", range_end));
        }

        for category in 0..=self.f_header.f_cat_count {
            out.println(&format!(
                "{}  {}",
                Self::int_to_string(category, 5),
                cat_strings[category as usize]
            ));
        }
        out.println("");
    }
}
