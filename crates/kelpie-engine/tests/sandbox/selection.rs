//! Which tests an invocation names, read as `libtest` reads its arguments
//!
//! A child of `tests/sandbox.rs`, which is its own `main` and so parses the names it
//! is given to run.

/// Which tests an invocation names, read as `libtest` reads its arguments
///
/// A bare word is a name filter, `--skip` names tests to leave out and `--exact`
/// makes a filter match a whole name. The other options `libtest` takes are
/// ignored, and those that take a value (`--test-threads 1`) keep it from
/// being read as a filter.
pub(super) struct Selection {
    filters: Vec<String>,
    skips: Vec<String>,
    exact: bool,
}

/// The options `libtest` takes a value for, as `--name value`
const VALUED: [&str; 7] = [
    "--test-threads",
    "--skip",
    "--format",
    "--color",
    "--logfile",
    "--report-time",
    "--shuffle-seed",
];

impl Selection {
    pub(super) fn of(args: &[String]) -> Self {
        let mut selection = Self {
            filters: Vec::new(),
            skips: Vec::new(),
            exact: false,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let (option, value) = match arg.split_once('=') {
                Some((option, value)) if arg.starts_with("--") => (option, Some(value.to_owned())),
                _ => (arg.as_str(), None),
            };
            if option == "--exact" {
                selection.exact = true;
            } else if VALUED.contains(&option) {
                let value = value.or_else(|| args.next().cloned());
                if let (true, Some(value)) = (option == "--skip", value) {
                    selection.skips.push(value);
                }
            } else if !arg.starts_with('-') {
                selection.filters.push(arg.clone());
            }
        }
        selection
    }

    fn matches(&self, name: &str, word: &str) -> bool {
        if self.exact {
            name == word
        } else {
            name.contains(word)
        }
    }

    pub(super) fn runs(&self, name: &str) -> bool {
        (self.filters.is_empty() || self.filters.iter().any(|word| self.matches(name, word)))
            && !self.skips.iter().any(|word| self.matches(name, word))
    }
}

pub(super) fn the_value_of_a_libtest_option_is_not_a_filter() {
    let words =
        |args: &[&str]| -> Vec<String> { args.iter().map(|arg| (*arg).to_owned()).collect() };
    let name = "the_sweep_finds_a_sandboxed_process";

    let threads = Selection::of(&words(&["--test-threads", "1", "--nocapture"]));
    assert!(threads.runs(name), "a thread count filtered the tests out");

    let skip = Selection::of(&words(&["--skip", "sweep"]));
    assert!(!skip.runs(name), "--skip sweep ran the sweep");
    assert!(skip.runs("a_write_outside_the_state_dir_is_refused"));

    let skip_equals = Selection::of(&words(&["--skip=sweep", "--format=terse"]));
    assert!(!skip_equals.runs(name));

    let filter = Selection::of(&words(&["--test-threads", "1", "refused"]));
    assert!(filter.runs("a_write_outside_the_state_dir_is_refused"));
    assert!(!filter.runs(name));

    let exact = Selection::of(&words(&["--exact", "refused"]));
    assert!(!exact.runs("a_write_outside_the_state_dir_is_refused"));
}
