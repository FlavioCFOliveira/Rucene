//! Analysis factories ported from `org.apache.lucene.analysis`.
//!
//! A factory builds one analysis component from a string-keyed configuration —
//! the form a schema file or a query parameter carries — so an analysis chain
//! can be assembled without naming Rust types.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use crate::analysis::{CharFilter, TokenFilterLogic, TokenizerLogic};
use crate::error::{LuceneError, Result};
use crate::util::extra::Version;

/// Configuration key naming the Lucene version a factory should behave as.
///
/// Equivalent to `AbstractAnalysisFactory.LUCENE_MATCH_VERSION_PARAM`.
pub const LUCENE_MATCH_VERSION_PARAM: &str = "luceneMatchVersion";
/// Configuration key naming the implementing class, consumed on construction.
pub const CLASS_NAME: &str = "class";
/// Configuration key naming the SPI name, consumed on construction.
pub const SPI_NAME: &str = "name";

/// The configuration a factory was built from, and the accessors that read it.
///
/// Equivalent to `org.apache.lucene.analysis.AbstractAnalysisFactory`.
///
/// **Divergence from Lucene 10.5.0.** Java makes this an abstract class every
/// factory extends, inheriting the `require`/`get` accessors. Rust has no
/// implementation inheritance, so the port is a struct each factory holds. The
/// accessors consume the argument they read, exactly as Java's do, so a
/// leftover argument still signals a misconfiguration.
#[derive(Clone, Debug)]
pub struct AbstractAnalysisFactory {
    original_args: HashMap<String, String>,
    lucene_match_version: Version,
    /// Whether the caller passed `luceneMatchVersion` explicitly.
    ///
    /// Equivalent to `AbstractAnalysisFactory.isExplicitLuceneMatchVersion`.
    is_explicit_lucene_match_version: bool,
}

impl AbstractAnalysisFactory {
    /// Builds the configuration holder from `args`, consuming the version, the
    /// class name and the SPI name.
    ///
    /// Equivalent to `AbstractAnalysisFactory(Map<String, String>)`.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let original_args = args.clone();
        let (lucene_match_version, explicit_version) = match args.remove(LUCENE_MATCH_VERSION_PARAM)
        {
            Some(version) => (Version::parse_leniently(&version)?, true),
            None => (Version::LATEST, false),
        };
        args.remove(CLASS_NAME);
        args.remove(SPI_NAME);
        Ok(Self {
            original_args,
            lucene_match_version,
            is_explicit_lucene_match_version: explicit_version,
        })
    }

    /// Returns the arguments the factory was built from, before consumption.
    ///
    /// Equivalent to `getOriginalArgs()`.
    pub fn get_original_args(&self) -> &HashMap<String, String> {
        &self.original_args
    }

    /// Returns the version the factory should behave as.
    ///
    /// Equivalent to `getLuceneMatchVersion()`.
    pub fn get_lucene_match_version(&self) -> Version {
        self.lucene_match_version
    }

    /// Returns whether the caller passed an explicit `luceneMatchVersion`.
    ///
    /// Equivalent to `AbstractAnalysisFactory.isExplicitLuceneMatchVersion()`.
    pub fn is_explicit_lucene_match_version(&self) -> bool {
        self.is_explicit_lucene_match_version
    }

    /// Records whether the caller passed an explicit `luceneMatchVersion`.
    ///
    /// Equivalent to `AbstractAnalysisFactory.setExplicitLuceneMatchVersion(boolean)`.
    pub fn set_explicit_lucene_match_version(&mut self, explicit: bool) {
        self.is_explicit_lucene_match_version = explicit;
    }

    /// Returns the error a no-argument factory constructor must raise.
    ///
    /// Equivalent to `AbstractAnalysisFactory.defaultCtorException()`, which
    /// every subclass's no-argument constructor throws: a factory is only ever
    /// built from its argument map.
    pub fn default_ctor_exception() -> LuceneError {
        LuceneError::UnsupportedOperation(
            "Analysis factories cannot be instantiated without arguments. Use applicable factory \
             methods of TokenizerFactory, CharFilterFactory, or TokenFilterFactory."
                .to_string(),
        )
    }

    /// Consumes an optional single-character argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getChar(Map, String, char)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the value is not exactly
    /// one character long.
    pub fn get_char(
        args: &mut HashMap<String, String>,
        name: &str,
        default_value: char,
    ) -> Result<char> {
        match args.remove(name) {
            None => Ok(default_value),
            Some(s) => {
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Ok(c),
                    _ => Err(LuceneError::IllegalArgument(format!(
                        "{name} should be a char. \"{s}\" is invalid"
                    ))),
                }
            }
        }
    }

    /// Consumes a required single-character argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.requireChar(Map, String)`, which
    /// takes the first character of the required value.
    pub fn require_char(args: &mut HashMap<String, String>, name: &str) -> Result<char> {
        let value = Self::require(args, name)?;
        value.chars().next().ok_or_else(|| {
            LuceneError::IllegalArgument(format!("{name} should be a char but is empty"))
        })
    }

    /// Splits `list` on `separator`, honouring a backslash escape.
    ///
    /// Equivalent to `AbstractAnalysisFactory.splitAt(char, String)`. A
    /// separator preceded by a backslash is part of the item, and the backslash
    /// is removed.
    pub fn split_at(separator: char, list: Option<&str>) -> Vec<String> {
        let Some(list) = list else {
            return Vec::new();
        };
        let mut result = Vec::new();
        let mut current = String::new();
        let mut escaped = false;
        for c in list.chars() {
            if escaped {
                // A backslash only escapes the separator; anything else keeps
                // both characters, as Java's regex does.
                if c != separator {
                    current.push('\\');
                }
                current.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == separator {
                result.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        }
        if escaped {
            current.push('\\');
        }
        result.push(current);
        result
    }

    /// Splits a comma-separated list of file names.
    ///
    /// Equivalent to `AbstractAnalysisFactory.splitFileNames(String)`.
    pub fn split_file_names(file_names: Option<&str>) -> Vec<String> {
        Self::split_at(',', file_names)
    }

    /// Returns the argument as a set of comma-separated values.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getClassArg()` in its general
    /// form: the value is split on commas and each item trimmed.
    pub fn get_class_arg(&self, name: &str) -> Option<Vec<String>> {
        self.original_args.get(name).map(|value| {
            Self::split_at(',', Some(value))
                .into_iter()
                .map(|item| item.trim().to_string())
                .collect()
        })
    }

    /// Reads the data-carrying lines of one resource.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getLines(ResourceLoader, String)`,
    /// which is `WordlistLoader.getLines(loader.openResource(resource),
    /// StandardCharsets.UTF_8)` — so comment lines (`#`) and blank lines are
    /// dropped, a leading byte-order mark is removed, and every kept line is
    /// trimmed. See [`crate::analysis::wordlist_loader::get_lines`], which does
    /// that work and documents the order the four rules apply in.
    ///
    /// # Errors
    ///
    /// Returns whatever opening or reading the resource raises, or an error
    /// when its bytes are not valid UTF-8.
    pub fn get_lines(
        loader: &dyn crate::util::resource_loader::ResourceLoader,
        resource: &str,
    ) -> Result<Vec<String>> {
        let reader = loader.open_resource(resource)?;
        crate::analysis::wordlist_loader::get_lines(reader, crate::util::IOUtils::UTF_8)
    }

    /// Reads one or more word-list resources into a set.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getWordSet(ResourceLoader, String, boolean)`.
    /// Returns `None` when no file is named, which is how Lucene signals "no
    /// word set configured" as distinct from an empty one.
    pub fn get_word_set(
        loader: &dyn crate::util::resource_loader::ResourceLoader,
        word_files: Option<&str>,
        ignore_case: bool,
    ) -> Result<Option<crate::analysis::CharArraySet>> {
        let files = Self::split_file_names(word_files);
        if files.is_empty() {
            return Ok(None);
        }
        // The default stop-word list holds around 35 words; ten per file is
        // the starting size Lucene picks.
        let mut words = crate::analysis::CharArraySet::new(files.len() * 10, ignore_case);
        for file in &files {
            for line in Self::get_lines(loader, file.trim())? {
                words.add(line);
            }
        }
        Ok(Some(words))
    }

    /// Reads one or more Snowball-format word-list resources into a set.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getSnowballWordSet(ResourceLoader, String, boolean)`.
    /// The Snowball format allows a `|` comment on any line and blank lines
    /// between entries.
    pub fn get_snowball_word_set(
        loader: &dyn crate::util::resource_loader::ResourceLoader,
        word_files: Option<&str>,
        ignore_case: bool,
    ) -> Result<Option<crate::analysis::CharArraySet>> {
        let files = Self::split_file_names(word_files);
        if files.is_empty() {
            return Ok(None);
        }
        let mut words = crate::analysis::CharArraySet::new(files.len() * 10, ignore_case);
        for file in &files {
            // Java does *not* route this through `getLines`: it opens the
            // resource with a reporting UTF-8 decoder and hands the reader
            // straight to `WordlistLoader.getSnowballWordSet`
            // (`AbstractAnalysisFactory.java:307-327`). The two differ, because
            // `getLines` drops `#` comments and blank lines and trims, none of
            // which the Snowball format asks for — its comment character is
            // `|`, and `#` is an ordinary character.
            let stream = loader.open_resource(file.trim())?;
            let decoded =
                crate::util::IOUtils::get_decoding_reader(stream, crate::util::IOUtils::UTF_8)?;
            let lines = crate::analysis::wordlist_loader::read_lines(&decoded.into_inner());
            for word in crate::analysis::wordlist_loader::get_snowball_word_set(lines).iter() {
                words.add(word.iter().collect::<String>());
            }
        }
        Ok(Some(words))
    }

    /// Consumes and returns a required argument.
    ///
    /// Equivalent to `require(Map, String)`.
    pub fn require(args: &mut HashMap<String, String>, name: &str) -> Result<String> {
        args.remove(name).ok_or_else(|| {
            LuceneError::IllegalArgument(format!("Configuration Error: missing parameter '{name}'"))
        })
    }

    /// Compiles a pattern for the value of the argument `name`, consuming it.
    ///
    /// Equivalent to `AbstractAnalysisFactory#getPattern`
    /// (`AbstractAnalysisFactory.java:265-277`), which is
    /// `Pattern.compile(require(args, name))`: a missing argument fails as
    /// [`require`](Self::require) does, with
    /// `Configuration Error: missing parameter '<name>'`, and a value that does
    /// not compile fails with
    /// `Configuration Error: '<name>' can not be parsed in <SimpleName>`, where
    /// `<SimpleName>` is `factory_simple_name`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`], with the messages above.
    ///
    /// **Divergence from Lucene 10.5.0.** Four:
    ///
    /// * **Iterating matches differs where an empty match starts at the end of
    ///   a non-empty one.** Java's `Matcher.find()` resumes where the previous
    ///   match ended, one character further only after an empty match
    ///   (`Matcher.java:770-786`), so it reports an empty match that starts
    ///   where a non-empty match ended, the one at the end of input included:
    ///   `\s*` over `a b` finds `""`, `" "`, `""`, `""`.
    ///   `fancy_regex::Regex::find_iter` drops such a match and finds `""`,
    ///   `" "`, `""` (`fancy-regex-0.19.2/src/lib.rs:257-277`). After an empty
    ///   match both move one character on, so a pattern whose matches are all
    ///   empty — `(?m)^`, `$`, `\b`, the empty pattern — iterates alike: on
    ///   each of its corpus rows `find_iter` and the loop below report the
    ///   same matches, and the rows that differ from Java do so for the
    ///   dialect reasons of the next item, or for the residue below. A caller
    ///   that iterates matches — a `PatternTokenizer`-style loop — must
    ///   reproduce Java's rule itself: loop over
    ///   [`fancy_regex::Regex::find_from_pos`], resuming at each match's end
    ///   and one character further after an empty match. Measured on the
    ///   corpus below, that loop matches Java on every iteration row except
    ///   one residue, which `find_iter` shares: Java steps one UTF-16 unit, so
    ///   after an empty match just before a supplementary character it also
    ///   reports an empty match between the character's two halves, a
    ///   position UTF-8 text does not have.
    /// * **The regular-expression dialect is `fancy-regex` 0.19's, not
    ///   `java.util.regex`'s.** Rust has no `java.util.regex`, and an exact
    ///   translation would have to reproduce Java's parser front end
    ///   (`RemoveQEQuoting`, `clazz`, `group0`, `escape`, `ref`, and the
    ///   comments-mode skipping inside each of them), so the pattern is
    ///   compiled as written. The difference is measured, not assumed: the
    ///   fixture `RuceneMemberContractsFixture.PATTERN_CORPUS` holds 207
    ///   discriminating rows — every construct of `Pattern`'s summary table,
    ///   every inline flag, the syntax the two dialects spell differently, and
    ///   the audit's iteration, nesting and one-dialect-only cases — run
    ///   through the real `getPattern` on the JDK 25.0.4 runtime and iterated
    ///   with `find_iter`. The portability test
    ///   `pattern_dialect_differences_are_exactly_the_declared_ones` fails if
    ///   any row other than these 111 differs, or if one of these stops
    ///   differing. Both sides use Unicode 16.0 character tables. The list is
    ///   closed over the corpus; a construct outside it is unmeasured, and
    ///   nothing here asserts that it agrees.
    ///   1. *Iteration:* the nine rows of the first item above (`a*`, `a|`,
    ///      `\s*`, `,?`, `x*`, `\d*`, `(?=b)|a`, and two over a supplementary
    ///      character).
    ///   2. *Characters:* octal escapes `\0n`, `\0nn`, `\0mnn` (rejected
    ///      here); a surrogate pair written as two `\uhhhh` (rejected); `\N{name}`
    ///      (accepted here, matching nothing); `\cX` (rejected); `\u{hh}` and
    ///      `\U000000hh` (accepted here, rejected by Java).
    ///   3. *Classes:* `[[:alpha:]]` and the other `[:…:]` forms (a POSIX class
    ///      here, a union of literal characters in Java); `--` and `~~` (set
    ///      difference and symmetric difference here; Java reads a range ending
    ///      at `-`, an error, and literal tildes); a predefined class before
    ///      `-`, as in `[\d-z]` (rejected here; Java reads the `-` literally);
    ///      an intersection with nothing after `&&`, `[a&&]` (Java matches `a`;
    ///      no match here).
    ///   4. `.`, which Java also stops at `\r`, U+0085, U+2028 and U+2029.
    ///   5. `\d`, `\D`, `\w`, `\W`, `\s`, `\S` (ASCII in Java, Unicode here);
    ///      `\h` (horizontal whitespace in Java, a hex digit here); `\H` and
    ///      `\V` (rejected here); `\v` (Java's vertical whitespace; only
    ///      U+000B here).
    ///   6. POSIX `\p{Lower}`, `Upper`, `Alpha`, `Digit`, `Alnum`, `Punct`,
    ///      `Graph`, `Print`, `Blank`, `Cntrl`, `Space` and their `\P`
    ///      complements (ASCII in Java, Unicode here); `\p{XDigit}` (rejected).
    ///   7. `\p{javaLowerCase}`, `javaUpperCase`, `javaWhitespace`,
    ///      `javaMirrored`, and the other `java…` classes (rejected); `\p{all}`
    ///      (rejected here).
    ///   8. Unicode blocks, `\p{InGreek}`, `\P{InGreek}`, `\p{blk=Greek}`
    ///      (rejected); a script or binary property without `Is` or `sc=`,
    ///      `\p{Latin}`, `\p{Lowercase}` (accepted here, rejected by Java).
    ///   9. *Boundaries:* `^` in multi-line mode (Java also matches after `\r`,
    ///      U+0085, U+2028 and U+2029, never between `\r` and `\n`, and never
    ///      at the end of input); `$` and `\Z` (Java also matches before a final
    ///      line terminator, `\r\n` included, and in multi-line mode before
    ///      every line terminator); `\b` and `\B` (Java's word is the ASCII
    ///      `\w` plus a non-spacing mark that follows a letter or digit);
    ///      `\b{g}` (rejected); `\K` (accepted here, rejected by Java).
    ///   10. `\R` followed by `\n` over `\r\n` (Java backtracks `\R` to the
    ///       `\r` alone); `\X` (rejected).
    ///   11. *Quantifiers:* `{,n}`, an unterminated `{` and `{x}` (rejected by
    ///       Java; a repetition or a literal here); a pattern starting with
    ///       `{n}` (Java repeats an empty atom; no match here); a quantified
    ///       quantifier, `x{2}{3}` (Java finds `xx` three times over six `x`;
    ///       no match here); a quantified look-ahead, `(?=a)*b` (rejected here).
    ///   12. *Back references:* to a group that does not exist, `(a)\2` (Java
    ///       compiles it, never matching; rejected here); `\11` with one group
    ///       (Java reads `\1` and a literal `1`; rejected here).
    ///   13. *Groups:* Java allows only `[a-zA-Z][a-zA-Z0-9]*` as a name and
    ///       rejects a repeated name, both accepted here; `(?P<name>…)`,
    ///       conditionals `(?(1)…|…)` and comments `(?#…)` (accepted here,
    ///       rejected by Java).
    ///   14. *Quotation:* `\Q…\E` (rejected here, inside a class too); `\<` and
    ///       `\>` (literal characters in Java, word boundaries here).
    ///   15. *Flags:* `(?i)` without `u` (Java folds ASCII only; here `é`/`É`,
    ///       `k`/U+212A and `s`/U+017F fold too, and so does a class:
    ///       `[à-ÿ]` matches `À`); `(?d)` (rejected here); `(?x)` (Java also
    ///       ignores whitespace inside a class); `(?U)` (`UNICODE_CHARACTER_CLASS`
    ///       in Java, swapped greediness here); `(?R)` (rejected by Java); `(?c)`
    ///       (`CANON_EQ` in Java, rejected here).
    ///
    ///   The other 96 corpus rows agree match for match. They exercise
    ///   literals, `\\`, `\xhh`, `\x{h…h}`, BMP `\uhhhh`, `\t \n \r \f \a \e`,
    ///   simple, negated, range, union, intersection and subtraction classes,
    ///   the anchors `^` (single-line), `\A`, `\G`, `\z`, greedy, reluctant
    ///   and possessive quantifiers on single atoms and groups, alternation,
    ///   numbered, named and forward back references, escaped punctuation and
    ///   non-ASCII, look-ahead, look-behind (unbounded included), atomic
    ///   groups, the `Is`, `gc=`, `sc=` and one-letter property forms, and the
    ///   flags `s`, `u` and `iu` — each on the samples the corpus gives it.
    /// * **The factory names itself.** Java reads the name off
    ///   `this.getClass().getSimpleName()`; this struct is held by the
    ///   concrete factory and cannot see its type, so the factory passes its
    ///   Java simple name in.
    /// * **The cause is not chained.** Java attaches the
    ///   `PatternSyntaxException` as the cause of the
    ///   `IllegalArgumentException`; [`LuceneError::IllegalArgument`] carries a
    ///   message only, so the message is Java's and the parser's own
    ///   diagnostic is dropped.
    pub fn get_pattern(
        args: &mut HashMap<String, String>,
        name: &str,
        factory_simple_name: &str,
    ) -> Result<fancy_regex::Regex> {
        let value = Self::require(args, name)?;
        fancy_regex::Regex::new(&value).map_err(|_| {
            LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' can not be parsed in {factory_simple_name}"
            ))
        })
    }

    /// Consumes and returns a required argument, checking it against a list.
    ///
    /// Equivalent to `require(Map, String, Collection, boolean)`.
    pub fn require_one_of(
        args: &mut HashMap<String, String>,
        name: &str,
        allowed_values: &[&str],
        case_sensitive: bool,
    ) -> Result<String> {
        let value = Self::require(args, name)?;
        let matches = allowed_values.iter().any(|allowed| {
            if case_sensitive {
                value == *allowed
            } else {
                value.eq_ignore_ascii_case(allowed)
            }
        });
        if matches {
            Ok(value)
        } else {
            Err(LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' value must be one of {allowed_values:?}"
            )))
        }
    }

    /// Consumes and returns an optional argument.
    ///
    /// Equivalent to `get(Map, String)`.
    pub fn get(args: &mut HashMap<String, String>, name: &str) -> Option<String> {
        args.remove(name)
    }

    /// Consumes an optional argument, falling back to `default_val`.
    ///
    /// Equivalent to `get(Map, String, String)`.
    pub fn get_or(args: &mut HashMap<String, String>, name: &str, default_val: &str) -> String {
        args.remove(name).unwrap_or_else(|| default_val.to_string())
    }

    /// Consumes a required integer argument.
    ///
    /// Equivalent to `requireInt(Map, String)`.
    pub fn require_int(args: &mut HashMap<String, String>, name: &str) -> Result<i32> {
        let value = Self::require(args, name)?;
        value.parse::<i32>().map_err(|e| {
            LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' is not an int: {e}"
            ))
        })
    }

    /// Consumes an optional integer argument.
    ///
    /// Equivalent to `getInt(Map, String, int)`.
    pub fn get_int(
        args: &mut HashMap<String, String>,
        name: &str,
        default_val: i32,
    ) -> Result<i32> {
        match args.remove(name) {
            None => Ok(default_val),
            Some(value) => value.parse::<i32>().map_err(|e| {
                LuceneError::IllegalArgument(format!(
                    "Configuration Error: '{name}' is not an int: {e}"
                ))
            }),
        }
    }

    /// Consumes a required boolean argument.
    ///
    /// Equivalent to `requireBoolean(Map, String)`.
    pub fn require_boolean(args: &mut HashMap<String, String>, name: &str) -> Result<bool> {
        let value = Self::require(args, name)?;
        value.parse::<bool>().map_err(|e| {
            LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' is not a boolean: {e}"
            ))
        })
    }

    /// Consumes an optional boolean argument.
    ///
    /// Equivalent to `getBoolean(Map, String, boolean)`.
    pub fn get_boolean(
        args: &mut HashMap<String, String>,
        name: &str,
        default_val: bool,
    ) -> Result<bool> {
        match args.remove(name) {
            None => Ok(default_val),
            Some(value) => value.parse::<bool>().map_err(|e| {
                LuceneError::IllegalArgument(format!(
                    "Configuration Error: '{name}' is not a boolean: {e}"
                ))
            }),
        }
    }

    /// Consumes a required float argument.
    ///
    /// Equivalent to `requireFloat(Map, String)`.
    pub fn require_float(args: &mut HashMap<String, String>, name: &str) -> Result<f32> {
        let value = Self::require(args, name)?;
        value.parse::<f32>().map_err(|e| {
            LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' is not a float: {e}"
            ))
        })
    }

    /// Consumes an optional float argument.
    ///
    /// Equivalent to `getFloat(Map, String, float)`.
    pub fn get_float(
        args: &mut HashMap<String, String>,
        name: &str,
        default_val: f32,
    ) -> Result<f32> {
        match args.remove(name) {
            None => Ok(default_val),
            Some(value) => value.parse::<f32>().map_err(|e| {
                LuceneError::IllegalArgument(format!(
                    "Configuration Error: '{name}' is not a float: {e}"
                ))
            }),
        }
    }

    /// Consumes an optional comma-separated list argument.
    ///
    /// Equivalent to `getSet(Map, String)`.
    pub fn get_set(args: &mut HashMap<String, String>, name: &str) -> Option<Vec<String>> {
        args.remove(name).map(|value| {
            value
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
    }
}

/// Builds a tokenizer from a configuration.
///
/// Equivalent to `org.apache.lucene.analysis.TokenizerFactory`.
pub trait TokenizerFactory: Send + Sync + std::fmt::Debug {
    /// The SPI name this factory is registered under.
    fn name(&self) -> &str;

    /// Returns the binary name of the Lucene class this factory ports.
    ///
    /// Stands in for `getClass().getName()`, which Rust cannot answer: there is
    /// no runtime class object, and `std::any::type_name` names the Rust type,
    /// not the Java one. `AnalysisSPILoader.reload` names a service by its
    /// class when the service's SPI name is invalid, so the name has to be
    /// carried by the factory itself. There is no default, as for
    /// [`Analyzer::java_class_name`](crate::analysis::Analyzer::java_class_name):
    /// every factory states its own.
    fn java_class_name(&self) -> &'static str;

    /// Creates the tokenizer.
    ///
    /// Equivalent to `TokenizerFactory.create(AttributeFactory)`.
    fn create(&self) -> Result<Box<dyn TokenizerLogic>>;
}

/// Builds a token filter from a configuration.
///
/// Equivalent to `org.apache.lucene.analysis.TokenFilterFactory`.
pub trait TokenFilterFactory: Send + Sync + std::fmt::Debug {
    /// The SPI name this factory is registered under.
    fn name(&self) -> &str;

    /// Returns the binary name of the Lucene class this factory ports.
    ///
    /// Stands in for `getClass().getName()`, which Rust cannot answer: there is
    /// no runtime class object, and `std::any::type_name` names the Rust type,
    /// not the Java one. `AnalysisSPILoader.reload` names a service by its
    /// class when the service's SPI name is invalid, so the name has to be
    /// carried by the factory itself. There is no default, as for
    /// [`Analyzer::java_class_name`](crate::analysis::Analyzer::java_class_name):
    /// every factory states its own.
    fn java_class_name(&self) -> &'static str;

    /// Wraps `input` in the filter.
    ///
    /// Equivalent to `TokenFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenFilterLogic>) -> Result<Box<dyn TokenFilterLogic>>;

    /// Normalizes `input`.
    ///
    /// The default returns `input` unchanged. A filter that should also apply
    /// at normalization time — when a query term is folded rather than
    /// analysed — overrides this and delegates to
    /// [`create`](Self::create).
    ///
    /// # Errors
    ///
    /// The default returns none; an override returns whatever
    /// [`create`](Self::create) returns.
    ///
    /// Equivalent to `org.apache.lucene.analysis.TokenFilterFactory#normalize`.
    fn normalize(&self, input: Box<dyn TokenFilterLogic>) -> Result<Box<dyn TokenFilterLogic>> {
        Ok(input)
    }
}

/// Builds a character filter from a configuration.
///
/// Equivalent to `org.apache.lucene.analysis.CharFilterFactory`.
pub trait CharFilterFactory: Send + Sync + std::fmt::Debug {
    /// The SPI name this factory is registered under.
    fn name(&self) -> &str;

    /// Returns the binary name of the Lucene class this factory ports.
    ///
    /// Stands in for `getClass().getName()`, which Rust cannot answer: there is
    /// no runtime class object, and `std::any::type_name` names the Rust type,
    /// not the Java one. `AnalysisSPILoader.reload` names a service by its
    /// class when the service's SPI name is invalid, so the name has to be
    /// carried by the factory itself. There is no default, as for
    /// [`Analyzer::java_class_name`](crate::analysis::Analyzer::java_class_name):
    /// every factory states its own.
    fn java_class_name(&self) -> &'static str;

    /// Wraps `input` in the filter.
    ///
    /// Equivalent to `CharFilterFactory.create(Reader)`.
    fn create(&self, input: Box<dyn CharFilter>) -> Result<Box<dyn CharFilter>>;

    /// Normalizes `input`.
    ///
    /// The default returns `input` unchanged. A character filter that should
    /// also apply at normalization time overrides this and delegates to
    /// [`create`](Self::create).
    ///
    /// # Errors
    ///
    /// The default returns none; an override returns whatever
    /// [`create`](Self::create) returns.
    ///
    /// Equivalent to `org.apache.lucene.analysis.CharFilterFactory#normalize`.
    fn normalize(&self, input: Box<dyn CharFilter>) -> Result<Box<dyn CharFilter>> {
        Ok(input)
    }
}

/// The SPI name a factory declares.
///
/// Java reads it reflectively out of the factory class's
/// `public static final String NAME` field, which is why
/// `AnalysisSPILoader.lookupSPIName` can accept any
/// `Class<? extends AbstractAnalysisFactory>`. Rust has neither reflection nor
/// a shared supertype for the three factory traits — `AbstractAnalysisFactory`
/// is a struct here, not a base class — so this trait carries what the three
/// have in common, and each implementation forwards to that factory's own
/// `name`.
pub trait AnalysisFactorySPIName {
    /// Returns the name the factory declares.
    fn spi_name(&self) -> &str;

    /// Returns the binary name of the factory's Lucene class, Java's
    /// `service.getName()`.
    fn java_class_name(&self) -> &str;
}

impl AnalysisFactorySPIName for dyn TokenizerFactory {
    fn spi_name(&self) -> &str {
        self.name()
    }

    fn java_class_name(&self) -> &str {
        TokenizerFactory::java_class_name(self)
    }
}

impl AnalysisFactorySPIName for dyn TokenFilterFactory {
    fn spi_name(&self) -> &str {
        self.name()
    }

    fn java_class_name(&self) -> &str {
        TokenFilterFactory::java_class_name(self)
    }
}

impl AnalysisFactorySPIName for dyn CharFilterFactory {
    fn spi_name(&self) -> &str {
        self.name()
    }

    fn java_class_name(&self) -> &str {
        CharFilterFactory::java_class_name(self)
    }
}

/// A registry of analysis factories of one kind, by SPI name.
///
/// Equivalent to `org.apache.lucene.analysis.AnalysisSPILoader`.
///
/// **Divergence from Lucene 10.5.0.** Java's loader discovers factories through
/// `ServiceLoader`, reading `META-INF/services` off the classpath, and
/// instantiates them reflectively from a `Map<String, String>`. Rust has
/// neither at run time, so factories are registered explicitly, as the crate
/// already does for codecs and doc-values formats. The lookup semantics — case
/// preserved, unknown name refused with the available names listed — are the
/// same.
pub struct AnalysisSPILoader<T: ?Sized> {
    /// Keyed by the lower-cased name, as Java's `services` map is.
    services: HashMap<String, Arc<T>>,
    /// The names as they were registered, which `availableServices` returns.
    original_names: Vec<String>,
    /// What the loader holds, for the error message.
    kind: &'static str,
}

impl<T: ?Sized> std::fmt::Debug for AnalysisSPILoader<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisSPILoader")
            .field("kind", &self.kind)
            .field("services", &self.services.len())
            .finish()
    }
}

impl<T: ?Sized> AnalysisSPILoader<T> {
    /// Creates an empty registry of `kind`.
    pub fn new(kind: &'static str) -> Self {
        Self {
            services: HashMap::new(),
            original_names: Vec::new(),
            kind,
        }
    }

    /// Returns whether `name` is a valid SPI name.
    ///
    /// Equivalent to the private `AnalysisSPILoader.isValidName(String)`, which
    /// matches `^[a-zA-Z][a-zA-Z0-9_]+$`: a letter, then at least one more
    /// letter, digit or underscore.
    pub fn is_valid_name(name: &str) -> bool {
        let mut chars = name.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() => {}
            _ => return false,
        }
        let mut rest = 0;
        for c in chars {
            if !(c.is_ascii_alphanumeric() || c == '_') {
                return false;
            }
            rest += 1;
        }
        rest >= 1
    }

    /// Looks a factory up by name, case-insensitively.
    ///
    /// Equivalent to `AnalysisSPILoader.lookupClass(String)`, which lower-cases
    /// the name before consulting the map. Java returns the `Class`; the
    /// registry holds instances, so this returns the instance.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] naming the available services,
    /// as Java does.
    pub fn lookup_class(&self, name: &str) -> Result<Arc<T>> {
        self.services
            .get(&name.to_lowercase())
            .map(Arc::clone)
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "A SPI class of type {} with name '{name}' does not exist. You need to add \
                     the corresponding registration. Currently registered: {:?}",
                    self.kind,
                    self.available_services()
                ))
            })
    }

    /// Looks a factory up by name.
    ///
    /// Equivalent to `AnalysisSPILoader.newInstance(String, Map)`, which looks
    /// the class up and then constructs it. The registry holds constructed
    /// factories, so the two steps collapse into
    /// [`lookup_class`](Self::lookup_class).
    ///
    /// # Errors
    ///
    /// Returns whatever [`lookup_class`](Self::lookup_class) returns.
    pub fn lookup(&self, name: &str) -> Result<Arc<T>> {
        self.lookup_class(name)
    }

    /// Returns every registered name, in registration order and with its
    /// original spelling.
    ///
    /// Equivalent to `AnalysisSPILoader.availableServices()`.
    pub fn available_services(&self) -> Vec<String> {
        self.original_names.clone()
    }

    /// Builds a factory by calling `ctor` with `args`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java looks the `Map<String, String>`
    /// constructor up on the `Class` reflectively and turns a class that has
    /// none into an `UnsupportedOperationException`
    /// (`AnalysisSPILoader.java` lines 173-201). Rust has no run-time class
    /// lookup, so the caller names the constructor; a type that has no such
    /// constructor cannot be passed here at all, which is the compile-time form
    /// of the same refusal. Java also unwraps an `InvocationTargetException` so
    /// the constructor's own failure surfaces unchanged, which is already how a
    /// returned `Err` behaves.
    ///
    /// # Errors
    ///
    /// Returns whatever the constructor returns.
    ///
    /// Equivalent to `org.apache.lucene.analysis.AnalysisSPILoader#newFactoryClassInstance`.
    pub fn new_factory_class_instance<F, R>(
        ctor: F,
        args: &mut HashMap<String, String>,
    ) -> Result<R>
    where
        F: FnOnce(&mut HashMap<String, String>) -> Result<R>,
    {
        ctor(args)
    }
}

impl<T: ?Sized + AnalysisFactorySPIName> AnalysisSPILoader<T> {
    /// Registers `service` under `name`.
    ///
    /// This is [`reload`](Self::reload) over a single service: it refuses a
    /// name the pattern rejects, keys the map by the lower-cased name, keeps
    /// the original spelling for
    /// [`available_services`](Self::available_services), and ignores a name
    /// that is already taken so that the first registration wins.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `name` is not a valid SPI
    /// name, standing in for Java's `ServiceConfigurationError`.
    pub fn register(&mut self, name: impl Into<String>, service: Arc<T>) -> Result<()> {
        self.reload([(name.into(), service)])
    }

    /// Adds every service in `services` whose name is not registered yet.
    ///
    /// Equivalent to `AnalysisSPILoader#reload` (`AnalysisSPILoader.java:76-133`).
    /// Each service's name must match the SPI name pattern; the map is keyed
    /// by the lower-cased name and the original spelling is kept for
    /// [`available_services`](Self::available_services). Only new names are
    /// added — a service placed earlier wins and an existing one is never
    /// replaced — and, as in Java, the update is all or nothing: the new map
    /// and name list are built on copies and installed only once every name
    /// has been accepted, so a refused name leaves the registry unchanged.
    ///
    /// **Divergence from Lucene 10.5.0.** Java discovers the services by
    /// scanning a `ClassLoader` with `ServiceLoader` and reads each name from
    /// the class's `NAME` field. Rust has neither at run time, so the caller
    /// supplies the `(name, service)` pairs, in the order the scan would have
    /// produced them, as [`NamedSPILoader::reload`](crate::util::spi::NamedSPILoader::reload)
    /// does for the codec loaders.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`], standing in for Java's
    /// `ServiceConfigurationError`, when a name is not a valid SPI name — with
    /// Java's message, which names the service by its
    /// [`java_class_name`](AnalysisFactorySPIName::java_class_name), Java's
    /// `service.getName()` — or
    /// when the lookup keys and the original names disagree in number — the
    /// consistency check Java performs before installing the result.
    pub fn reload<I, N>(&mut self, services: I) -> Result<()>
    where
        I: IntoIterator<Item = (N, Arc<T>)>,
        N: Into<String>,
    {
        let mut new_services = self.services.clone();
        let mut original_names = self.original_names.clone();
        for (name, service) in services {
            let original_name = name.into();
            if !Self::is_valid_name(&original_name) {
                return Err(LuceneError::IllegalArgument(format!(
                    "The name {original_name} for {} is invalid: Allowed characters are (English) \
                     alphabet, digits, and underscore. It should be started with an alphabet.",
                    service.java_class_name()
                )));
            }
            let name = original_name.to_lowercase();
            // Only the first service for each name is added; later ones are
            // ignored, which is what lets one be placed ahead of another.
            if !new_services.contains_key(&name) {
                new_services.insert(name, service);
                original_names.push(original_name);
            }
        }
        if new_services.len() != original_names.len() {
            return Err(LuceneError::IllegalArgument(
                "Service lookup key set is inconsistent with original name set!".to_string(),
            ));
        }
        self.services = new_services;
        self.original_names = original_names;
        Ok(())
    }

    /// Returns the SPI name `service` declares.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reads the class's
    /// `public static final String NAME` field by reflection and refuses a
    /// field that is not static, not final, not declared on that very class or
    /// not a `String` — with `NoSuchFieldException`, `IllegalAccessException`
    /// or `IllegalStateException` (`AnalysisSPILoader.java` lines 160-172).
    /// None of those failures can arise here: a factory carries its name
    /// through [`AnalysisFactorySPIName`], so a type without one does not
    /// compile, and the lookup cannot fail at run time.
    ///
    /// Equivalent to `org.apache.lucene.analysis.AnalysisSPILoader#lookupSPIName`.
    pub fn lookup_spi_name(service: &T) -> &str {
        service.spi_name()
    }
}

// ---------------------------------------------------------------------------
// The global loaders behind the factories' static entry points
// ---------------------------------------------------------------------------

/// The loader behind [`TokenizerFactory`]'s static entry points.
///
/// Equivalent to `TokenizerFactory.Holder.LOADER`, which Java hides in a
/// holder class so the factories are only discovered when first needed. A
/// `LazyLock` gives the same deferred initialisation; the `RwLock` is what lets
/// [`reload_tokenizers`] add to it afterwards, which in Java is `reload`'s
/// `synchronized`.
static TOKENIZER_LOADER: LazyLock<RwLock<AnalysisSPILoader<dyn TokenizerFactory>>> =
    LazyLock::new(|| RwLock::new(AnalysisSPILoader::new("TokenizerFactory")));

/// The loader behind [`TokenFilterFactory`]'s static entry points.
///
/// Equivalent to `TokenFilterFactory.Holder.LOADER`.
static TOKEN_FILTER_LOADER: LazyLock<RwLock<AnalysisSPILoader<dyn TokenFilterFactory>>> =
    LazyLock::new(|| RwLock::new(AnalysisSPILoader::new("TokenFilterFactory")));

/// The loader behind [`CharFilterFactory`]'s static entry points.
///
/// Equivalent to `CharFilterFactory.Holder.LOADER`.
static CHAR_FILTER_LOADER: LazyLock<RwLock<AnalysisSPILoader<dyn CharFilterFactory>>> =
    LazyLock::new(|| RwLock::new(AnalysisSPILoader::new("CharFilterFactory")));

/// Looks up a tokenizer factory by name.
///
/// Equivalent to `TokenizerFactory.forName(String, Map)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn tokenizer_for_name(name: &str) -> Result<Arc<dyn TokenizerFactory>> {
    tokenizer_lookup_class(name)
}

/// Looks a tokenizer factory up by name.
///
/// Equivalent to `TokenizerFactory.lookupClass(String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn tokenizer_lookup_class(name: &str) -> Result<Arc<dyn TokenizerFactory>> {
    TOKENIZER_LOADER
        .read()
        .expect("INVARIANT: the tokenizer loader lock is never poisoned")
        .lookup_class(name)
}

/// Returns the names of every registered tokenizer factory.
///
/// Equivalent to `TokenizerFactory.availableTokenizers()`.
pub fn available_tokenizers() -> Vec<String> {
    TOKENIZER_LOADER
        .read()
        .expect("INVARIANT: the tokenizer loader lock is never poisoned")
        .available_services()
}

/// Returns the SPI name a tokenizer factory declares.
///
/// Equivalent to `TokenizerFactory.findSPIName(Class)`, which reads the class's
/// `public static final String NAME` field by reflection. The trait declares
/// [`TokenizerFactory::name`] instead, so the name is read from the factory.
pub fn find_tokenizer_spi_name(service: &dyn TokenizerFactory) -> &str {
    service.name()
}

/// Adds `factory` to the tokenizer registry under its own SPI name.
///
/// Equivalent to `TokenizerFactory.reloadTokenizers(ClassLoader)`, which
/// rescans a classloader for factories it has not seen. Rust has no run-time
/// service discovery, so the caller supplies the factory; as in Java, only new
/// names are added and an existing one is never replaced.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the factory's name is not a
/// valid SPI name.
pub fn reload_tokenizers(factory: Arc<dyn TokenizerFactory>) -> Result<()> {
    let name = factory.name().to_string();
    TOKENIZER_LOADER
        .write()
        .expect("INVARIANT: the tokenizer loader lock is never poisoned")
        .register(name, factory)
}

/// Looks up a token-filter factory by name.
///
/// Equivalent to `TokenFilterFactory.forName(String, Map)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn token_filter_for_name(name: &str) -> Result<Arc<dyn TokenFilterFactory>> {
    token_filter_lookup_class(name)
}

/// Looks a token-filter factory up by name.
///
/// Equivalent to `TokenFilterFactory.lookupClass(String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn token_filter_lookup_class(name: &str) -> Result<Arc<dyn TokenFilterFactory>> {
    TOKEN_FILTER_LOADER
        .read()
        .expect("INVARIANT: the token-filter loader lock is never poisoned")
        .lookup_class(name)
}

/// Returns the names of every registered token-filter factory.
///
/// Equivalent to `TokenFilterFactory.availableTokenFilters()`.
pub fn available_token_filters() -> Vec<String> {
    TOKEN_FILTER_LOADER
        .read()
        .expect("INVARIANT: the token-filter loader lock is never poisoned")
        .available_services()
}

/// Returns the SPI name a token-filter factory declares.
///
/// Equivalent to `TokenFilterFactory.findSPIName(Class)`.
pub fn find_token_filter_spi_name(service: &dyn TokenFilterFactory) -> &str {
    service.name()
}

/// Adds `factory` to the token-filter registry under its own SPI name.
///
/// Equivalent to `TokenFilterFactory.reloadTokenFilters(ClassLoader)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the factory's name is not a
/// valid SPI name.
pub fn reload_token_filters(factory: Arc<dyn TokenFilterFactory>) -> Result<()> {
    let name = factory.name().to_string();
    TOKEN_FILTER_LOADER
        .write()
        .expect("INVARIANT: the token-filter loader lock is never poisoned")
        .register(name, factory)
}

/// Looks up a character-filter factory by name.
///
/// Equivalent to `CharFilterFactory.forName(String, Map)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn char_filter_for_name(name: &str) -> Result<Arc<dyn CharFilterFactory>> {
    char_filter_lookup_class(name)
}

/// Looks a character-filter factory up by name.
///
/// Equivalent to `CharFilterFactory.lookupClass(String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn char_filter_lookup_class(name: &str) -> Result<Arc<dyn CharFilterFactory>> {
    CHAR_FILTER_LOADER
        .read()
        .expect("INVARIANT: the char-filter loader lock is never poisoned")
        .lookup_class(name)
}

/// Returns the names of every registered character-filter factory.
///
/// Equivalent to `CharFilterFactory.availableCharFilters()`.
pub fn available_char_filters() -> Vec<String> {
    CHAR_FILTER_LOADER
        .read()
        .expect("INVARIANT: the char-filter loader lock is never poisoned")
        .available_services()
}

/// Returns the SPI name a character-filter factory declares.
///
/// Equivalent to `CharFilterFactory.findSPIName(Class)`.
pub fn find_char_filter_spi_name(service: &dyn CharFilterFactory) -> &str {
    service.name()
}

/// Adds `factory` to the character-filter registry under its own SPI name.
///
/// Equivalent to `CharFilterFactory.reloadCharFilters(ClassLoader)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the factory's name is not a
/// valid SPI name.
pub fn reload_char_filters(factory: Arc<dyn CharFilterFactory>) -> Result<()> {
    let name = factory.name().to_string();
    CHAR_FILTER_LOADER
        .write()
        .expect("INVARIANT: the char-filter loader lock is never poisoned")
        .register(name, factory)
}

#[cfg(test)]
mod get_lines_tests {
    use super::*;
    use crate::util::resource_loader::ResourceLoader;
    use std::io::Read;

    /// A loader over one in-memory resource.
    struct OneResource(&'static str);

    impl ResourceLoader for OneResource {
        fn open_resource(&self, _resource: &str) -> Result<Box<dyn Read + Send>> {
            Ok(Box::new(std::io::Cursor::new(self.0.as_bytes().to_vec())))
        }
    }

    /// `AbstractAnalysisFactory.getLines` is `WordlistLoader.getLines(stream,
    /// UTF_8)`, so it must drop comment and blank lines rather than hand every
    /// raw line back. Reading the resource and splitting it on newlines — which
    /// is what this port did before rmp task #173 — turns a `#` comment into a
    /// stop word.
    #[test]
    fn get_lines_applies_the_wordlist_loader_rules() {
        let loader = OneResource("# a comment\n\n  spaced  \nword\n");
        let lines = AbstractAnalysisFactory::get_lines(&loader, "stop.txt").expect("lines");
        assert_eq!(lines, vec!["spaced".to_string(), "word".to_string()]);
    }

    /// And therefore neither does the word set built from it.
    #[test]
    fn get_word_set_does_not_admit_comments_or_blanks() {
        let loader = OneResource("# a comment\n\nword\n");
        let words = AbstractAnalysisFactory::get_word_set(&loader, Some("stop.txt"), false)
            .expect("word set")
            .expect("a file was named");
        let mut got: Vec<String> = words.iter().map(|w| w.iter().collect()).collect();
        got.sort();
        assert_eq!(got, vec!["word".to_string()]);
    }

    /// The Snowball reader must *not* go through `getLines`: its comment
    /// character is `|`, `#` is an ordinary character, and Java hands the
    /// decoded reader straight to `WordlistLoader.getSnowballWordSet`
    /// (`AbstractAnalysisFactory.java:307-327`).
    #[test]
    fn snowball_word_set_keeps_what_get_lines_would_drop() {
        let loader = OneResource("#hash a | comment\n\nb\n");
        let words =
            AbstractAnalysisFactory::get_snowball_word_set(&loader, Some("stop.txt"), false)
                .expect("word set")
                .expect("a file was named");
        let mut got: Vec<String> = words.iter().map(|w| w.iter().collect()).collect();
        got.sort();
        assert_eq!(
            got,
            vec!["#hash".to_string(), "a".to_string(), "b".to_string()],
            "'#hash' is a Snowball word, not a comment"
        );
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;

    /// A service that states its SPI name and its Lucene class, as a factory
    /// does through [`AnalysisFactorySPIName`].
    #[derive(Debug)]
    struct Service {
        tag: &'static str,
        class: &'static str,
    }

    impl AnalysisFactorySPIName for Service {
        fn spi_name(&self) -> &str {
            self.tag
        }

        fn java_class_name(&self) -> &str {
            self.class
        }
    }

    fn loader() -> AnalysisSPILoader<Service> {
        AnalysisSPILoader::new("TokenFilterFactory")
    }

    fn service(tag: &'static str) -> Arc<Service> {
        Arc::new(Service {
            tag,
            class: "org.example.ExampleFilterFactory",
        })
    }

    /// `reload` adds only names it has not seen, keyed case-insensitively,
    /// keeping the first service for a name and its original spelling —
    /// `AnalysisSPILoader.java:116-120`, where a later service of the same
    /// lower-cased name is ignored, whether it comes from an earlier reload or
    /// from the same scan.
    #[test]
    fn reload_adds_new_names_and_keeps_the_first_service() {
        let mut spi = loader();
        spi.reload([
            ("Lowercase", service("first")),
            ("lowercase", service("same scan")),
        ])
        .unwrap();
        spi.reload([
            ("LOWERCASE", service("later scan")),
            ("stop", service("stop")),
        ])
        .unwrap();

        assert_eq!(spi.lookup_class("lowerCASE").unwrap().tag, "first");
        assert_eq!(spi.lookup_class("Stop").unwrap().tag, "stop");
        assert_eq!(
            spi.available_services(),
            vec!["Lowercase".to_string(), "stop".to_string()]
        );
    }

    /// Java builds the new map and name set on copies and installs them only
    /// at the end (`AnalysisSPILoader.java:78-79,131-132`), so a
    /// `ServiceConfigurationError` thrown for one service leaves every
    /// service of that scan out — including the valid ones before it. The
    /// message is Java's, naming the offending service by its class
    /// (`AnalysisSPILoader.java:91-96`).
    #[test]
    fn reload_is_all_or_nothing() {
        let mut spi = loader();
        spi.reload([("stop", service("stop"))]).unwrap();

        let bad = Arc::new(Service {
            tag: "invalid",
            class: "org.example.BadFilterFactory",
        });
        let err = spi
            .reload([("Lowercase", service("valid")), ("1bad", bad)])
            .unwrap_err();
        match err {
            LuceneError::IllegalArgument(message) => assert_eq!(
                message,
                "The name 1bad for org.example.BadFilterFactory is invalid: Allowed characters \
                 are (English) alphabet, digits, and underscore. It should be started with an \
                 alphabet."
            ),
            other => panic!("expected an illegal-argument error, got {other:?}"),
        }
        assert!(
            spi.lookup_class("lowercase").is_err(),
            "the valid name was rolled back"
        );
        assert_eq!(spi.available_services(), vec!["stop".to_string()]);
    }

    /// `register` is `reload` over one service.
    #[test]
    fn register_is_reload_of_one_service() {
        let mut spi = loader();
        spi.register("Stop", service("a")).unwrap();
        spi.register("STOP", service("b")).unwrap();
        assert!(
            spi.register("x", service("c")).is_err(),
            "one character is too short"
        );
        assert_eq!(spi.lookup_class("stop").unwrap().tag, "a");
        assert_eq!(spi.available_services(), vec!["Stop".to_string()]);
    }
}
