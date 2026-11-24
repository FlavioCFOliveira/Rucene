//! `java.io.StreamTokenizer` (JDK 25), over a text already in memory.

/// `TT_EOF`.
pub const TT_EOF: i32 = -1;
/// `TT_EOL`.
pub const TT_EOL: i32 = '\n' as i32;
/// `TT_NUMBER`.
pub const TT_NUMBER: i32 = -2;
/// `TT_WORD`.
pub const TT_WORD: i32 = -3;
/// `TT_NOTHING`.
const TT_NOTHING: i32 = -4;

const NEED_CHAR: i32 = i32::MAX;
const SKIP_LF: i32 = i32::MAX - 1;

const CT_WHITESPACE: u8 = 1;
const CT_DIGIT: u8 = 2;
const CT_ALPHA: u8 = 4;
const CT_QUOTE: u8 = 8;
const CT_COMMENT: u8 = 16;

/// Equivalent to `java.io.StreamTokenizer` built with
/// `new StreamTokenizer(Reader)` over the UTF-16 code units of a string.
///
/// The `slashSlashComments`, `slashStarComments` and `lowerCaseMode`
/// switches, which the benchmark never turns on, keep their `false`
/// defaults.
#[derive(Debug)]
pub struct StreamTokenizer {
    input: Vec<u16>,
    pos: usize,
    ctype: [u8; 256],
    peekc: i32,
    pushed_back: bool,
    eol_is_significant: bool,
    lineno: i32,
    /// `ttype`.
    pub ttype: i32,
    /// `sval`.
    pub sval: Option<String>,
    /// `nval`.
    pub nval: f64,
}

impl StreamTokenizer {
    /// `new StreamTokenizer(new StringReader(text))`.
    pub fn new(text: &str) -> Self {
        let mut st = StreamTokenizer {
            input: text.encode_utf16().collect(),
            pos: 0,
            ctype: [0; 256],
            peekc: NEED_CHAR,
            pushed_back: false,
            eol_is_significant: false,
            lineno: 1,
            ttype: TT_NOTHING,
            sval: None,
            nval: 0.0,
        };
        st.word_chars('a' as i32, 'z' as i32);
        st.word_chars('A' as i32, 'Z' as i32);
        st.word_chars(128 + 32, 255);
        st.whitespace_chars(0, ' ' as i32);
        st.comment_char('/' as i32);
        st.quote_char('"' as i32);
        st.quote_char('\'' as i32);
        st.parse_numbers();
        st
    }

    fn read(&mut self) -> i32 {
        if self.pos < self.input.len() {
            let c = self.input[self.pos];
            self.pos += 1;
            i32::from(c)
        } else {
            -1
        }
    }

    /// `wordChars(low, hi)`.
    pub fn word_chars(&mut self, low: i32, hi: i32) {
        let low = low.max(0);
        let hi = hi.min(255);
        for i in low..=hi {
            self.ctype[i as usize] |= CT_ALPHA;
        }
    }

    /// `whitespaceChars(low, hi)`.
    pub fn whitespace_chars(&mut self, low: i32, hi: i32) {
        let low = low.max(0);
        let hi = hi.min(255);
        for i in low..=hi {
            self.ctype[i as usize] = CT_WHITESPACE;
        }
    }

    /// `ordinaryChar(ch)`.
    pub fn ordinary_char(&mut self, ch: i32) {
        if (0..256).contains(&ch) {
            self.ctype[ch as usize] = 0;
        }
    }

    /// `commentChar(ch)`.
    pub fn comment_char(&mut self, ch: i32) {
        if (0..256).contains(&ch) {
            self.ctype[ch as usize] = CT_COMMENT;
        }
    }

    /// `quoteChar(ch)`.
    pub fn quote_char(&mut self, ch: i32) {
        if (0..256).contains(&ch) {
            self.ctype[ch as usize] = CT_QUOTE;
        }
    }

    /// `parseNumbers()`.
    pub fn parse_numbers(&mut self) {
        for c in &mut self.ctype['0' as usize..='9' as usize] {
            *c |= CT_DIGIT;
        }
        self.ctype['.' as usize] |= CT_DIGIT;
        self.ctype['-' as usize] |= CT_DIGIT;
    }

    /// `eolIsSignificant(flag)`.
    pub fn eol_is_significant(&mut self, flag: bool) {
        self.eol_is_significant = flag;
    }

    /// `lineno()`.
    pub fn lineno(&self) -> i32 {
        self.lineno
    }

    /// `pushBack()`.
    pub fn push_back(&mut self) {
        if self.ttype != TT_NOTHING {
            self.pushed_back = true;
        }
    }

    fn class_of(&self, c: i32) -> u8 {
        if c < 256 {
            self.ctype[c as usize]
        } else {
            CT_ALPHA
        }
    }

    /// `nextToken()`.
    pub fn next_token(&mut self) -> i32 {
        if self.pushed_back {
            self.pushed_back = false;
            return self.ttype;
        }
        self.sval = None;

        let mut c = self.peekc;
        if c < 0 {
            c = NEED_CHAR;
        }
        if c == SKIP_LF {
            c = self.read();
            if c < 0 {
                self.ttype = TT_EOF;
                return self.ttype;
            }
            if c == '\n' as i32 {
                c = NEED_CHAR;
            }
        }
        if c == NEED_CHAR {
            c = self.read();
            if c < 0 {
                self.ttype = TT_EOF;
                return self.ttype;
            }
        }
        self.ttype = c;
        self.peekc = NEED_CHAR;

        let mut ctype = self.class_of(c);
        while ctype & CT_WHITESPACE != 0 {
            if c == '\r' as i32 {
                self.lineno += 1;
                if self.eol_is_significant {
                    self.peekc = SKIP_LF;
                    self.ttype = TT_EOL;
                    return self.ttype;
                }
                c = self.read();
                if c == '\n' as i32 {
                    c = self.read();
                }
            } else {
                if c == '\n' as i32 {
                    self.lineno += 1;
                    if self.eol_is_significant {
                        self.ttype = TT_EOL;
                        return self.ttype;
                    }
                }
                c = self.read();
            }
            if c < 0 {
                self.ttype = TT_EOF;
                return self.ttype;
            }
            ctype = self.class_of(c);
        }

        if ctype & CT_DIGIT != 0 {
            let mut neg = false;
            if c == '-' as i32 {
                c = self.read();
                if c != '.' as i32 && !('0' as i32..='9' as i32).contains(&c) {
                    self.peekc = c;
                    self.ttype = '-' as i32;
                    return self.ttype;
                }
                neg = true;
            }
            let mut v = 0.0f64;
            let mut decexp = 0;
            let mut seendot = 0;
            loop {
                if c == '.' as i32 && seendot == 0 {
                    seendot = 1;
                } else if ('0' as i32..='9' as i32).contains(&c) {
                    v = v * 10.0 + f64::from(c - '0' as i32);
                    decexp += seendot;
                } else {
                    break;
                }
                c = self.read();
            }
            self.peekc = c;
            if decexp != 0 {
                let mut denom = 10.0f64;
                decexp -= 1;
                while decexp > 0 {
                    denom *= 10.0;
                    decexp -= 1;
                }
                v /= denom;
            }
            self.nval = if neg { -v } else { v };
            self.ttype = TT_NUMBER;
            return self.ttype;
        }

        if ctype & CT_ALPHA != 0 {
            let mut buf: Vec<u16> = Vec::new();
            loop {
                buf.push(c as u16);
                c = self.read();
                let ct = if c < 0 {
                    CT_WHITESPACE
                } else {
                    self.class_of(c)
                };
                if ct & (CT_ALPHA | CT_DIGIT) == 0 {
                    break;
                }
            }
            self.peekc = c;
            self.sval = Some(String::from_utf16_lossy(&buf));
            self.ttype = TT_WORD;
            return self.ttype;
        }

        if ctype & CT_QUOTE != 0 {
            self.ttype = c;
            let mut buf: Vec<u16> = Vec::new();
            let mut d = self.read();
            while d >= 0 && d != self.ttype && d != '\n' as i32 && d != '\r' as i32 {
                if d == '\\' as i32 {
                    c = self.read();
                    let first = c;
                    if ('0' as i32..='7' as i32).contains(&c) {
                        c -= '0' as i32;
                        let mut c2 = self.read();
                        if ('0' as i32..='7' as i32).contains(&c2) {
                            c = (c << 3) + (c2 - '0' as i32);
                            c2 = self.read();
                            if ('0' as i32..='7' as i32).contains(&c2) && first <= '3' as i32 {
                                c = (c << 3) + (c2 - '0' as i32);
                                d = self.read();
                            } else {
                                d = c2;
                            }
                        } else {
                            d = c2;
                        }
                    } else {
                        c = match c {
                            0x61 => 0x7,
                            0x62 => 0x8,
                            0x66 => 0xC,
                            0x6E => '\n' as i32,
                            0x72 => '\r' as i32,
                            0x74 => '\t' as i32,
                            0x76 => 0xB,
                            other => other,
                        };
                        d = self.read();
                    }
                } else {
                    c = d;
                    d = self.read();
                }
                buf.push(c as u16);
            }
            self.peekc = if d == self.ttype { NEED_CHAR } else { d };
            self.sval = Some(String::from_utf16_lossy(&buf));
            return self.ttype;
        }

        if ctype & CT_COMMENT != 0 {
            loop {
                c = self.read();
                if c == '\n' as i32 || c == '\r' as i32 || c < 0 {
                    break;
                }
            }
            self.peekc = c;
            return self.next_token();
        }

        self.ttype = c;
        self.ttype
    }
}

impl std::fmt::Display for StreamTokenizer {
    /// `toString()`: `"Token[" + ret + "], line " + LINENO`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ret = match self.ttype {
            TT_EOF => "EOF".to_string(),
            TT_EOL => "EOL".to_string(),
            TT_WORD => self.sval.clone().unwrap_or_else(|| "null".to_string()),
            TT_NUMBER => format!("n={}", crate::jdk::double_to_string(self.nval)),
            TT_NOTHING => "NOTHING".to_string(),
            t => {
                if t < 256 && self.ctype[t as usize] & CT_QUOTE != 0 {
                    self.sval.clone().unwrap_or_else(|| "null".to_string())
                } else {
                    let ch = char::from_u32(t as u32).unwrap_or('\u{FFFD}');
                    format!("'{ch}'")
                }
            }
        };
        write!(f, "Token[{ret}], line {}", self.lineno)
    }
}
