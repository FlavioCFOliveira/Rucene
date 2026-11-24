// © 2022 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.DisplayOptions`.

use std::fmt;

/// Represents all the display options that are supported by CLDR such as
/// grammatical case, noun class, ... etc. It currently supports enums, but
/// may be extended in the future to have other types of data. It replaces
/// a `DisplayContext[]` as a method parameter.
///
/// Equivalent to `com.ibm.icu.text.DisplayOptions` (immutable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayOptions {
    grammatical_case: GrammaticalCase,
    noun_class: NounClass,
    plural_category: PluralCategory,
    capitalization: Capitalization,
    name_style: NameStyle,
    display_length: DisplayLength,
    substitute_handling: SubstituteHandling,
}

impl DisplayOptions {
    /// `builder()`: a builder with every option `UNDEFINED`.
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// `copyToBuilder()`: a builder with this object's options.
    pub fn copy_to_builder(&self) -> Builder {
        Builder::from_options(self)
    }

    /// `getGrammaticalCase()`.
    pub fn get_grammatical_case(&self) -> GrammaticalCase {
        self.grammatical_case
    }

    /// `getNounClass()`.
    pub fn get_noun_class(&self) -> NounClass {
        self.noun_class
    }

    /// `getPluralCategory()`.
    pub fn get_plural_category(&self) -> PluralCategory {
        self.plural_category
    }

    /// `getCapitalization()`.
    pub fn get_capitalization(&self) -> Capitalization {
        self.capitalization
    }

    /// `getNameStyle()`.
    pub fn get_name_style(&self) -> NameStyle {
        self.name_style
    }

    /// `getDisplayLength()`.
    pub fn get_display_length(&self) -> DisplayLength {
        self.display_length
    }

    /// `getSubstituteHandling()`.
    pub fn get_substitute_handling(&self) -> SubstituteHandling {
        self.substitute_handling
    }
}

/// Responsible for building `DisplayOptions`.
///
/// Equivalent to `DisplayOptions.Builder`; the setters return the builder,
/// as Java's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Builder {
    grammatical_case: GrammaticalCase,
    noun_class: NounClass,
    plural_category: PluralCategory,
    capitalization: Capitalization,
    name_style: NameStyle,
    display_length: DisplayLength,
    substitute_handling: SubstituteHandling,
}

impl Builder {
    /// The private constructor: every option `UNDEFINED`.
    fn new() -> Self {
        Self {
            grammatical_case: GrammaticalCase::Undefined,
            noun_class: NounClass::Undefined,
            plural_category: PluralCategory::Undefined,
            capitalization: Capitalization::Undefined,
            name_style: NameStyle::Undefined,
            display_length: DisplayLength::Undefined,
            substitute_handling: SubstituteHandling::Undefined,
        }
    }

    /// The private constructor `Builder(DisplayOptions)`.
    fn from_options(display_options: &DisplayOptions) -> Self {
        Self {
            grammatical_case: display_options.grammatical_case,
            noun_class: display_options.noun_class,
            plural_category: display_options.plural_category,
            capitalization: display_options.capitalization,
            name_style: display_options.name_style,
            display_length: display_options.display_length,
            substitute_handling: display_options.substitute_handling,
        }
    }

    /// `setGrammaticalCase(GrammaticalCase grammatical_case)`.
    pub fn set_grammatical_case(&mut self, grammatical_case: GrammaticalCase) -> &mut Self {
        self.grammatical_case = grammatical_case;
        self
    }

    /// `setNounClass(NounClass noun_class)`.
    pub fn set_noun_class(&mut self, noun_class: NounClass) -> &mut Self {
        self.noun_class = noun_class;
        self
    }

    /// `setPluralCategory(PluralCategory plural_category)`.
    pub fn set_plural_category(&mut self, plural_category: PluralCategory) -> &mut Self {
        self.plural_category = plural_category;
        self
    }

    /// `setCapitalization(Capitalization capitalization)`.
    pub fn set_capitalization(&mut self, capitalization: Capitalization) -> &mut Self {
        self.capitalization = capitalization;
        self
    }

    /// `setNameStyle(NameStyle name_style)`.
    pub fn set_name_style(&mut self, name_style: NameStyle) -> &mut Self {
        self.name_style = name_style;
        self
    }

    /// `setDisplayLength(DisplayLength display_length)`.
    pub fn set_display_length(&mut self, display_length: DisplayLength) -> &mut Self {
        self.display_length = display_length;
        self
    }

    /// `setSubstituteHandling(SubstituteHandling substitute_handling)`.
    pub fn set_substitute_handling(
        &mut self,
        substitute_handling: SubstituteHandling,
    ) -> &mut Self {
        self.substitute_handling = substitute_handling;
        self
    }

    /// `build()`.
    pub fn build(&self) -> DisplayOptions {
        DisplayOptions {
            grammatical_case: self.grammatical_case,
            noun_class: self.noun_class,
            plural_category: self.plural_category,
            capitalization: self.capitalization,
            name_style: self.name_style,
            display_length: self.display_length,
            substitute_handling: self.substitute_handling,
        }
    }
}

/// Represents all the grammatical noun classes that are supported by CLDR.
///
/// Equivalent to the enum `DisplayOptions.NounClass` (declaration order kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NounClass {
    /// `UNDEFINED`.
    Undefined,
    /// `OTHER`.
    Other,
    /// `NEUTER`.
    Neuter,
    /// `FEMININE`.
    Feminine,
    /// `MASCULINE`.
    Masculine,
    /// `ANIMATE`.
    Animate,
    /// `INANIMATE`.
    Inanimate,
    /// `PERSONAL`.
    Personal,
    /// `COMMON`.
    Common,
}

impl NounClass {
    /// `VALUES`: the unmodifiable list of every value.
    pub const VALUES: [NounClass; 9] = [
        NounClass::Undefined,
        NounClass::Other,
        NounClass::Neuter,
        NounClass::Feminine,
        NounClass::Masculine,
        NounClass::Animate,
        NounClass::Inanimate,
        NounClass::Personal,
        NounClass::Common,
    ];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            NounClass::Undefined => "UNDEFINED",
            NounClass::Other => "OTHER",
            NounClass::Neuter => "NEUTER",
            NounClass::Feminine => "FEMININE",
            NounClass::Masculine => "MASCULINE",
            NounClass::Animate => "ANIMATE",
            NounClass::Inanimate => "INANIMATE",
            NounClass::Personal => "PERSONAL",
            NounClass::Common => "COMMON",
        }
    }

    /// `getIdentifier()`: the lowercase CLDR identifier.
    pub fn get_identifier(self) -> &'static str {
        match self {
            NounClass::Undefined => "undefined",
            NounClass::Other => "other",
            NounClass::Neuter => "neuter",
            NounClass::Feminine => "feminine",
            NounClass::Masculine => "masculine",
            NounClass::Animate => "animate",
            NounClass::Inanimate => "inanimate",
            NounClass::Personal => "personal",
            NounClass::Common => "common",
        }
    }

    /// `fromIdentifier(String identifier)`: the value with this
    /// identifier, `UNDEFINED` for an unknown one or Java's `null`.
    pub fn from_identifier(identifier: Option<&str>) -> NounClass {
        let Some(identifier) = identifier else {
            return NounClass::Undefined;
        };
        for value in Self::VALUES {
            if identifier == value.get_identifier() {
                return value;
            }
        }
        NounClass::Undefined
    }
}

impl fmt::Display for NounClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Represents all the dialect handlings.
///
/// Equivalent to the enum `DisplayOptions.NameStyle` (declaration order kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NameStyle {
    /// `UNDEFINED`.
    Undefined,
    /// `STANDARD_NAMES`.
    StandardNames,
    /// `DIALECT_NAMES`.
    DialectNames,
}

impl NameStyle {
    /// `VALUES`: the unmodifiable list of every value.
    pub const VALUES: [NameStyle; 3] = [
        NameStyle::Undefined,
        NameStyle::StandardNames,
        NameStyle::DialectNames,
    ];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            NameStyle::Undefined => "UNDEFINED",
            NameStyle::StandardNames => "STANDARD_NAMES",
            NameStyle::DialectNames => "DIALECT_NAMES",
        }
    }
}

impl fmt::Display for NameStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Represents all the substitute handling.
///
/// Equivalent to the enum `DisplayOptions.SubstituteHandling` (declaration order kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SubstituteHandling {
    /// `UNDEFINED`.
    Undefined,
    /// `SUBSTITUTE`.
    Substitute,
    /// `NO_SUBSTITUTE`.
    NoSubstitute,
}

impl SubstituteHandling {
    /// `VALUES`: the unmodifiable list of every value.
    pub const VALUES: [SubstituteHandling; 3] = [
        SubstituteHandling::Undefined,
        SubstituteHandling::Substitute,
        SubstituteHandling::NoSubstitute,
    ];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            SubstituteHandling::Undefined => "UNDEFINED",
            SubstituteHandling::Substitute => "SUBSTITUTE",
            SubstituteHandling::NoSubstitute => "NO_SUBSTITUTE",
        }
    }
}

impl fmt::Display for SubstituteHandling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Represents all the display lengths.
///
/// Equivalent to the enum `DisplayOptions.DisplayLength` (declaration order kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DisplayLength {
    /// `UNDEFINED`.
    Undefined,
    /// `LENGTH_FULL`.
    LengthFull,
    /// `LENGTH_SHORT`.
    LengthShort,
}

impl DisplayLength {
    /// `VALUES`: the unmodifiable list of every value.
    pub const VALUES: [DisplayLength; 3] = [
        DisplayLength::Undefined,
        DisplayLength::LengthFull,
        DisplayLength::LengthShort,
    ];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            DisplayLength::Undefined => "UNDEFINED",
            DisplayLength::LengthFull => "LENGTH_FULL",
            DisplayLength::LengthShort => "LENGTH_SHORT",
        }
    }
}

impl fmt::Display for DisplayLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Represents all the capitalization options.
///
/// Equivalent to the enum `DisplayOptions.Capitalization` (declaration order kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capitalization {
    /// `UNDEFINED`.
    Undefined,
    /// `BEGINNING_OF_SENTENCE`.
    BeginningOfSentence,
    /// `MIDDLE_OF_SENTENCE`.
    MiddleOfSentence,
    /// `STANDALONE`.
    Standalone,
    /// `UI_LIST_OR_MENU`.
    UiListOrMenu,
}

impl Capitalization {
    /// `VALUES`: the unmodifiable list of every value.
    pub const VALUES: [Capitalization; 5] = [
        Capitalization::Undefined,
        Capitalization::BeginningOfSentence,
        Capitalization::MiddleOfSentence,
        Capitalization::Standalone,
        Capitalization::UiListOrMenu,
    ];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            Capitalization::Undefined => "UNDEFINED",
            Capitalization::BeginningOfSentence => "BEGINNING_OF_SENTENCE",
            Capitalization::MiddleOfSentence => "MIDDLE_OF_SENTENCE",
            Capitalization::Standalone => "STANDALONE",
            Capitalization::UiListOrMenu => "UI_LIST_OR_MENU",
        }
    }
}

impl fmt::Display for Capitalization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Standard CLDR plural form/category constants.
///
/// Equivalent to the enum `DisplayOptions.PluralCategory` (declaration order kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PluralCategory {
    /// `UNDEFINED`.
    Undefined,
    /// `ZERO`.
    Zero,
    /// `ONE`.
    One,
    /// `TWO`.
    Two,
    /// `FEW`.
    Few,
    /// `MANY`.
    Many,
    /// `OTHER`.
    Other,
}

impl PluralCategory {
    /// `VALUES`: the unmodifiable list of every value.
    pub const VALUES: [PluralCategory; 7] = [
        PluralCategory::Undefined,
        PluralCategory::Zero,
        PluralCategory::One,
        PluralCategory::Two,
        PluralCategory::Few,
        PluralCategory::Many,
        PluralCategory::Other,
    ];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            PluralCategory::Undefined => "UNDEFINED",
            PluralCategory::Zero => "ZERO",
            PluralCategory::One => "ONE",
            PluralCategory::Two => "TWO",
            PluralCategory::Few => "FEW",
            PluralCategory::Many => "MANY",
            PluralCategory::Other => "OTHER",
        }
    }

    /// `getIdentifier()`: the lowercase CLDR identifier.
    pub fn get_identifier(self) -> &'static str {
        match self {
            PluralCategory::Undefined => "undefined",
            PluralCategory::Zero => "zero",
            PluralCategory::One => "one",
            PluralCategory::Two => "two",
            PluralCategory::Few => "few",
            PluralCategory::Many => "many",
            PluralCategory::Other => "other",
        }
    }

    /// `fromIdentifier(String identifier)`: the value with this
    /// identifier, `UNDEFINED` for an unknown one or Java's `null`.
    pub fn from_identifier(identifier: Option<&str>) -> PluralCategory {
        let Some(identifier) = identifier else {
            return PluralCategory::Undefined;
        };
        for value in Self::VALUES {
            if identifier == value.get_identifier() {
                return value;
            }
        }
        PluralCategory::Undefined
    }
}

impl fmt::Display for PluralCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Represents all the grammatical cases that are supported by CLDR.
///
/// Equivalent to the enum `DisplayOptions.GrammaticalCase` (declaration order kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GrammaticalCase {
    /// `UNDEFINED`.
    Undefined,
    /// `ABLATIVE`.
    Ablative,
    /// `ACCUSATIVE`.
    Accusative,
    /// `COMITATIVE`.
    Comitative,
    /// `DATIVE`.
    Dative,
    /// `ERGATIVE`.
    Ergative,
    /// `GENITIVE`.
    Genitive,
    /// `INSTRUMENTAL`.
    Instrumental,
    /// `LOCATIVE`.
    Locative,
    /// `LOCATIVE_COPULATIVE`.
    LocativeCopulative,
    /// `NOMINATIVE`.
    Nominative,
    /// `OBLIQUE`.
    Oblique,
    /// `PREPOSITIONAL`.
    Prepositional,
    /// `SOCIATIVE`.
    Sociative,
    /// `VOCATIVE`.
    Vocative,
}

impl GrammaticalCase {
    /// `VALUES`: the unmodifiable list of every value.
    pub const VALUES: [GrammaticalCase; 15] = [
        GrammaticalCase::Undefined,
        GrammaticalCase::Ablative,
        GrammaticalCase::Accusative,
        GrammaticalCase::Comitative,
        GrammaticalCase::Dative,
        GrammaticalCase::Ergative,
        GrammaticalCase::Genitive,
        GrammaticalCase::Instrumental,
        GrammaticalCase::Locative,
        GrammaticalCase::LocativeCopulative,
        GrammaticalCase::Nominative,
        GrammaticalCase::Oblique,
        GrammaticalCase::Prepositional,
        GrammaticalCase::Sociative,
        GrammaticalCase::Vocative,
    ];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            GrammaticalCase::Undefined => "UNDEFINED",
            GrammaticalCase::Ablative => "ABLATIVE",
            GrammaticalCase::Accusative => "ACCUSATIVE",
            GrammaticalCase::Comitative => "COMITATIVE",
            GrammaticalCase::Dative => "DATIVE",
            GrammaticalCase::Ergative => "ERGATIVE",
            GrammaticalCase::Genitive => "GENITIVE",
            GrammaticalCase::Instrumental => "INSTRUMENTAL",
            GrammaticalCase::Locative => "LOCATIVE",
            GrammaticalCase::LocativeCopulative => "LOCATIVE_COPULATIVE",
            GrammaticalCase::Nominative => "NOMINATIVE",
            GrammaticalCase::Oblique => "OBLIQUE",
            GrammaticalCase::Prepositional => "PREPOSITIONAL",
            GrammaticalCase::Sociative => "SOCIATIVE",
            GrammaticalCase::Vocative => "VOCATIVE",
        }
    }

    /// `getIdentifier()`: the lowercase CLDR identifier.
    pub fn get_identifier(self) -> &'static str {
        match self {
            GrammaticalCase::Undefined => "undefined",
            GrammaticalCase::Ablative => "ablative",
            GrammaticalCase::Accusative => "accusative",
            GrammaticalCase::Comitative => "comitative",
            GrammaticalCase::Dative => "dative",
            GrammaticalCase::Ergative => "ergative",
            GrammaticalCase::Genitive => "genitive",
            GrammaticalCase::Instrumental => "instrumental",
            GrammaticalCase::Locative => "locative",
            GrammaticalCase::LocativeCopulative => "locative_copulative",
            GrammaticalCase::Nominative => "nominative",
            GrammaticalCase::Oblique => "oblique",
            GrammaticalCase::Prepositional => "prepositional",
            GrammaticalCase::Sociative => "sociative",
            GrammaticalCase::Vocative => "vocative",
        }
    }

    /// `fromIdentifier(String identifier)`: the value with this
    /// identifier, `UNDEFINED` for an unknown one or Java's `null`.
    pub fn from_identifier(identifier: Option<&str>) -> GrammaticalCase {
        let Some(identifier) = identifier else {
            return GrammaticalCase::Undefined;
        };
        for value in Self::VALUES {
            if identifier == value.get_identifier() {
                return value;
            }
        }
        GrammaticalCase::Undefined
    }
}

impl fmt::Display for GrammaticalCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
