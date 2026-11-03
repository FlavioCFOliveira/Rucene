//! `org.junit.runner.JUnitCore`, `org.junit.runner.Request`, `Runner` and
//! `org.junit.runners.model.InitializationError`, rebuilt from their
//! documented behaviour.

use std::sync::Arc;

use super::description::Description;
use super::notification::{Failure, JUnitResult, ListenerRef, Notifier, RunNotifier};
use super::statement::Filter;
use crate::jdk::exception::Throwable;
use crate::jdk::reflect::Class;
use crate::randomized_runner::RandomizedRunner;

/// `org.junit.runner.Runner`.
pub trait Runner: std::any::Any + Send + Sync {
    /// `getDescription()`.
    fn description(&self) -> Arc<Description>;
    /// `run(notifier)`.
    fn run(&self, notifier: &Arc<RunNotifier>);
}

/// `org.junit.runners.model.InitializationError`: the causes that prevent a
/// runner from being built.
#[derive(Debug, Clone)]
pub struct InitializationError {
    causes: Vec<Throwable>,
}

impl InitializationError {
    /// `new InitializationError(Throwable error)`.
    pub fn new(error: Throwable) -> InitializationError {
        InitializationError {
            causes: vec![error],
        }
    }

    /// `getCauses()`.
    pub fn causes(&self) -> &[Throwable] {
        &self.causes
    }
}

/// The runner JUnit substitutes for a class whose runner cannot be built
/// (`ErrorReportingRunner`): one `initializationError` test per cause, each
/// started, failed and finished.
struct ErrorReportingRunner {
    class_name: String,
    causes: Vec<Throwable>,
}

impl ErrorReportingRunner {
    fn describe_cause(&self) -> Arc<Description> {
        Description::create_test_description_named(&self.class_name, "initializationError")
    }
}

impl Runner for ErrorReportingRunner {
    fn description(&self) -> Arc<Description> {
        let d = Description::create_suite_description(&self.class_name, Vec::new());
        for _ in &self.causes {
            d.add_child(self.describe_cause());
        }
        d
    }

    fn run(&self, notifier: &Arc<RunNotifier>) {
        for cause in &self.causes {
            let d = self.describe_cause();
            notifier.fire_test_started(&d);
            notifier.fire_test_failure(Failure::new(Arc::clone(&d), cause.clone()));
            notifier.fire_test_finished(&d);
        }
    }
}

/// A filtered runner (`Request.filterWith`): `RandomizedRunner.filter`, or
/// the "no tests remain" error runner JUnit substitutes.
struct FilteredRequest {
    runner: Arc<dyn Runner>,
}

impl Runner for FilteredRequest {
    fn description(&self) -> Arc<Description> {
        self.runner.description()
    }
    fn run(&self, notifier: &Arc<RunNotifier>) {
        self.runner.run(notifier)
    }
}

/// `org.junit.runner.Request`: what to run.
#[derive(Clone)]
pub struct Request {
    runner: Arc<dyn Runner>,
}

impl Request {
    /// `Request.aClass(clazz)`: the class under `RandomizedRunner`.
    ///
    /// **Divergence.** JUnit picks the runner from `@RunWith` (and runs a
    /// plain class with `BlockJUnit4ClassRunner`); every class runs under
    /// [`RandomizedRunner`] here, the only runner the port provides.
    pub fn a_class(class: &Arc<Class>) -> Request {
        match RandomizedRunner::new(class) {
            Ok(r) => Request {
                runner: Arc::new(r),
            },
            Err(e) => Request {
                runner: Arc::new(ErrorReportingRunner {
                    class_name: class.name().to_string(),
                    causes: e.causes().to_vec(),
                }),
            },
        }
    }

    /// `Request.method(clazz, methodName)`: the class, filtered to the test
    /// described as `methodName(className)`.
    pub fn method(class: &Arc<Class>, method_name: &str) -> Request {
        let desired = Description::create_test_description(class, method_name, Vec::new());
        Request::a_class(class).filter_with(Arc::new(MatchMethod(desired)))
    }

    /// `filterWith(filter)`.
    pub fn filter_with(self, filter: Arc<dyn Filter>) -> Request {
        let runner = Arc::clone(&self.runner);
        if let Some(rr) = downcast_randomized(&runner) {
            if let Err(e) = rr.filter(filter.as_ref()) {
                let class_name = rr.description().display_name().to_string();
                return Request {
                    runner: Arc::new(ErrorReportingRunner {
                        class_name,
                        causes: vec![e],
                    }),
                };
            }
        }
        Request {
            runner: Arc::new(FilteredRequest { runner }),
        }
    }

    /// `getRunner()`.
    pub fn runner(&self) -> &Arc<dyn Runner> {
        &self.runner
    }
}

fn downcast_randomized(r: &Arc<dyn Runner>) -> Option<&RandomizedRunner> {
    crate::randomized_runner::as_randomized_runner(r.as_ref())
}

/// `Filter.matchMethodDescription(desired)`.
struct MatchMethod(Arc<Description>);

impl Filter for MatchMethod {
    fn should_run(&self, description: &Description) -> bool {
        if description.is_test() {
            return *self.0 == *description;
        }
        description.children().iter().any(|c| self.should_run(c))
    }

    fn describe(&self) -> String {
        format!("Method {}", self.0.display_name())
    }
}

/// `org.junit.runner.JUnitCore`: runs requests and reports to its
/// listeners.
#[derive(Default)]
pub struct JUnitCore {
    listeners: Vec<ListenerRef>,
}

impl JUnitCore {
    /// `new JUnitCore()`.
    pub fn new() -> JUnitCore {
        JUnitCore::default()
    }

    /// `addListener(listener)`.
    pub fn add_listener(&mut self, listener: ListenerRef) {
        self.listeners.push(listener);
    }

    /// `run(Request)`: fires `testRunStarted` with the runner's
    /// description, runs it and fires `testRunFinished`.
    pub fn run(&self, request: &Request) -> Arc<JUnitResult> {
        let _quiet = crate::jdk::exception::QuietPanics::new();
        let result = JUnitResult::new();
        let notifier = Arc::new(RunNotifier::new());
        let listener = result.create_listener();
        notifier.add_first_listener(Arc::clone(&listener));
        for l in &self.listeners {
            notifier.add_listener(Arc::clone(l));
        }
        let runner = request.runner();
        notifier.fire_test_run_started(&runner.description());
        runner.run(&notifier);
        notifier.fire_test_run_finished(&result);
        notifier.remove_listener(&listener);
        result
    }

    /// `run(Class<?>... classes)` for one class.
    pub fn run_class(&self, class: &Arc<Class>) -> Arc<JUnitResult> {
        self.run(&Request::a_class(class))
    }

    /// `JUnitCore.runClasses(classes)`: each class in turn, into one
    /// result.
    pub fn run_classes(classes: &[Arc<Class>]) -> Arc<JUnitResult> {
        let core = JUnitCore::new();
        let result = JUnitResult::new();
        let notifier = Arc::new(RunNotifier::new());
        let listener = result.create_listener();
        notifier.add_first_listener(Arc::clone(&listener));
        let suite = Description::create_suite_description("null", Vec::new());
        let requests: Vec<Request> = classes.iter().map(Request::a_class).collect();
        for r in &requests {
            suite.add_child(r.runner().description());
        }
        notifier.fire_test_run_started(&suite);
        for r in &requests {
            r.runner().run(&notifier);
        }
        notifier.fire_test_run_finished(&result);
        let _ = core;
        result
    }
}
