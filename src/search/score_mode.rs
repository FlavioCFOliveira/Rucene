//! Search modes, ported from `org.apache.lucene.search.ScoreMode`.

#![deny(unsafe_code)]

/// Different modes of search.
///
/// Equivalent to `org.apache.lucene.search.ScoreMode`. The two flags carried by
/// every variant drive early termination: [`ScoreMode::needs_scores`] tells a
/// [`crate::search::Weight`] whether scores have to be computed at all, and
/// [`ScoreMode::is_exhaustive`] tells it whether every match has to be visited
/// or whether it may skip non-competitive hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum ScoreMode {
    /// Produced scorers will allow visiting all matches and get their score.
    COMPLETE,

    /// Produced scorers will allow visiting all matches but scores won't be
    /// available.
    COMPLETE_NO_SCORES,

    /// Produced scorers will optionally allow skipping over non-competitive
    /// hits using the [`crate::search::Scorable::set_min_competitive_score`]
    /// API.
    TOP_SCORES,

    /// Score mode for top field collectors that can provide their own
    /// iterators, to optionally allow skipping non-competitive docs.
    TOP_DOCS,

    /// Score mode for top field collectors that can provide their own
    /// iterators, to optionally allow skipping non-competitive docs. This mode
    /// is used when there is a secondary sort by `_score`.
    TOP_DOCS_WITH_SCORES,
}

impl ScoreMode {
    /// Whether this [`ScoreMode`] needs to compute scores.
    ///
    /// Equivalent to `ScoreMode.needsScores()`.
    pub fn needs_scores(self) -> bool {
        match self {
            Self::COMPLETE => true,
            Self::COMPLETE_NO_SCORES => false,
            Self::TOP_SCORES => true,
            Self::TOP_DOCS => false,
            Self::TOP_DOCS_WITH_SCORES => true,
        }
    }

    /// Returns `true` if for this [`ScoreMode`] it is necessary to process all
    /// documents, or `false` if it is enough to go through top documents only.
    ///
    /// Equivalent to `ScoreMode.isExhaustive()`.
    pub fn is_exhaustive(self) -> bool {
        match self {
            Self::COMPLETE => true,
            Self::COMPLETE_NO_SCORES => true,
            Self::TOP_SCORES => false,
            Self::TOP_DOCS => false,
            Self::TOP_DOCS_WITH_SCORES => false,
        }
    }
}
