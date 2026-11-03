//! Port of `com.carrotsearch.randomizedtesting.GroupEvaluator`
//! (randomizedtesting 2.8.1, `GroupEvaluator.java`): the enabled state of
//! test groups and the `tests.filter` expression.

use std::sync::Mutex;

use crate::annotations::{AnnotationType, TestGroupUtilities};
use crate::filter_expression_parser::{FilterExpressionParser, IContext, Node};
use crate::jdk::lang::string_hash_code;
use crate::jdk::reflect::AnnotationRef;
use crate::jdk::system;
use crate::jdk::util::hash_iteration_order;
use crate::randomized_test::RandomizedTest;
use crate::reproduce_error_message_builder::ReproduceErrorMessageBuilder;
use crate::sys_globals::SysGlobals;

/// The state of one test group (`GroupEvaluator.TestGroupInfo`).
#[derive(Debug, Clone)]
struct TestGroupInfo {
    name: String,
    sys_property: String,
    enabled: bool,
}

impl TestGroupInfo {
    fn new(ann_type: &AnnotationType) -> TestGroupInfo {
        let group = ann_type.test_group.expect("a test group annotation");
        let name = TestGroupUtilities::get_group_name(ann_type);
        let sys_property = TestGroupUtilities::get_sys_property(ann_type);
        // A malformed property disables the group (Java catches the
        // IllegalArgumentException of systemPropertyAsBoolean).
        let enabled = RandomizedTest::try_system_property_as_boolean(&sys_property, group.enabled)
            .unwrap_or(false);
        TestGroupInfo {
            name,
            sys_property,
            enabled,
        }
    }
}

/// Evaluates the enabled or disabled state of test groups and the filter
/// expression: `GroupEvaluator`.
///
/// **Divergence.** Java keys the groups by annotation `Class` in a
/// `HashMap` (`GroupEvaluator.java:42`), whose iteration order (identity hashes) is not reproducible;
/// `appendGroupFilteringOptions` lists the groups' properties in that order.
/// The port keeps the groups in first-seen order.
#[derive(Debug, Default)]
pub struct GroupEvaluator {
    test_groups: Mutex<Vec<(&'static str, TestGroupInfo)>>,
    filter: Option<Node>,
    filter_expression: Option<String>,
}

impl GroupEvaluator {
    /// An evaluator with no collected groups and no filter expression: the
    /// state the constructor reaches for a suite without group
    /// annotations and without `tests.filter`.
    pub(crate) fn new() -> GroupEvaluator {
        GroupEvaluator::default()
    }

    /// `GroupEvaluator(List<TestCandidate>)`: collects the groups declared
    /// on the candidates' methods and test classes, and parses
    /// `tests.filter`.
    ///
    /// `candidate_annotations` holds, per candidate, the method's and the
    /// test class's annotations.
    ///
    /// # Panics
    ///
    /// `FilterExpressionParser$SyntaxException` for a malformed filter.
    pub(crate) fn from_candidates(candidate_annotations: &[Vec<AnnotationRef>]) -> GroupEvaluator {
        let mut groups: Vec<(&'static str, TestGroupInfo)> = Vec::new();
        for anns in candidate_annotations {
            for ann in anns {
                let ty = ann.annotation_type();
                if ty.test_group.is_some() && !groups.iter().any(|(n, _)| *n == ty.name) {
                    groups.push((ty.name, TestGroupInfo::new(ty)));
                }
            }
        }
        let filter_expression = system::get_property(&SysGlobals::sysprop_testfilter())
            .filter(|f| !crate::jdk::lang::trim(f).is_empty());
        let filter = filter_expression
            .as_deref()
            .map(|f| FilterExpressionParser::new().parse(f));
        GroupEvaluator {
            test_groups: Mutex::new(groups),
            filter,
            filter_expression,
        }
    }

    fn info(&self, ty: &AnnotationType) -> TestGroupInfo {
        let mut groups = self.test_groups.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, i)) = groups.iter().find(|(n, _)| *n == ty.name) {
            return i.clone();
        }
        let info = TestGroupInfo::new(ty);
        groups.push((ty.name, info.clone()));
        info
    }

    /// `appendGroupFilteringOptions(builder)`: the properties of the groups
    /// that are set, and the filter expression.
    pub(crate) fn append_group_filtering_options(
        &self,
        builder: &mut ReproduceErrorMessageBuilder,
    ) {
        let groups = self
            .test_groups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for (_, info) in groups {
            if let Some(v) = system::get_property(&info.sys_property) {
                builder.append_opt(&info.sys_property, &v);
            }
        }
        if let Some(f) = &self.filter_expression {
            builder.append_opt(&SysGlobals::sysprop_testfilter(), f);
        }
    }

    /// `hasFilteringExpression()`.
    pub(crate) fn has_filtering_expression(&self) -> bool {
        self.filter_expression.is_some()
    }

    /// `getIgnoreReason(AnnotatedElement... elements)`: why a test with the
    /// given annotations (method and class) is ignored, or `None`.
    pub fn get_ignore_reason(&self, element_annotations: &[&[AnnotationRef]]) -> Option<String> {
        // `HashMap<String, Annotation>` keyed by group name.
        let mut annotations: Vec<(String, AnnotationRef)> = Vec::new();
        for anns in element_annotations {
            for ann in anns.iter() {
                let ty = ann.annotation_type();
                if ty.test_group.is_some() {
                    let name = self.info(ty).name;
                    match annotations.iter_mut().find(|(n, _)| *n == name) {
                        Some(slot) => slot.1 = std::sync::Arc::clone(ann),
                        None => annotations.push((name, std::sync::Arc::clone(ann))),
                    }
                }
            }
        }
        let keyed: Vec<(&String, i32)> = annotations
            .iter()
            .map(|(n, _)| (n, string_hash_code(n)))
            .collect();
        let order = hash_iteration_order(&keyed, |a, b| a == b);
        let values: Vec<&AnnotationRef> = order.iter().map(|&i| &annotations[i].1).collect();

        let mut default_state: Option<String> = None;
        for ann in &values {
            let g = self.info(ann.annotation_type());
            if !g.enabled {
                default_state = Some(format!(
                    "'{}' test group is disabled ({})",
                    g.name,
                    annotation_to_string(ann)
                ));
                break;
            }
        }

        match &self.filter {
            Some(filter) => {
                struct Ctx<'a> {
                    default: bool,
                    evaluator: &'a GroupEvaluator,
                    values: &'a [&'a AnnotationRef],
                }
                impl IContext for Ctx<'_> {
                    fn default_value(&self) -> bool {
                        self.default
                    }
                    fn has_group(&self, value: &str) -> bool {
                        let value = value.strip_prefix('@').unwrap_or(value);
                        self.values.iter().any(|a| {
                            equals_ignore_case(
                                value,
                                &self.evaluator.info(a.annotation_type()).name,
                            )
                        })
                    }
                }
                let ctx = Ctx {
                    default: default_state.is_none(),
                    evaluator: self,
                    values: &values,
                };
                if filter.evaluate(&ctx) {
                    None
                } else {
                    Some(format!(
                        "Test filter condition: {}",
                        self.filter_expression.as_deref().unwrap_or("")
                    ))
                }
            }
            None => default_state,
        }
    }

    /// `isGroupEnabled(Class<? extends Annotation>)`: the state of a
    /// `@TestGroup` annotation, read once from its system property. A
    /// filter expression may still enable or disable a test of an enabled
    /// group.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when the annotation is not marked with
    /// `@TestGroup`.
    pub fn is_group_enabled(&self, test_group_annotation: &AnnotationType) -> bool {
        if test_group_annotation.test_group.is_none() {
            let known = self
                .test_groups
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .any(|(n, _)| *n == test_group_annotation.name);
            if !known {
                crate::jdk::exception::illegal_argument(format!(
                    "This annotation is not marked with \
                     @com.carrotsearch.randomizedtesting.annotations.TestGroup: {}",
                    test_group_annotation.name
                ));
            }
        }
        self.info(test_group_annotation).enabled
    }
}

/// `toString(Annotation)`: the annotation's `toString()` with its type's
/// name shortened to the simple name.
fn annotation_to_string(ann: &AnnotationRef) -> String {
    let ty = ann.annotation_type();
    ann.to_java_string().replace(ty.name, ty.simple_name)
}

/// `String.equalsIgnoreCase`: equal lengths and every UTF-16 unit pair
/// equal after upper- or lower-casing.
fn equals_ignore_case(a: &str, b: &str) -> bool {
    let a: Vec<u16> = a.encode_utf16().collect();
    let b: Vec<u16> = b.encode_utf16().collect();
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(&x, &y)| {
        if x == y {
            return true;
        }
        let (Some(cx), Some(cy)) = (char::from_u32(u32::from(x)), char::from_u32(u32::from(y)))
        else {
            return false;
        };
        let ux = single_upper(cx);
        let uy = single_upper(cy);
        ux == uy || single_lower(ux) == single_lower(uy)
    })
}

fn single_upper(c: char) -> char {
    let mut u = c.to_uppercase();
    match (u.next(), u.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

fn single_lower(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}
