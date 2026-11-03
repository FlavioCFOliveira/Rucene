//! Port of `com.carrotsearch.randomizedtesting.Rants`
//! (randomizedtesting 2.8.1, `Rants.java`): the documented complaints about
//! JUnit that explain some of the runner's choices.

/// `Rants.RantType`.
#[allow(dead_code)] // Java keeps these constants as documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RantType {
    /// `ANNOYANCE`.
    Annoyance,
    /// `DAMN_TERRIBLE`.
    DamnTerrible,
    /// `WTF`.
    Wtf,
    /// `ISHOULDHAVEBECOMEALAWYER`.
    IShouldHaveBecomeALawyer,
}

/// `Rants`.
#[allow(dead_code)] // Java keeps these constants as documentation.
pub(crate) struct Rants;

#[allow(dead_code)] // Java keeps these constants as documentation.
impl Rants {
    /// `RANT_1`: a `Description` cannot carry the test class and name apart
    /// from the display name, so seeds in display names confuse IDEs.
    pub(crate) const RANT_1: RantType = RantType::DamnTerrible;
    /// `RANT_2`: assumption methods cannot carry a custom message.
    pub(crate) const RANT_2: RantType = RantType::Annoyance;
    /// `RANT_3`: a failed assumption is propagated as a `Failure`.
    pub(crate) const RANT_3: RantType = RantType::DamnTerrible;
    /// `RANT_4`: JUnit treats method annotations inconsistently across
    /// overrides.
    pub(crate) const RANT_4: RantType = RantType::DamnTerrible;
}
