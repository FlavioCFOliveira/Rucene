//! Analysis factories ported from `org.apache.lucene.analysis`.
//!
//! A factory builds one analysis component from a string-keyed configuration —
//! the form a schema file or a query parameter carries — so an analysis chain
//! can be assembled without naming Rust types.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use crate::analysis::{CharFilter, TokenFilterLogic, TokenizerLogic};
use crate::error::{LuceneError, Result};
use crate::util::automaton::case_folding;
use crate::util::extra::Version;
use crate::util::java_collections::JavaHashSet;
use crate::util::resource_loader::{ClassLoader, ClassLoaderUtils};
use crate::util::spi::{load_service_classes, lucene_core_class_loader, ServiceClass};

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
    /// Whether the version was set explicitly, as a schema reader records it
    /// through [`set_explicit_lucene_match_version`](Self::set_explicit_lucene_match_version).
    ///
    /// Equivalent to `AbstractAnalysisFactory.isExplicitLuceneMatchVersion`,
    /// which starts `false` (`AbstractAnalysisFactory.java:64`) and which the
    /// constructor does not touch, whether or not `luceneMatchVersion` is
    /// among the arguments.
    is_explicit_lucene_match_version: bool,
}

impl AbstractAnalysisFactory {
    /// Builds the configuration holder from `args`, consuming the version, the
    /// class name and the SPI name.
    ///
    /// Equivalent to `AbstractAnalysisFactory(Map<String, String>)`
    /// (`AbstractAnalysisFactory.java:95-109`), which reads the version with
    /// [`get`](Self::get).
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let original_args = args.clone();
        let lucene_match_version = match Self::get(args, LUCENE_MATCH_VERSION_PARAM) {
            None => Version::LATEST,
            // `catch (ParseException pe) { throw new IllegalArgumentException(pe); }`
            // (`AbstractAnalysisFactory.java:101-105`): the message is the
            // `ParseException`'s `toString()`, and it is the cause.
            // `parse_leniently` reports the `ParseException` as an
            // `IllegalArgumentWithCause` carrying its message.
            Some(version) => Version::parse_leniently(&version).map_err(|pe| {
                let message = match &pe {
                    LuceneError::IllegalArgument(message)
                    | LuceneError::IllegalArgumentWithCause { message, .. } => message.clone(),
                    other => other.to_string(),
                };
                LuceneError::illegal_argument_with_cause(
                    format!("java.text.ParseException: {message}"),
                    std::sync::Arc::new(pe),
                )
            })?,
        };
        args.remove(CLASS_NAME); // consume the class arg
        args.remove(SPI_NAME); // consume the spi arg
        Ok(Self {
            original_args,
            lucene_match_version,
            is_explicit_lucene_match_version: false,
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

    /// Returns whether the version was recorded as explicitly set.
    ///
    /// Equivalent to `AbstractAnalysisFactory.isExplicitLuceneMatchVersion()`:
    /// `false` until [`set_explicit_lucene_match_version`](Self::set_explicit_lucene_match_version)
    /// says otherwise.
    pub fn is_explicit_lucene_match_version(&self) -> bool {
        self.is_explicit_lucene_match_version
    }

    /// Records whether the version was set explicitly.
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
    /// Equivalent to `AbstractAnalysisFactory.getChar(Map, String, char)`
    /// (`AbstractAnalysisFactory.java:231-242`), whose `s.length() != 1`
    /// counts UTF-16 code units: a supplementary character, two units long,
    /// is refused as `ab` is.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the value is not exactly
    /// one UTF-16 code unit long.
    pub fn get_char(
        args: &mut HashMap<String, String>,
        name: &str,
        default_value: char,
    ) -> Result<char> {
        match args.remove(name) {
            None => Ok(default_value),
            Some(s) => match s.chars().next() {
                Some(c) if s.encode_utf16().count() == 1 => Ok(c),
                _ => Err(LuceneError::IllegalArgument(format!(
                    "{name} should be a char. \"{s}\" is invalid"
                ))),
            },
        }
    }

    /// Consumes a required single-character argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.requireChar(Map, String)`
    /// (`AbstractAnalysisFactory.java:227-229`), `require(args, name).charAt(0)`:
    /// the first UTF-16 code unit of the value, Java's `char` — the high
    /// surrogate when the value starts with a supplementary character.
    ///
    /// # Errors
    ///
    /// Returns what [`require`](Self::require) returns for a missing
    /// argument, and, for an empty one, [`LuceneError::Other`] with the
    /// message of the `StringIndexOutOfBoundsException` `charAt(0)` throws,
    /// `Index 0 out of bounds for length 0`.
    ///
    /// **Divergence from Lucene 10.5.0.** [`LuceneError`] has no
    /// `StringIndexOutOfBoundsException`, so the JDK's message is carried by
    /// [`LuceneError::Other`], as the port does for the other JDK exceptions
    /// it has no variant for.
    pub fn require_char(args: &mut HashMap<String, String>, name: &str) -> Result<u16> {
        let value = Self::require(args, name)?;
        value
            .encode_utf16()
            .next()
            .ok_or_else(|| LuceneError::Other("Index 0 out of bounds for length 0".to_string()))
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

    /// Returns the `class` argument the factory was built from, or the
    /// factory's own class name when there was none.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getClassArg()`
    /// (`AbstractAnalysisFactory.java:369-377`), which falls back to
    /// `getClass().getName()`. This struct is held by the concrete factory
    /// and cannot see its type, so the factory passes its Java binary name in
    /// as `java_class_name` — the value its `java_class_name()` reports — as
    /// it passes its simple name to [`get_pattern`](Self::get_pattern).
    pub fn get_class_arg(&self, java_class_name: &str) -> String {
        match self.original_args.get(CLASS_NAME) {
            Some(class_name) => class_name.clone(),
            None => java_class_name.to_string(),
        }
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
            let wlist = Self::get_lines(loader, file.trim())?;
            // `words.addAll(StopFilter.makeStopSet(wlist, ignoreCase))`
            // (`AbstractAnalysisFactory.java:292-293`).
            for word in crate::analysis::stop_filter::make_stop_set(&wlist, ignore_case).iter() {
                let mut word = word.to_vec();
                let len = word.len();
                words.add_chars(&mut word, 0, len);
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
    /// The pattern is written in Java's dialect, and Rust has no
    /// `java.util.regex`: the port parses it as the JDK 25 that runs Lucene
    /// 10.5.0 parses it — `\Q…\E` quoting, comments mode, inline flags,
    /// classes with their unions, intersections and Java's case folding,
    /// every escape, property and boundary — and compiles the equivalent
    /// `fancy-regex` 0.19 pattern, with Unicode 16.0 tables on both sides,
    /// and with no limit on backtracking, as Java's matcher has none (save
    /// the case the second item below names). A counted repetition the
    /// engine's automaton cannot hold is compiled for its backtracking VM,
    /// which repeats with a counter as Java does.
    /// The translation is measured, not assumed: the fixture
    /// `RuceneMemberContractsFixture.PATTERN_CORPUS` runs 207 discriminating
    /// rows — every construct of `Pattern`'s summary table, every inline flag,
    /// the syntax the two dialects spell differently, and the iteration,
    /// nesting and one-dialect-only cases — through the real `getPattern`,
    /// and the portability test
    /// `pattern_dialect_differences_are_exactly_the_declared_ones` requires
    /// every row to match Java's matches at Java's positions, save the two
    /// rows the first item below names.
    ///
    /// **Divergence from Lucene 10.5.0.** Seven:
    ///
    /// * **A position between the two UTF-16 halves of a supplementary
    ///   character does not exist here.** Java searches and reports positions
    ///   in UTF-16 units: after an empty match just before a supplementary
    ///   character, and after a failed attempt at one, it searches at its low
    ///   half, where a lone surrogate can match — `x*` over `a😀b` and the
    ///   empty pattern over `😀` report an extra empty match there (two corpus
    ///   rows), and `[^😀]b` matches the half before `b`. A Rust string has
    ///   no such position.
    /// * **What the regex engine cannot express is refused or differs.**
    ///   A case-insensitive back reference folds Unicode's simple case folding
    ///   and only between texts of equal UTF-8 length, where Java folds ASCII
    ///   only without `UNICODE_CASE`, and compares `Character.toUpperCase` and
    ///   `toLowerCase` per character with it: `(?i)(é)\1` matches `éÉ` here,
    ///   `(?iu)(k)\1` does not match `kK` (`K` is U+212A). A look-behind of
    ///   variable length that needs an atomic group — a possessive
    ///   quantifier, an atomic group, `\X`, a repeated `\R` — is refused
    ///   (`fancy-regex` matches a variable-length look-behind only with its
    ///   automaton). A counted repetition (`{n}`, `{n,}`, `{n,m}` beyond one
    ///   iteration) of a body that can match the empty string, such as
    ///   `(?:x|){3000}` or `(?:a?){2,9}`, makes the engine keep its limit of a
    ///   million backtracking steps per search, and a search that needs more
    ///   fails with the engine's error: the engine checks for an empty
    ///   iteration only in an unbounded repetition past its minimum, where
    ///   Java's loop ends at the first empty iteration, so without the limit
    ///   such a search could take time and memory in proportion to the count
    ///   (`^(?:a?){2147483647}` over `xxxx`), where Java answers at once. A
    ///   counted repetition too large for the engine's automaton, such as
    ///   `(?<=\p{L}{1000})b` or `(?:a?){1000000}`, is refused when it lies
    ///   in a look-behind or its body can match the empty string (elsewhere
    ///   the VM runs it), and so is a pattern nested in 64 groups or 250
    ///   classes, the engine's own nesting limits.
    ///   A capturing group that only matches the empty string, repeated
    ///   greedily by `*` or `{0,n}`, never runs here, where Java runs it for
    ///   the captures made in its look-arounds and discards its own.
    /// * **Matcher state.** Java's `\G` anchors at the end of the previous
    ///   match, and its `\b{g}` measures grapheme clusters from there (on the
    ///   JDK, a match that starts after a line terminator measures them from
    ///   elsewhere still: `.\b{g}` over `\r\u{308}\u{308}` reports a boundary
    ///   between the two marks). The compiled pattern has no matcher: `\G`
    ///   anchors where the search starts, and `\b{g}` reports the boundaries
    ///   of the whole input, which is what Java reports for `\b{g}` alone.
    ///   A caller that iterates matches as Java does (the fifth item) differs
    ///   from Java only after an empty match for `\G`, and after a match for
    ///   `\b{g}`.
    /// * **`CANON_EQ` set inline.** Java accepts `(?c)` and its classes then
    ///   match canonically equivalent sequences (`(?c)[é]` matches
    ///   `e\u{301}`); here `(?c)` has no effect.
    /// * **Iterating matches differs where an empty match starts at the end of
    ///   a non-empty one.** Java's `Matcher.find()` resumes where the previous
    ///   match ended, one character further only after an empty match
    ///   (`Matcher.java:770-786`), so it reports an empty match that starts
    ///   where a non-empty match ended, the one at the end of input included:
    ///   `\s*` over `a b` finds `""`, `" "`, `""`, `""`.
    ///   `fancy_regex::Regex::find_iter` drops such a match and finds `""`,
    ///   `" "`, `""` (`fancy-regex-0.19.2/src/lib.rs:257-277`). A caller that
    ///   iterates matches — a `PatternTokenizer`-style loop — must reproduce
    ///   Java's rule itself: loop over [`fancy_regex::Regex::find_from_pos`],
    ///   resuming at each match's end and one character further after an
    ///   empty match, as the portability test does.
    /// * **The factory names itself.** Java reads the name off
    ///   `this.getClass().getSimpleName()`; this struct is held by the
    ///   concrete factory and cannot see its type, so the factory passes its
    ///   Java simple name in.
    /// * **The cause is the port's diagnostic.** Java attaches the
    ///   `PatternSyntaxException` as the cause of the
    ///   `IllegalArgumentException` (`AbstractAnalysisFactory.java:266-276`).
    ///   The message is Java's, and the cause is
    ///   [`LuceneError::IllegalArgument`] — `PatternSyntaxException` extends
    ///   `IllegalArgumentException` — carrying the port parser's description,
    ///   index and pattern laid out as `PatternSyntaxException.getMessage()`
    ///   lays them out, or the engine's own diagnostic for a pattern it
    ///   refuses; the descriptions follow Java's but are not asserted equal.
    pub fn get_pattern(
        args: &mut HashMap<String, String>,
        name: &str,
        factory_simple_name: &str,
    ) -> Result<fancy_regex::Regex> {
        let value = Self::require(args, name)?;
        let can_not_be_parsed = |cause: String| {
            LuceneError::illegal_argument_with_cause(
                format!("Configuration Error: '{name}' can not be parsed in {factory_simple_name}"),
                std::sync::Arc::new(LuceneError::IllegalArgument(cause)),
            )
        };
        let translation = crate::analysis::java_regex::translate_for_engine(&value, false)
            .map_err(|syntax| can_not_be_parsed(syntax.to_string()))?;
        // Java's matcher has no backtracking limit, and neither has the
        // compiled pattern, save over a bounded loop of a body that can match
        // the empty string, which the engine runs without Java's check for an
        // empty iteration and which keeps the engine's limit (see the notes).
        let unlimited = !translation.bounded_empty_loop;
        let compile = |pattern: &str| {
            let mut builder = fancy_regex::RegexBuilder::new(pattern);
            if unlimited {
                builder.backtrack_limit(usize::MAX);
            }
            builder.build()
        };
        match compile(&translation.pattern) {
            Ok(regex) => Ok(regex),
            Err(engine) => {
                // A counted repetition the engine's automaton cannot hold is
                // run by its VM instead (see `translate_for_engine`).
                let counted_in_vm = crate::analysis::java_regex::translate_for_engine(&value, true)
                    .map_err(|syntax| can_not_be_parsed(syntax.to_string()))?
                    .pattern;
                if counted_in_vm != translation.pattern {
                    if let Ok(regex) = compile(&counted_in_vm) {
                        return Ok(regex);
                    }
                }
                Err(can_not_be_parsed(engine.to_string()))
            }
        }
    }

    /// Consumes and returns a required argument, checking it against a list.
    ///
    /// Equivalent to `AbstractAnalysisFactory.require(Map, String, Collection, boolean)`
    /// (`AbstractAnalysisFactory.java:131-154`): the value is compared with
    /// `String.equals`, or `String.equalsIgnoreCase` when not
    /// `case_sensitive`, and a value matching none is refused with the list
    /// rendered as `Collection.toString()` does, `[a, b]`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the argument is missing
    /// or matches none of `allowed_values`.
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
                java_equals_ignore_case(&value, allowed)
            }
        });
        if matches {
            Ok(value)
        } else {
            Err(LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' value must be one of [{}]",
                allowed_values.join(", ")
            )))
        }
    }

    /// Consumes and returns an optional argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.get(Map, String)`.
    pub fn get(args: &mut HashMap<String, String>, name: &str) -> Option<String> {
        args.remove(name)
    }

    /// Consumes an optional argument, falling back to `default_val`.
    ///
    /// Equivalent to the overload `AbstractAnalysisFactory.get(Map, String, String)`.
    pub fn get_or(args: &mut HashMap<String, String>, name: &str, default_val: &str) -> String {
        args.remove(name).unwrap_or_else(|| default_val.to_string())
    }

    /// Consumes a required integer argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.requireInt(Map, String)`,
    /// `Integer.parseInt(require(args, name))`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the argument is missing,
    /// and, standing in for the `NumberFormatException` (an
    /// `IllegalArgumentException`) `Integer.parseInt` throws, with its
    /// message `For input string: "<value>"` when it is not an `int`.
    pub fn require_int(args: &mut HashMap<String, String>, name: &str) -> Result<i32> {
        let value = Self::require(args, name)?;
        parse_int(&value)
    }

    /// Consumes an optional integer argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getInt(Map, String, int)`.
    ///
    /// # Errors
    ///
    /// Returns what [`require_int`](Self::require_int) returns for a value
    /// that is not an `int`.
    pub fn get_int(
        args: &mut HashMap<String, String>,
        name: &str,
        default_val: i32,
    ) -> Result<i32> {
        match args.remove(name) {
            None => Ok(default_val),
            Some(value) => parse_int(&value),
        }
    }

    /// Consumes a required boolean argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.requireBoolean(Map, String)`,
    /// `Boolean.parseBoolean(require(args, name))`: `true` for `"true"` in
    /// any case, `false` for anything else.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the argument is missing.
    pub fn require_boolean(args: &mut HashMap<String, String>, name: &str) -> Result<bool> {
        let value = Self::require(args, name)?;
        Ok(parse_boolean(&value))
    }

    /// Consumes an optional boolean argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getBoolean(Map, String, boolean)`,
    /// which never fails: `Boolean.parseBoolean` maps every value other than
    /// `"true"` (in any case) to `false`.
    pub fn get_boolean(args: &mut HashMap<String, String>, name: &str, default_val: bool) -> bool {
        match args.remove(name) {
            None => default_val,
            Some(value) => parse_boolean(&value),
        }
    }

    /// Consumes a required float argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.requireFloat(Map, String)`,
    /// `Float.parseFloat(require(args, name))`; see [`get_float`](Self::get_float)
    /// for the grammar.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the argument is missing
    /// or is not a float.
    pub fn require_float(args: &mut HashMap<String, String>, name: &str) -> Result<f32> {
        let value = Self::require(args, name)?;
        parse_float(&value)
    }

    /// Consumes an optional float argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getFloat(Map, String, float)`,
    /// whose `Float.parseFloat` trims the characters up to U+0020 and reads
    /// an optional sign, then `NaN`, `Infinity`, a hexadecimal float
    /// (`0x1.8p1`) or a decimal one (`1.5`, `.5`, `5.`, `1e5`), with an
    /// optional `f`, `F`, `d` or `D` suffix, rounded to the nearest `float`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`], standing in for the
    /// `NumberFormatException` (an `IllegalArgumentException`) Java throws,
    /// with its message: `empty String` for a value that trims to nothing,
    /// `For input string: "<value>"` otherwise.
    pub fn get_float(
        args: &mut HashMap<String, String>,
        name: &str,
        default_val: f32,
    ) -> Result<f32> {
        match args.remove(name) {
            None => Ok(default_val),
            Some(value) => parse_float(&value),
        }
    }

    /// Consumes an optional set-of-items argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getSet(Map, String)`
    /// (`AbstractAnalysisFactory.java:244-263`): the items are the matches of
    /// `[^,\s]+` — so commas and the ASCII whitespace `\s` stands for both
    /// separate them — gathered into a `java.util.HashSet`, iterated in
    /// Java's order; a value with no item yields `None`, as Java's `null`.
    pub fn get_set(args: &mut HashMap<String, String>, name: &str) -> Option<JavaHashSet> {
        let value = args.remove(name)?;
        let mut set: Option<JavaHashSet> = None;
        let is_separator =
            |c: char| matches!(c, ',' | ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r');
        for item in value.split(is_separator).filter(|item| !item.is_empty()) {
            set.get_or_insert_with(JavaHashSet::new)
                .insert(item.to_string());
        }
        set
    }
}

/// `Integer.parseInt(String)`, failing with the `NumberFormatException`
/// message `For input string: "<s>"`.
fn parse_int(s: &str) -> Result<i32> {
    crate::util::extra::java_parse_int(s)
        .ok_or_else(|| LuceneError::IllegalArgument(format!("For input string: \"{s}\"")))
}

/// `Boolean.parseBoolean(String)`: `"true".equalsIgnoreCase(s)`. No character
/// outside ASCII case-maps to a letter of `true`, so an ASCII comparison is
/// the same test.
fn parse_boolean(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

/// `String.equalsIgnoreCase(String)`: the same UTF-16 length, then every pair
/// of code points equal as they are, after `Character.toUpperCase`, or after
/// `Character.toLowerCase(Character.toUpperCase(..))` (the JDK's
/// `StringUTF16.regionMatchesCI`, which compares supplementary characters by
/// code point).
fn java_equals_ignore_case(a: &str, b: &str) -> bool {
    if a.encode_utf16().count() != b.encode_utf16().count() {
        return false;
    }
    let mut left = a.chars();
    let mut right = b.chars();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) => {
                if x == y {
                    continue;
                }
                let upper_x = case_folding::to_upper_case(x as i32);
                let upper_y = case_folding::to_upper_case(y as i32);
                if upper_x == upper_y
                    || case_folding::to_lower_case(upper_x) == case_folding::to_lower_case(upper_y)
                {
                    continue;
                }
                return false;
            }
            _ => return false,
        }
    }
}

/// `Float.parseFloat(String)`: see [`AbstractAnalysisFactory::get_float`].
fn parse_float(input: &str) -> Result<f32> {
    let not_a_float = || LuceneError::IllegalArgument(format!("For input string: \"{input}\""));
    // `in = in.trim()`: the characters up to U+0020.
    let trimmed = input.trim_matches(|c: char| c <= '\u{20}');
    if trimmed.is_empty() {
        return Err(LuceneError::IllegalArgument("empty String".to_string()));
    }
    let (negative, rest) = match trimmed.as_bytes()[0] {
        b'-' => (true, &trimmed[1..]),
        b'+' => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let signed = |value: f32| if negative { -value } else { value };
    if rest == "NaN" {
        return Ok(f32::NAN);
    }
    if rest == "Infinity" {
        return Ok(signed(f32::INFINITY));
    }
    if let Some(hex) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        return parse_hex_float(hex).map(signed).ok_or_else(not_a_float);
    }
    // Decimal: digits, an optional point and digits (at least one digit in
    // all), an optional exponent with at least one digit, an optional suffix.
    let core = rest.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(rest);
    let bytes = core.as_bytes();
    let mut i = 0;
    let int_digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    i += int_digits;
    let mut frac_digits = 0;
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        frac_digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
        i += frac_digits;
    }
    if int_digits + frac_digits == 0 {
        return Err(not_a_float());
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp_digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
        if exp_digits == 0 {
            return Err(not_a_float());
        }
        i += exp_digits;
    }
    if i != bytes.len() {
        return Err(not_a_float());
    }
    // Rust's parser rounds a decimal string to the nearest `f32`, ties to
    // even, as `Float.parseFloat` does, and accepts exactly this grammar.
    core.parse::<f32>().map(signed).map_err(|_| not_a_float())
}

/// The hexadecimal significand and binary exponent after `0x` — Java's
/// `(\p{XDigit}+\.?|\p{XDigit}*\.\p{XDigit}+)[pP][+-]?\d+[fFdD]?` — rounded to
/// the nearest `float`, ties to even, or `None` when `hex` does not match.
fn parse_hex_float(hex: &str) -> Option<f32> {
    let hex = hex.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(hex);
    let p = hex.find(['p', 'P'])?;
    let (significand, exponent) = (&hex[..p], &hex[p + 1..]);
    let (exponent_negative, exponent_digits) = match exponent.as_bytes().first()? {
        b'+' => (false, &exponent[1..]),
        b'-' => (true, &exponent[1..]),
        _ => (false, exponent),
    };
    if exponent_digits.is_empty() || !exponent_digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (int_part, frac_part) = match significand.find('.') {
        Some(dot) => (&significand[..dot], &significand[dot + 1..]),
        None => (significand, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.bytes().all(|b| b.is_ascii_hexdigit())
        || !frac_part.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    // A saturated exponent is as good as the exact one: the value is then far
    // outside the `float` range either way.
    let exponent_value = exponent_digits
        .bytes()
        .fold(0i64, |acc, b| (acc * 10 + i64::from(b - b'0')).min(1 << 40));
    let mut binary_exponent = if exponent_negative {
        -exponent_value
    } else {
        exponent_value
    } - 4 * frac_part.len() as i64;
    // Up to 64 significant bits, and whether any non-zero digit was dropped.
    let mut mantissa: u64 = 0;
    let mut sticky = false;
    for digit in int_part.bytes().chain(frac_part.bytes()) {
        let value = u64::from((digit as char).to_digit(16)?);
        if mantissa >> 60 == 0 {
            mantissa = (mantissa << 4) | value;
        } else {
            binary_exponent += 4;
            sticky |= value != 0;
        }
    }
    if mantissa == 0 {
        return Some(0.0);
    }
    let top_bit = 63 - i64::from(mantissa.leading_zeros());
    // The exponent of the result's lowest significand bit: 24 bits for a
    // normal float, down to 2^-149 for a subnormal one.
    let lowest = (top_bit + binary_exponent - 23).max(-149);
    let shift = lowest - binary_exponent;
    let significand = if shift <= 0 {
        // Exact: no bit is dropped (and `sticky` is set only when more than
        // 60 bits were read, which makes `shift` positive).
        mantissa << (-shift)
    } else if shift > 64 {
        // Below half the smallest subnormal.
        0
    } else {
        let dropped = if shift == 64 {
            mantissa
        } else {
            mantissa & ((1u64 << shift) - 1)
        };
        let half = 1u64 << (shift - 1);
        let kept = if shift == 64 { 0 } else { mantissa >> shift };
        let round_up = dropped > half || (dropped == half && (sticky || kept & 1 == 1));
        kept + u64::from(round_up)
    };
    // `significand` fits 25 bits and `lowest` is at least -149, so the
    // product is exact in `f64` until it overflows the `float` range, where
    // the conversion yields infinity.
    let scale = if lowest > 1023 {
        f64::INFINITY
    } else {
        f64::from_bits(((lowest + 1023) as u64) << 52)
    };
    Some((significand as f64 * scale) as f32)
}

/// Builds a tokenizer from a configuration.
///
/// Equivalent to `org.apache.lucene.analysis.TokenizerFactory`. Its static
/// entry points are [`tokenizer_for_name`], [`tokenizer_lookup_class`],
/// [`available_tokenizers`], [`find_tokenizer_spi_name`] and
/// [`reload_tokenizers`].
pub trait TokenizerFactory: Send + Sync + std::fmt::Debug {
    /// Returns the binary name of the Lucene class this factory ports.
    ///
    /// Stands in for `getClass().getName()`, which Rust cannot answer: there is
    /// no runtime class object, and `std::any::type_name` names the Rust type,
    /// not the Java one. `AbstractAnalysisFactory.getClassArg()` falls back to
    /// it, so the name has to be carried by the factory itself. There is no
    /// default, as for
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
/// Equivalent to `org.apache.lucene.analysis.TokenFilterFactory`. Its static
/// entry points are [`token_filter_for_name`], [`token_filter_lookup_class`],
/// [`available_token_filters`], [`find_token_filter_spi_name`] and
/// [`reload_token_filters`].
pub trait TokenFilterFactory: Send + Sync + std::fmt::Debug {
    /// Returns the binary name of the Lucene class this factory ports.
    ///
    /// Stands in for `getClass().getName()`; see
    /// [`TokenizerFactory::java_class_name`].
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
/// Equivalent to `org.apache.lucene.analysis.CharFilterFactory`. Its static
/// entry points are [`char_filter_for_name`], [`char_filter_lookup_class`],
/// [`available_char_filters`], [`find_char_filter_spi_name`] and
/// [`reload_char_filters`].
pub trait CharFilterFactory: Send + Sync + std::fmt::Debug {
    /// Returns the binary name of the Lucene class this factory ports.
    ///
    /// Stands in for `getClass().getName()`; see
    /// [`TokenizerFactory::java_class_name`].
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

/// What reflection finds for a factory class's `NAME` field, which
/// `AnalysisSPILoader.lookupSPIName` reads as the factory's SPI name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SPINameField {
    /// `public static final String NAME`, declared on the class itself: the
    /// SPI name.
    Declared(&'static str),
    /// A public `NAME` field that does not qualify: inherited from a
    /// superclass, not `static` or not `final`, or not a `String`.
    /// `lookupSPIName` throws `IllegalStateException("No SPI name defined.")`.
    Unsuitable,
    /// No public `NAME` field at all: `Class.getField("NAME")` throws
    /// `NoSuchFieldException("NAME")`.
    Missing,
}

/// The public `Map<String, String>` constructor of an analysis factory class,
/// returning the factory as a `T` (`dyn TokenizerFactory` and its siblings).
pub type AnalysisFactoryConstructor<T> = fn(&mut HashMap<String, String>) -> Result<Arc<T>>;

/// The class of an analysis factory, as a `META-INF/services` line names it
/// and as `AnalysisSPILoader` holds it.
///
/// Stands in for the `Class<? extends S>` Java's loader stores
/// (`AnalysisSPILoader.java:41`): its binary name (`Class.getName()`), what
/// reflection finds for its `NAME` field, and its public
/// `Map<String, String>` constructor, which `newFactoryClassInstance` looks up
/// with `getConstructor(Map.class)` — `None` for a class that has none.
pub struct AnalysisFactoryClass<T: ?Sized> {
    name: &'static str,
    spi_name: SPINameField,
    map_constructor: Option<AnalysisFactoryConstructor<T>>,
}

impl<T: ?Sized> AnalysisFactoryClass<T> {
    /// Describes the factory class whose binary name is `name`.
    pub const fn new(
        name: &'static str,
        spi_name: SPINameField,
        map_constructor: Option<AnalysisFactoryConstructor<T>>,
    ) -> Self {
        Self {
            name,
            spi_name,
            map_constructor,
        }
    }

    /// Returns the class's binary name, Java's `Class.getName()`.
    pub fn get_name(&self) -> &'static str {
        self.name
    }

    /// Returns what reflection finds for the class's `NAME` field.
    pub fn spi_name_field(&self) -> SPINameField {
        self.spi_name
    }
}

impl<T: ?Sized> Clone for AnalysisFactoryClass<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name,
            spi_name: self.spi_name,
            map_constructor: self.map_constructor,
        }
    }
}

impl<T: ?Sized> std::fmt::Debug for AnalysisFactoryClass<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisFactoryClass")
            .field("name", &self.name)
            .field("spi_name", &self.spi_name)
            .field("map_constructor", &self.map_constructor.is_some())
            .finish()
    }
}

/// A registry of analysis factory classes of one kind, by SPI name.
///
/// Equivalent to `org.apache.lucene.analysis.AnalysisSPILoader`. It scans a
/// [`ClassLoader`]'s `META-INF/services` files (see
/// [`load_service_classes`]) for [`AnalysisFactoryClass`]es and instantiates
/// them on demand with their argument map.
///
/// **Divergence from Lucene 10.5.0.** Java's `ServiceConfigurationError` has
/// no [`LuceneError`] counterpart; the errors Java raises as one are
/// [`LuceneError::IllegalArgument`] here (with Java's message, and its cause
/// as [`LuceneError::IllegalArgumentWithCause`]). And Java keeps the original
/// names in `Set.copyOf(originalNames)`, an immutable set whose iteration
/// order is unspecified (it varies from one JVM run to the next); this port
/// keeps them in registration order, which
/// [`available_services`](Self::available_services) and the lookup error
/// report.
pub struct AnalysisSPILoader<T: ?Sized> {
    /// Keyed by the lower-cased name, as Java's `services` map is.
    services: HashMap<String, Arc<AnalysisFactoryClass<T>>>,
    /// The names as they were registered, which `availableServices` returns.
    original_names: Vec<String>,
    /// The service type's binary name, Java's `clazz.getName()`: the name of
    /// the service files scanned and of the type in the lookup error.
    clazz_name: &'static str,
}

impl<T: ?Sized> std::fmt::Debug for AnalysisSPILoader<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisSPILoader")
            .field("clazz_name", &self.clazz_name)
            .field("available_services", &self.original_names)
            .finish()
    }
}

impl<T: ?Sized + 'static> AnalysisSPILoader<T> {
    /// Creates a registry for the factory type `clazz`, scanning
    /// `classloader` — or, when it is `None`, the loader that defines
    /// `clazz`.
    ///
    /// Equivalent to `AnalysisSPILoader(Class<S>)` and
    /// `AnalysisSPILoader(Class<S>, ClassLoader)` (`AnalysisSPILoader.java:47-63`):
    /// when the defining loader of `clazz` is not `classloader` or one of its
    /// parents ([`ClassLoaderUtils::is_parent_class_loader`]), it is reloaded
    /// first; then `classloader` is reloaded.
    ///
    /// # Errors
    ///
    /// Returns whatever [`reload`](Self::reload) returns.
    pub fn new(clazz: &ServiceClass, classloader: Option<&Arc<ClassLoader>>) -> Result<Self> {
        let mut loader = Self {
            services: HashMap::new(),
            original_names: Vec::new(),
            clazz_name: clazz.get_name(),
        };
        // if clazz' classloader is not a parent of the given one, we scan
        // clazz's classloader, too:
        let clazz_classloader = clazz.get_class_loader();
        let classloader = classloader.unwrap_or(clazz_classloader);
        if !ClassLoaderUtils::is_parent_class_loader(clazz_classloader, classloader) {
            loader.reload(clazz_classloader)?;
        }
        loader.reload(classloader)?;
        Ok(loader)
    }

    /// Reloads the factory classes from `classloader`, adding the new ones.
    ///
    /// Equivalent to `AnalysisSPILoader.reload(ClassLoader)`
    /// (`AnalysisSPILoader.java:76-133`): for every class the scan reaches
    /// ([`load_service_classes`]), its SPI name is read with
    /// [`lookup_spi_name`](Self::lookup_spi_name) and checked against
    /// `^[a-zA-Z][a-zA-Z0-9_]+$`; the map is keyed by the lower-cased name and
    /// the original spelling kept for
    /// [`available_services`](Self::available_services). Only new names are
    /// added — a class placed earlier wins and an existing one is never
    /// replaced — and the update is all or nothing: the new map and name list
    /// are built on copies and installed only once the whole scan has been
    /// accepted.
    ///
    /// `classloader` cannot be null here, so Java's
    /// `Objects.requireNonNull(classloader, "classloader")` has nothing to
    /// check.
    ///
    /// # Errors
    ///
    /// Returns, standing in for Java's `ServiceConfigurationError` and with
    /// its message, [`LuceneError::IllegalArgumentWithCause`] when a class has
    /// no usable `NAME` field (the cause is what
    /// [`lookup_spi_name`](Self::lookup_spi_name) returned), and
    /// [`LuceneError::IllegalArgument`] when a name is not a valid SPI name,
    /// when the scan reaches a class that is not of this factory type, or
    /// when the lookup keys and the original names disagree in number.
    pub fn reload(&mut self, classloader: &Arc<ClassLoader>) -> Result<()> {
        let mut services = self.services.clone();
        let mut original_names = self.original_names.clone();
        for provider in
            load_service_classes::<AnalysisFactoryClass<T>>(self.clazz_name, classloader)
        {
            let (_, service) = provider?;
            let original_name = Self::lookup_spi_name(service).map_err(|cause| {
                LuceneError::illegal_argument_with_cause(
                    format!(
                        "The class name {} has no service name field: [public static final \
                         String NAME]",
                        service.get_name()
                    ),
                    Arc::new(cause),
                )
            })?;
            let name = original_name.to_lowercase();
            if !Self::is_valid_name(original_name) {
                return Err(LuceneError::IllegalArgument(format!(
                    "The name {original_name} for {} is invalid: Allowed characters are (English) \
                     alphabet, digits, and underscore. It should be started with an alphabet.",
                    service.get_name()
                )));
            }
            // only add the first one for each name, later services will be
            // ignored; this allows to place services before others in the
            // classpath to make them used instead of others
            if let std::collections::hash_map::Entry::Vacant(slot) = services.entry(name) {
                slot.insert(Arc::new(service.clone()));
                // preserve (case-sensitive) original name for reference
                original_names.push(original_name.to_string());
            }
        }
        // make sure that the number of lookup keys is same to the number of
        // original names
        if services.len() != original_names.len() {
            return Err(LuceneError::IllegalArgument(
                "Service lookup key set is inconsistent with original name set!".to_string(),
            ));
        }
        self.services = services;
        self.original_names = original_names;
        Ok(())
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

    /// Instantiates the factory registered under `name` with `args`.
    ///
    /// Equivalent to `AnalysisSPILoader.newInstance(String, Map)`
    /// (`AnalysisSPILoader.java:139-142`):
    /// [`lookup_class`](Self::lookup_class), then
    /// [`new_factory_class_instance`](Self::new_factory_class_instance).
    ///
    /// # Errors
    ///
    /// Returns whatever those two return.
    pub fn new_instance(&self, name: &str, args: &mut HashMap<String, String>) -> Result<Arc<T>> {
        let service = self.lookup_class(name)?;
        Self::new_factory_class_instance(&service, args)
    }

    /// Looks a factory class up by name, case-insensitively.
    ///
    /// Equivalent to `AnalysisSPILoader.lookupClass(String)`
    /// (`AnalysisSPILoader.java:144-159`), which lower-cases the name before
    /// consulting the map.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] with Java's message, naming the
    /// available services as `[a, b]`.
    pub fn lookup_class(&self, name: &str) -> Result<Arc<AnalysisFactoryClass<T>>> {
        self.services
            .get(&name.to_lowercase())
            .map(Arc::clone)
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "A SPI class of type {} with name '{name}' does not exist. You need to add \
                     the corresponding JAR file supporting this SPI to your classpath. The \
                     current classpath supports the following names: [{}]",
                    self.clazz_name,
                    self.original_names.join(", ")
                ))
            })
    }

    /// Returns every registered name, with its original spelling.
    ///
    /// Equivalent to `AnalysisSPILoader.availableServices()`; see the type's
    /// documentation for the order.
    pub fn available_services(&self) -> Vec<String> {
        self.original_names.clone()
    }

    /// Returns the SPI name `service` declares.
    ///
    /// Equivalent to `AnalysisSPILoader.lookupSPIName(Class)`
    /// (`AnalysisSPILoader.java:175-186`), which reads the class's `NAME`
    /// field reflectively and accepts it only when it is static, final,
    /// declared on that very class and a `String`.
    ///
    /// # Errors
    ///
    /// Returns, for a class without a public `NAME` field, the
    /// `NoSuchFieldException("NAME")` Java throws, as [`LuceneError::Other`]
    /// with its message `NAME` ([`LuceneError`] has no reflective exception,
    /// as for `CommandLineUtil`'s), and for a field that does not qualify
    /// [`LuceneError::IllegalState`] with `No SPI name defined.`.
    pub fn lookup_spi_name(service: &AnalysisFactoryClass<T>) -> Result<&'static str> {
        match service.spi_name {
            SPINameField::Declared(name) => Ok(name),
            SPINameField::Unsuitable => Err(LuceneError::illegal_state("No SPI name defined.")),
            SPINameField::Missing => Err(LuceneError::Other("NAME".to_string())),
        }
    }

    /// Instantiates `clazz` with `args`.
    ///
    /// Equivalent to `AnalysisSPILoader.newFactoryClassInstance(Class, Map)`
    /// (`AnalysisSPILoader.java:192-213`), which calls the class's public
    /// `Map<String, String>` constructor and lets the constructor's own
    /// failure surface unchanged.
    ///
    /// # Errors
    ///
    /// Returns whatever the constructor returns, and, for a class without such
    /// a constructor, [`LuceneError::UnsupportedOperation`] with Java's message
    /// `Factory <class> cannot be instantiated. This is likely due to missing
    /// Map<String,String> constructor.`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java attaches the
    /// `NoSuchMethodException` as the cause of the
    /// `UnsupportedOperationException`; [`LuceneError::UnsupportedOperation`]
    /// carries no cause. And Java wraps a checked exception the constructor
    /// throws in a `RuntimeException`; a factory's `Map` constructor declares
    /// none, and the port returns the constructor's error as it is.
    pub fn new_factory_class_instance(
        clazz: &AnalysisFactoryClass<T>,
        args: &mut HashMap<String, String>,
    ) -> Result<Arc<T>> {
        match clazz.map_constructor {
            Some(constructor) => constructor(args),
            None => Err(LuceneError::UnsupportedOperation(format!(
                "Factory {} cannot be instantiated. This is likely due to missing \
                 Map<String,String> constructor.",
                clazz.name
            ))),
        }
    }
}

/// The `IllegalStateException(e)` the three `findSPIName` methods throw for
/// what [`AnalysisSPILoader::lookup_spi_name`] threw: its message is
/// `e.toString()`, and `e` is its cause.
fn find_spi_name_failure(cause: LuceneError) -> LuceneError {
    let message = match &cause {
        LuceneError::IllegalState { message, .. } => {
            format!("java.lang.IllegalStateException: {message}")
        }
        LuceneError::Other(message) => format!("java.lang.NoSuchFieldException: {message}"),
        other => other.to_string(),
    };
    LuceneError::illegal_state_with_cause(message, Arc::new(cause))
}

// ---------------------------------------------------------------------------
// The global loaders behind the factories' static entry points
// ---------------------------------------------------------------------------

/// `TokenizerFactory.class`: the service type's binary name and the loader
/// that defines it, `lucene-core`'s.
pub fn tokenizer_factory_class() -> ServiceClass {
    ServiceClass::new(
        "org.apache.lucene.analysis.TokenizerFactory",
        Arc::clone(lucene_core_class_loader()),
    )
}

/// `TokenFilterFactory.class`; see [`tokenizer_factory_class`].
pub fn token_filter_factory_class() -> ServiceClass {
    ServiceClass::new(
        "org.apache.lucene.analysis.TokenFilterFactory",
        Arc::clone(lucene_core_class_loader()),
    )
}

/// `CharFilterFactory.class`; see [`tokenizer_factory_class`].
pub fn char_filter_factory_class() -> ServiceClass {
    ServiceClass::new(
        "org.apache.lucene.analysis.CharFilterFactory",
        Arc::clone(lucene_core_class_loader()),
    )
}

/// The loader behind [`TokenizerFactory`]'s static entry points.
///
/// Equivalent to `TokenizerFactory.Holder.LOADER`,
/// `new AnalysisSPILoader<>(TokenizerFactory.class)`, which Java hides in a
/// holder class so the factories are only discovered when first needed. A
/// `LazyLock` gives the same deferred initialisation; the `RwLock` is what lets
/// [`reload_tokenizers`] add to it afterwards, which in Java is `reload`'s
/// `synchronized`. `lucene-core`'s loader lists one tokenizer factory,
/// `StandardTokenizerFactory`.
static TOKENIZER_LOADER: LazyLock<RwLock<AnalysisSPILoader<dyn TokenizerFactory>>> =
    LazyLock::new(|| {
        RwLock::new(
            AnalysisSPILoader::new(&tokenizer_factory_class(), None)
                .expect("INVARIANT: lucene-core's tokenizer factories have valid SPI names"),
        )
    });

/// The loader behind [`TokenFilterFactory`]'s static entry points.
///
/// Equivalent to `TokenFilterFactory.Holder.LOADER`. `lucene-core` lists no
/// token filter factory, so it starts empty.
static TOKEN_FILTER_LOADER: LazyLock<RwLock<AnalysisSPILoader<dyn TokenFilterFactory>>> =
    LazyLock::new(|| {
        RwLock::new(
            AnalysisSPILoader::new(&token_filter_factory_class(), None)
                .expect("INVARIANT: lucene-core lists no token filter factory"),
        )
    });

/// The loader behind [`CharFilterFactory`]'s static entry points.
///
/// Equivalent to `CharFilterFactory.Holder.LOADER`. `lucene-core` lists no
/// char filter factory, so it starts empty.
static CHAR_FILTER_LOADER: LazyLock<RwLock<AnalysisSPILoader<dyn CharFilterFactory>>> =
    LazyLock::new(|| {
        RwLock::new(
            AnalysisSPILoader::new(&char_filter_factory_class(), None)
                .expect("INVARIANT: lucene-core lists no char filter factory"),
        )
    });

/// Builds the tokenizer factory registered under `name` from `args`.
///
/// Equivalent to `TokenizerFactory.forName(String, Map)`,
/// `Holder.getLoader().newInstance(name, args)`. The class is looked up under
/// the loader's lock and instantiated after it is released, so a factory
/// constructor may itself look factories up.
///
/// # Errors
///
/// Returns whatever [`AnalysisSPILoader::new_instance`] returns.
pub fn tokenizer_for_name(
    name: &str,
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn TokenizerFactory>> {
    let class = tokenizer_lookup_class(name)?;
    AnalysisSPILoader::new_factory_class_instance(&class, args)
}

/// Looks a tokenizer factory class up by name.
///
/// Equivalent to `TokenizerFactory.lookupClass(String)`.
///
/// # Errors
///
/// Returns whatever [`AnalysisSPILoader::lookup_class`] returns.
pub fn tokenizer_lookup_class(
    name: &str,
) -> Result<Arc<AnalysisFactoryClass<dyn TokenizerFactory>>> {
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

/// Returns the SPI name a tokenizer factory class declares.
///
/// Equivalent to `TokenizerFactory.findSPIName(Class)`, which is
/// [`AnalysisSPILoader::lookup_spi_name`] with its failure wrapped.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalState`] whose message is the failure's
/// `toString()` and whose cause is the failure, as Java's
/// `new IllegalStateException(e)`.
pub fn find_tokenizer_spi_name(
    service_class: &AnalysisFactoryClass<dyn TokenizerFactory>,
) -> Result<&'static str> {
    AnalysisSPILoader::lookup_spi_name(service_class).map_err(find_spi_name_failure)
}

/// Reloads the tokenizer factories from `classloader`, adding the ones not
/// seen yet.
///
/// Equivalent to `TokenizerFactory.reloadTokenizers(ClassLoader)`,
/// `Holder.getLoader().reload(classloader)`: only new factories are added,
/// existing ones are never removed or replaced.
///
/// # Errors
///
/// Returns whatever [`AnalysisSPILoader::reload`] returns.
pub fn reload_tokenizers(classloader: &Arc<ClassLoader>) -> Result<()> {
    TOKENIZER_LOADER
        .write()
        .expect("INVARIANT: the tokenizer loader lock is never poisoned")
        .reload(classloader)
}

/// Builds the token-filter factory registered under `name` from `args`.
///
/// Equivalent to `TokenFilterFactory.forName(String, Map)`; see
/// [`tokenizer_for_name`].
///
/// # Errors
///
/// Returns whatever [`AnalysisSPILoader::new_instance`] returns.
pub fn token_filter_for_name(
    name: &str,
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn TokenFilterFactory>> {
    let class = token_filter_lookup_class(name)?;
    AnalysisSPILoader::new_factory_class_instance(&class, args)
}

/// Looks a token-filter factory class up by name.
///
/// Equivalent to `TokenFilterFactory.lookupClass(String)`.
///
/// # Errors
///
/// Returns whatever [`AnalysisSPILoader::lookup_class`] returns.
pub fn token_filter_lookup_class(
    name: &str,
) -> Result<Arc<AnalysisFactoryClass<dyn TokenFilterFactory>>> {
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

/// Returns the SPI name a token-filter factory class declares.
///
/// Equivalent to `TokenFilterFactory.findSPIName(Class)`; see
/// [`find_tokenizer_spi_name`].
///
/// # Errors
///
/// As [`find_tokenizer_spi_name`].
pub fn find_token_filter_spi_name(
    service_class: &AnalysisFactoryClass<dyn TokenFilterFactory>,
) -> Result<&'static str> {
    AnalysisSPILoader::lookup_spi_name(service_class).map_err(find_spi_name_failure)
}

/// Reloads the token-filter factories from `classloader`.
///
/// Equivalent to `TokenFilterFactory.reloadTokenFilters(ClassLoader)`.
///
/// # Errors
///
/// Returns whatever [`AnalysisSPILoader::reload`] returns.
pub fn reload_token_filters(classloader: &Arc<ClassLoader>) -> Result<()> {
    TOKEN_FILTER_LOADER
        .write()
        .expect("INVARIANT: the token-filter loader lock is never poisoned")
        .reload(classloader)
}

/// Builds the character-filter factory registered under `name` from `args`.
///
/// Equivalent to `CharFilterFactory.forName(String, Map)`; see
/// [`tokenizer_for_name`].
///
/// # Errors
///
/// Returns whatever [`AnalysisSPILoader::new_instance`] returns.
pub fn char_filter_for_name(
    name: &str,
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn CharFilterFactory>> {
    let class = char_filter_lookup_class(name)?;
    AnalysisSPILoader::new_factory_class_instance(&class, args)
}

/// Looks a character-filter factory class up by name.
///
/// Equivalent to `CharFilterFactory.lookupClass(String)`.
///
/// # Errors
///
/// Returns whatever [`AnalysisSPILoader::lookup_class`] returns.
pub fn char_filter_lookup_class(
    name: &str,
) -> Result<Arc<AnalysisFactoryClass<dyn CharFilterFactory>>> {
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

/// Returns the SPI name a character-filter factory class declares.
///
/// Equivalent to `CharFilterFactory.findSPIName(Class)`; see
/// [`find_tokenizer_spi_name`].
///
/// # Errors
///
/// As [`find_tokenizer_spi_name`].
pub fn find_char_filter_spi_name(
    service_class: &AnalysisFactoryClass<dyn CharFilterFactory>,
) -> Result<&'static str> {
    AnalysisSPILoader::lookup_spi_name(service_class).map_err(find_spi_name_failure)
}

/// Reloads the character-filter factories from `classloader`.
///
/// Equivalent to `CharFilterFactory.reloadCharFilters(ClassLoader)`.
///
/// # Errors
///
/// Returns whatever [`AnalysisSPILoader::reload`] returns.
pub fn reload_char_filters(classloader: &Arc<ClassLoader>) -> Result<()> {
    CHAR_FILTER_LOADER
        .write()
        .expect("INVARIANT: the char-filter loader lock is never poisoned")
        .reload(classloader)
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
mod parse_exception_tests {
    use super::*;

    /// `catch (ParseException pe) { throw new IllegalArgumentException(pe); }`
    /// (`AbstractAnalysisFactory.java:101-105`). Measured against
    /// `lucene-core-10.5.0.jar` (`oracle/fix/MiscFix.java`, item 16) with
    /// `luceneMatchVersion=x.y`: `IllegalArgumentException:
    /// java.text.ParseException: failed to parse lenient version string
    /// "x.y": Failed to parse major version from "X" (got: X.Y)`, whose cause
    /// is that `ParseException`.
    #[test]
    fn an_unparsable_match_version_wraps_the_parse_exception_as_java_does() {
        let mut args = HashMap::from([(LUCENE_MATCH_VERSION_PARAM.to_string(), "x.y".to_string())]);
        let expected = "failed to parse lenient version string \"x.y\": Failed to parse major \
                        version from \"X\" (got: X.Y)";
        match AbstractAnalysisFactory::new(&mut args) {
            Err(LuceneError::IllegalArgumentWithCause { message, cause }) => {
                assert_eq!(message, format!("java.text.ParseException: {expected}"));
                assert!(matches!(
                    cause.as_ref(),
                    LuceneError::IllegalArgumentWithCause { message, .. } if message == expected
                ));
            }
            other => panic!("expected the wrapped ParseException, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod spi_tests {
    use super::*;
    use crate::util::resource_loader::ServiceProvider;

    const TOKENIZER: &str = "org.apache.lucene.analysis.TokenizerFactory";

    /// A tokenizer factory of the tests' own, holding what its
    /// `AbstractAnalysisFactory` read.
    #[derive(Debug)]
    struct TestTokenizerFactory {
        base: AbstractAnalysisFactory,
    }

    impl TokenizerFactory for TestTokenizerFactory {
        fn java_class_name(&self) -> &'static str {
            "org.example.TestTokenizerFactory"
        }

        /// Refuses to create when the version was marked explicit, so that a
        /// test can observe the flag through the trait.
        fn create(&self) -> Result<Box<dyn TokenizerLogic>> {
            if self.base.is_explicit_lucene_match_version() {
                return Err(LuceneError::illegal_state("explicit version"));
            }
            Ok(Box::new(
                crate::analysis::standard::StandardTokenizerLogic::new(),
            ))
        }
    }

    fn test_factory(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenizerFactory>> {
        Ok(Arc::new(TestTokenizerFactory {
            base: AbstractAnalysisFactory::new(args)?,
        }))
    }

    /// A `META-INF/services/org.apache.lucene.analysis.TokenizerFactory` line
    /// naming the factory class `class` with the given `NAME` field and `Map`
    /// constructor.
    fn line(
        class: &'static str,
        spi_name: SPINameField,
        map_constructor: Option<AnalysisFactoryConstructor<dyn TokenizerFactory>>,
    ) -> ServiceProvider {
        ServiceProvider::new(
            TOKENIZER,
            class,
            AnalysisFactoryClass::<dyn TokenizerFactory>::new(class, spi_name, map_constructor),
        )
    }

    /// `SpiOracle` A1 and A9 (`oracle/h2/SpiOracle.java`): `lucene-core`
    /// lists one analysis factory, `standard`, and no token or char filter;
    /// `forName` builds it from its argument map and refuses a leftover
    /// argument with Java's message, and a name that is not registered with
    /// the loader's.
    #[test]
    fn the_default_loaders_hold_what_lucene_core_lists() {
        assert_eq!(available_tokenizers(), vec!["standard".to_string()]);
        assert!(available_token_filters().is_empty());
        assert!(available_char_filters().is_empty());

        let mut args = HashMap::from([("maxTokenLength".to_string(), "7".to_string())]);
        let factory = tokenizer_for_name("Standard", &mut args).unwrap();
        assert_eq!(
            factory.java_class_name(),
            "org.apache.lucene.analysis.standard.StandardTokenizerFactory"
        );
        assert_eq!(
            find_tokenizer_spi_name(&tokenizer_lookup_class("STANDARD").unwrap()).unwrap(),
            "standard"
        );
    }

    /// `SpiOracle` A9: Java printed `Unknown parameters: {foo=bar, zap=1}`.
    #[test]
    fn a_leftover_argument_is_reported_as_the_map() {
        let mut leftovers = HashMap::from([
            ("maxTokenLength".to_string(), "7".to_string()),
            ("foo".to_string(), "bar".to_string()),
            ("zap".to_string(), "1".to_string()),
        ]);
        match tokenizer_for_name("Standard", &mut leftovers) {
            Err(LuceneError::IllegalArgument(message)) => {
                assert_eq!(message, "Unknown parameters: {foo=bar, zap=1}")
            }
            other => panic!("expected the leftover-argument error, got {other:?}"),
        }
    }

    /// `SpiOracle` A9: `lucene-core` lists no token filter, so a name fails
    /// with the loader's message and an empty list.
    #[test]
    fn an_unregistered_name_fails_with_the_loaders_message() {
        match token_filter_for_name("lowercase", &mut HashMap::new()) {
            Err(LuceneError::IllegalArgument(message)) => assert_eq!(
                message,
                "A SPI class of type org.apache.lucene.analysis.TokenFilterFactory with name \
                 'lowercase' does not exist. You need to add the corresponding JAR file \
                 supporting this SPI to your classpath. The current classpath supports the \
                 following names: []"
            ),
            other => panic!("expected the lookup error, got {other:?}"),
        }
    }

    /// `SpiOracle` A2: a loader that does not descend from the factory type's
    /// loader is scanned after it, so `standard` is still there — Java listed
    /// `[myTok, myTok2, standard]` — and lookups ignore case.
    #[test]
    fn a_non_descendant_loader_keeps_the_factory_loaders_classes() {
        let other = ClassLoader::with_services(
            "N2",
            None,
            vec![
                line(
                    "org.example.MyTok2",
                    SPINameField::Declared("myTok2"),
                    Some(test_factory),
                ),
                line(
                    "org.example.MyTok",
                    SPINameField::Declared("myTok"),
                    Some(test_factory),
                ),
            ],
        );
        let loader = AnalysisSPILoader::<dyn TokenizerFactory>::new(
            &tokenizer_factory_class(),
            Some(&other),
        )
        .unwrap();
        assert_eq!(
            loader.available_services(),
            vec![
                "standard".to_string(),
                "myTok2".to_string(),
                "myTok".to_string()
            ]
        );
        assert_eq!(
            loader.lookup_class("MYTOK").unwrap().get_name(),
            "org.example.MyTok"
        );
    }

    /// `SpiOracle` A3: a class without a usable `NAME` field, or with an
    /// invalid one, fails the whole reload with Java's messages and causes,
    /// and the loader keeps what it had (the valid class listed first is not
    /// added).
    #[test]
    fn refused_factory_classes_leave_the_loader_unchanged() {
        let cases = [
            (
                SPINameField::Missing,
                "The class name org.example.Refused has no service name field: [public static \
                 final String NAME]",
                Some(LuceneError::Other("NAME".to_string())),
            ),
            (
                SPINameField::Unsuitable,
                "The class name org.example.Refused has no service name field: [public static \
                 final String NAME]",
                Some(LuceneError::illegal_state("No SPI name defined.")),
            ),
            (
                SPINameField::Declared("1bad"),
                "The name 1bad for org.example.Refused is invalid: Allowed characters are \
                 (English) alphabet, digits, and underscore. It should be started with an \
                 alphabet.",
                None,
            ),
        ];
        for (field, expected, expected_cause) in cases {
            let mut loader =
                AnalysisSPILoader::<dyn TokenizerFactory>::new(&tokenizer_factory_class(), None)
                    .unwrap();
            let refused = ClassLoader::with_services(
                "X",
                None,
                vec![
                    line(
                        "org.example.MyTok",
                        SPINameField::Declared("myTok"),
                        Some(test_factory),
                    ),
                    line("org.example.Refused", field, Some(test_factory)),
                ],
            );
            let error = loader.reload(&refused).unwrap_err();
            match (&error, &expected_cause) {
                (LuceneError::IllegalArgument(message), None) => assert_eq!(message, expected),
                (LuceneError::IllegalArgumentWithCause { message, cause }, Some(want)) => {
                    assert_eq!(message, expected);
                    assert_eq!(cause.to_string(), want.to_string());
                    assert_eq!(cause.java_simple_name(), want.java_simple_name());
                }
                other => panic!("{field:?}: unexpected error {other:?}"),
            }
            assert_eq!(loader.available_services(), vec!["standard".to_string()]);
        }
    }

    /// `SpiOracle` A6 and A8: `newInstance` calls the class's `Map`
    /// constructor (and the factory's `luceneMatchVersion` does not mark the
    /// version explicit), refuses a class without one with Java's message,
    /// and `findSPIName` wraps the reflective failure as
    /// `IllegalStateException(e)`.
    #[test]
    fn new_instance_and_find_spi_name_report_as_java_does() {
        let classes = ClassLoader::with_services(
            "NM",
            None,
            vec![
                line(
                    "org.example.NoMapCtor",
                    SPINameField::Declared("noMap"),
                    None,
                ),
                line(
                    "org.example.MyTok",
                    SPINameField::Declared("myTok"),
                    Some(test_factory),
                ),
            ],
        );
        let loader = AnalysisSPILoader::<dyn TokenizerFactory>::new(
            &tokenizer_factory_class(),
            Some(&classes),
        )
        .unwrap();
        match loader.new_instance("NOMAP", &mut HashMap::new()) {
            Err(LuceneError::UnsupportedOperation(message)) => assert_eq!(
                message,
                "Factory org.example.NoMapCtor cannot be instantiated. This is likely due to \
                 missing Map<String,String> constructor."
            ),
            other => panic!("expected the missing-constructor error, got {other:?}"),
        }
        let mut args =
            HashMap::from([(LUCENE_MATCH_VERSION_PARAM.to_string(), "9.0.0".to_string())]);
        let made = loader.new_instance("mytok", &mut args).unwrap();
        assert_eq!(made.java_class_name(), "org.example.TestTokenizerFactory");
        assert!(made.create().is_ok(), "Java printed explicit=false");

        let missing = AnalysisFactoryClass::<dyn TokenizerFactory>::new(
            "org.example.NoName",
            SPINameField::Missing,
            None,
        );
        let unsuitable = AnalysisFactoryClass::<dyn TokenizerFactory>::new(
            "org.example.Inherits",
            SPINameField::Unsuitable,
            None,
        );
        for (class, expected) in [
            (&missing, "java.lang.NoSuchFieldException: NAME"),
            (
                &unsuitable,
                "java.lang.IllegalStateException: No SPI name defined.",
            ),
        ] {
            match find_tokenizer_spi_name(class) {
                Err(LuceneError::IllegalState { message, cause }) => {
                    assert_eq!(message, expected);
                    assert!(cause.is_some(), "the failure is the cause");
                }
                other => panic!("expected the wrapped failure, got {other:?}"),
            }
        }
    }

    /// A loader listing a class of another type under `service`.
    fn wrong_loader(service: &'static str) -> Arc<ClassLoader> {
        ClassLoader::with_services(
            "W",
            None,
            vec![ServiceProvider::new(service, "org.example.Wrong", 0u8)],
        )
    }

    /// The scan's refusal of [`wrong_loader`]'s class.
    fn assert_not_a_subtype(service: &str, result: Result<()>) {
        match result {
            Err(LuceneError::IllegalArgument(message)) => {
                assert_eq!(
                    message,
                    format!("{service}: org.example.Wrong not a subtype")
                )
            }
            other => panic!("{service}: expected the scan's error, got {other:?}"),
        }
    }

    /// `reloadTokenizers(ClassLoader)` is `Holder.getLoader().reload(classloader)`:
    /// a loader listing a class of another type makes it fail with the scan's
    /// error, and the global loader keeps what it had.
    #[test]
    fn reload_tokenizers_scans_the_given_loader() {
        assert_not_a_subtype(TOKENIZER, reload_tokenizers(&wrong_loader(TOKENIZER)));
        assert_eq!(available_tokenizers(), vec!["standard".to_string()]);
    }

    /// `reloadTokenFilters(ClassLoader)`; see [`reload_tokenizers_scans_the_given_loader`].
    #[test]
    fn reload_token_filters_scans_the_given_loader() {
        let service = "org.apache.lucene.analysis.TokenFilterFactory";
        assert_not_a_subtype(service, reload_token_filters(&wrong_loader(service)));
    }

    /// `reloadCharFilters(ClassLoader)`; see [`reload_tokenizers_scans_the_given_loader`].
    #[test]
    fn reload_char_filters_scans_the_given_loader() {
        let service = "org.apache.lucene.analysis.CharFilterFactory";
        assert_not_a_subtype(service, reload_char_filters(&wrong_loader(service)));
    }

    /// `SpiOracle` A9: the public no-argument constructor exists only for
    /// `ServiceLoader` and throws `defaultCtorException()`.
    #[test]
    fn the_no_argument_constructor_throws_the_default_ctor_exception() {
        match crate::analysis::standard::StandardTokenizerFactory::new_without_args() {
            Err(LuceneError::UnsupportedOperation(message)) => assert_eq!(
                message,
                "Analysis factories cannot be instantiated without arguments. Use applicable \
                 factory methods of TokenizerFactory, CharFilterFactory, or TokenFilterFactory."
            ),
            other => panic!("expected the default constructor exception, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod accessor_tests {
    use super::*;

    fn one(name: &str, value: &str) -> HashMap<String, String> {
        HashMap::from([(name.to_string(), value.to_string())])
    }

    fn message(error: LuceneError) -> String {
        match error {
            LuceneError::IllegalArgument(message) => message,
            other => panic!("expected an IllegalArgumentException, got {other:?}"),
        }
    }

    /// `MiscOracle` O2 (`oracle/h2/MiscOracle.java`): the constructor reads
    /// the version with `get` and leaves `isExplicitLuceneMatchVersion`
    /// `false`; `getClassArg` is the `class` argument or the factory's class.
    #[test]
    fn the_constructor_reads_the_version_and_leaves_it_implicit() {
        let mut args = one(LUCENE_MATCH_VERSION_PARAM, "9.0.0");
        let factory = AbstractAnalysisFactory::new(&mut args).unwrap();
        assert!(args.is_empty(), "the version is consumed");
        assert_eq!(factory.get_lucene_match_version().to_string(), "9.0.0");
        assert!(!factory.is_explicit_lucene_match_version());
    }

    /// `MiscOracle` O2: Java printed `getClassArg with=foo.Bar
    /// without=MiscOracle$F`.
    #[test]
    fn get_class_arg_is_the_class_argument_or_the_class_name() {
        let with_class = AbstractAnalysisFactory::new(&mut one("class", "foo.Bar")).unwrap();
        assert_eq!(with_class.get_class_arg("MiscOracle$F"), "foo.Bar");
        let without = AbstractAnalysisFactory::new(&mut HashMap::new()).unwrap();
        assert_eq!(without.get_class_arg("MiscOracle$F"), "MiscOracle$F");
    }

    /// `MiscOracle` O2: `Boolean.parseBoolean`.
    #[test]
    fn boolean_arguments_parse_as_java_does() {
        for (value, expected) in [
            ("TRUE", true),
            ("True", true),
            ("yes", false),
            ("false ", false),
            ("", false),
        ] {
            assert_eq!(
                AbstractAnalysisFactory::get_boolean(&mut one("b", value), "b", true),
                expected,
                "{value:?}"
            );
        }
        assert!(AbstractAnalysisFactory::require_boolean(&mut one("b", "tRuE"), "b").unwrap());
    }

    /// `MiscOracle` O2: `getChar` checks the UTF-16 length.
    #[test]
    fn get_char_counts_utf16_units() {
        for value in ["ab", "\u{1F600}", ""] {
            assert_eq!(
                message(
                    AbstractAnalysisFactory::get_char(&mut one("c", value), "c", 'z').unwrap_err()
                ),
                format!("c should be a char. \"{value}\" is invalid")
            );
        }
        assert_eq!(
            AbstractAnalysisFactory::get_char(&mut one("c", "é"), "c", 'z').unwrap(),
            'é'
        );
    }

    /// `MiscOracle` O2: `requireChar` is `charAt(0)`, which Java refused with
    /// `StringIndexOutOfBoundsException: Index 0 out of bounds for length 0`,
    /// and which returns the first UTF-16 unit: the high surrogate `d83d` of
    /// `\u{1F600}` (oracle `scratchpad/auditH/oracle/AuditHExtra.java`, row
    /// `requireChar`).
    #[test]
    fn require_char_of_an_empty_value_fails_as_char_at_does() {
        match AbstractAnalysisFactory::require_char(&mut one("c", ""), "c") {
            Err(LuceneError::Other(message)) => {
                assert_eq!(message, "Index 0 out of bounds for length 0")
            }
            other => panic!("expected the index error, got {other:?}"),
        }
        assert_eq!(
            AbstractAnalysisFactory::require_char(&mut one("c", "ab"), "c").unwrap(),
            u16::from(b'a')
        );
        assert_eq!(
            AbstractAnalysisFactory::require_char(&mut one("c", "\u{1F600}x"), "c").unwrap(),
            0xD83D
        );
    }

    /// `MiscOracle` O2: `Integer.parseInt` and its message.
    #[test]
    fn int_arguments_parse_as_java_does() {
        for (value, expected) in [("+5", 5), ("\u{0663}", 3), ("-2147483648", i32::MIN)] {
            assert_eq!(
                AbstractAnalysisFactory::get_int(&mut one("i", value), "i", 1).unwrap(),
                expected
            );
        }
        for value in ["x", " 5", "", "2147483648"] {
            assert_eq!(
                message(
                    AbstractAnalysisFactory::get_int(&mut one("i", value), "i", 1).unwrap_err()
                ),
                format!("For input string: \"{value}\"")
            );
        }
        assert_eq!(
            message(AbstractAnalysisFactory::require_int(&mut HashMap::new(), "i").unwrap_err()),
            "Configuration Error: missing parameter 'i'"
        );
    }

    /// `MiscOracle` O2: `Float.parseFloat`, value by value, as the bits Java
    /// printed or the `NumberFormatException` message.
    #[test]
    fn float_arguments_parse_as_java_does() {
        let values: [(&str, std::result::Result<u32, &str>); 47] = [
            ("1.5f", Ok(0x3fc00000)),
            (" 2 ", Ok(0x40000000)),
            ("\u{a0}2", Err("For input string: \"\u{a0}2\"")),
            ("inf", Err("For input string: \"inf\"")),
            ("Infinity", Ok(0x7f800000)),
            ("-Infinity", Ok(0xff800000)),
            ("+NaN", Ok(0x7fc00000)),
            ("NaN", Ok(0x7fc00000)),
            ("nan", Err("For input string: \"nan\"")),
            ("", Err("empty String")),
            ("  ", Err("empty String")),
            ("x", Err("For input string: \"x\"")),
            (" x ", Err("For input string: \" x \"")),
            ("0x1p3", Ok(0x41000000)),
            ("0x1.8p1f", Ok(0x40400000)),
            ("0x.8p1", Ok(0x3f800000)),
            ("0X1P-149", Ok(0x1)),
            ("0x1.fffffep127", Ok(0x7f7fffff)),
            ("0x1.ffffffp127", Ok(0x7f800000)),
            ("0x1p", Err("For input string: \"0x1p\"")),
            ("1e", Err("For input string: \"1e\"")),
            ("1e40", Ok(0x7f800000)),
            ("1e-50", Ok(0x0)),
            (".5", Ok(0x3f000000)),
            ("5.", Ok(0x40a00000)),
            ("+.5e-3d", Ok(0x3a03126f)),
            ("1_0", Err("For input string: \"1_0\"")),
            ("3.4028235e38", Ok(0x7f7fffff)),
            ("3.4028236e38", Ok(0x7f800000)),
            ("1.00000005960464477539062500000000001", Ok(0x3f800001)),
            ("1.000000059604644775390625", Ok(0x3f800000)),
            ("0x1.000001p0", Ok(0x3f800000)),
            ("0x1.0000018p0", Ok(0x3f800001)),
            ("0x1.00000100000000000000001p0", Ok(0x3f800001)),
            ("1.4e-45", Ok(0x1)),
            ("7.0e-46", Ok(0x0)),
            ("1e+5", Ok(0x47c35000)),
            ("1E5D", Ok(0x47c35000)),
            ("0x", Err("For input string: \"0x\"")),
            ("0xp1", Err("For input string: \"0xp1\"")),
            ("1.5e+", Err("For input string: \"1.5e+\"")),
            ("--1", Err("For input string: \"--1\"")),
            ("1.5ff", Err("For input string: \"1.5ff\"")),
            ("Infinityx", Err("For input string: \"Infinityx\"")),
            ("0x1.fffffefffffffffp127", Ok(0x7f7fffff)),
            ("-0x1p3", Ok(0xc1000000)),
            ("-0.0", Ok(0x80000000)),
        ];
        for (value, expected) in values {
            let got = AbstractAnalysisFactory::get_float(&mut one("f", value), "f", 9.0)
                .map(f32::to_bits)
                .map_err(message);
            assert_eq!(got, expected.map_err(str::to_string), "{value:?}");
        }
        assert_eq!(
            AbstractAnalysisFactory::require_float(&mut one("f", "2.5"), "f").unwrap(),
            2.5
        );
    }

    /// `MiscOracle` O2: `getSet` splits on `[^,\s]+` into a `HashSet` (Java
    /// printed `[a, b, c, a d]` and `[a, b]`, and `null` for no item), and
    /// `require(Map, String, Collection, boolean)` compares with
    /// `equalsIgnoreCase` and lists the allowed values as `[a, b]`.
    #[test]
    fn set_arguments_read_as_java_does() {
        let rendered = |value: &str| {
            AbstractAnalysisFactory::get_set(&mut one("s", value), "s")
                .map(|set| set.iter().cloned().collect::<Vec<_>>())
        };
        assert_eq!(
            rendered("a b,,c a\u{a0}d"),
            Some(vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "a\u{a0}d".to_string()
            ])
        );
        assert_eq!(rendered(" ,, "), None);
        assert_eq!(rendered("x"), Some(vec!["x".to_string()]));
        assert_eq!(
            rendered("b,a,b"),
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    /// `MiscOracle` O2: `require(Map, String, Collection, boolean)`.
    #[test]
    fn allowed_values_compare_with_equals_ignore_case() {
        let allowed = ["a", "B", "\u{01C6}", "k", "\u{10428}", "\u{DF}"];
        let refused =
            "Configuration Error: 'x' value must be one of [a, B, \u{01C6}, k, \u{10428}, \u{DF}]";
        for value in ["b", "\u{01C5}", "\u{212A}", "\u{10400}"] {
            assert_eq!(
                AbstractAnalysisFactory::require_one_of(&mut one("x", value), "x", &allowed, false)
                    .unwrap(),
                value
            );
            assert_eq!(
                message(
                    AbstractAnalysisFactory::require_one_of(
                        &mut one("x", value),
                        "x",
                        &allowed,
                        true
                    )
                    .unwrap_err()
                ),
                refused
            );
        }
        for value in ["c", "SS"] {
            assert_eq!(
                message(
                    AbstractAnalysisFactory::require_one_of(
                        &mut one("x", value),
                        "x",
                        &allowed,
                        false
                    )
                    .unwrap_err()
                ),
                refused
            );
        }
    }

    /// `MiscOracle` O2: `getWordSet` adds `StopFilter.makeStopSet` of each
    /// file's lines; with `ignoreCase` Java printed `[bar, foo]`.
    #[test]
    fn get_word_set_adds_each_files_stop_set() {
        struct Words;
        impl crate::util::resource_loader::ResourceLoader for Words {
            fn open_resource(&self, _resource: &str) -> Result<Box<dyn std::io::Read + Send>> {
                Ok(Box::new(std::io::Cursor::new(
                    b"Foo\nBAR\nfoo\n# c\n".to_vec(),
                )))
            }
        }
        let words = AbstractAnalysisFactory::get_word_set(&Words, Some("a.txt"), true)
            .unwrap()
            .expect("a file was named");
        assert_eq!(words.to_string(), "[bar, foo]");
    }
}

#[cfg(test)]
mod get_pattern_tests {
    use super::*;

    fn compile(pattern: &str) -> Result<fancy_regex::Regex> {
        let mut args = HashMap::from([("pattern".to_string(), pattern.to_string())]);
        AbstractAnalysisFactory::get_pattern(&mut args, "pattern", "ProbeFactory")
    }

    /// The matches `Matcher.find()` reports, as `<code-point start>:<match>`:
    /// each search resumes at the previous match's end, one character further
    /// after an empty match.
    fn find_all(regex: &fancy_regex::Regex, text: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(m) = regex.find_from_pos(text, from).expect("no engine error") {
            found.push(format!(
                "{}:{}",
                text[..m.start()].chars().count(),
                m.as_str()
            ));
            from = m.end();
            if m.start() == m.end() {
                match text[from..].chars().next() {
                    Some(c) => from += c.len_utf8(),
                    None => break,
                }
            }
        }
        found
    }

    /// Java's matcher has no backtracking limit. Java oracle
    /// (`scratchpad/auditH/regex/a_bt.py` through `RegexOracle.java`):
    /// `\bzzz\b` over 1.2 MB of text finds nothing, and `a$` over two million
    /// `b` and an `a` finds the `a` at 2000000; `fancy-regex`'s default limit
    /// of a million backtracking steps failed both with
    /// `BacktrackLimitExceeded`.
    #[test]
    fn a_search_over_a_long_input_has_no_backtracking_limit() {
        let words = "lorem ipsum ".repeat(100_000);
        assert!(find_all(&compile(r"\bzzz\b").unwrap(), &words).is_empty());
        let long = "b".repeat(2_000_000) + "a";
        assert_eq!(
            compile("a$")
                .unwrap()
                .find(&long)
                .unwrap()
                .map(|m| m.start()),
            Some(2_000_000)
        );
    }

    /// Java repeats a node with a counter whatever the count; the engine's
    /// automaton refused each of these, and its VM runs them. Java oracle
    /// (`scratchpad/auditH/regex/a_lim.py` through `RegexOracle.java`, over
    /// `aaa`): `\p{L}{1000}` `OK|`, `(?i)a{1,100000}` `OK|0:616161`,
    /// `a{0,1000000}` `OK|0:616161,3:`, `a{2147483647}` `OK|`,
    /// `(a){1000000}` `OK|`, `a{1000000}+` `OK|`.
    #[test]
    fn a_large_counted_repetition_compiles_and_matches_as_java() {
        for (pattern, expected) in [
            (r"\p{L}{1000}", vec![]),
            ("(?i)a{1,100000}", vec!["0:aaa"]),
            ("a{0,1000000}", vec!["0:aaa", "3:"]),
            ("a{2147483647}", vec![]),
            ("(a){1000000}", vec![]),
            ("a{1000000}+", vec![]),
        ] {
            let regex = compile(pattern).unwrap_or_else(|e| panic!("{pattern}: {e}"));
            assert_eq!(find_all(&regex, "aaa"), expected, "{pattern}");
        }
        let thousand = "x".repeat(1000);
        assert_eq!(
            find_all(&compile(r"\p{L}{1000}").unwrap(), &thousand),
            vec![format!("0:{thousand}")]
        );
    }

    /// The declared limit: a counted repetition of a body that can match the
    /// empty string keeps the engine's backtracking limit, and one too large
    /// for the automaton is refused — where Java (oracle
    /// `scratchpad/auditH/regex/a_ctl.py` and `a_slow7.py` through
    /// `RegexOracle.java`) answers `OK|9:` for `(?<!y)(?:x|){24}(?=b)` over
    /// `xxxxxxxxcb` and `OK|0:` for `^(?:a?){2147483647}` over `xxxx`. Without
    /// the limit, the first search finds Java's match after more than a
    /// million backtracking steps.
    #[test]
    fn a_counted_loop_over_an_empty_match_keeps_the_engine_limit() {
        let regex = compile("(?<!y)(?:x|){24}(?=b)").unwrap();
        assert!(matches!(
            regex.find("xxxxxxxxcb"),
            Err(fancy_regex::Error::RuntimeError(
                fancy_regex::RuntimeError::BacktrackLimitExceeded
            ))
        ));
        assert!(compile("^(?:a?){2147483647}").is_err());
    }
}
