//! Each project's gate, built once from what [`Workers`] holds
//!
//! The gate ([`crate::gate::Gate`]) reads the project's pull requests and CI through the
//! project's [`Forge`], sends a worker back through [`Workers::fix_turn`], and keeps its
//! facts in `gate.json` beside the project's records. One gate per project is cached
//! here, so every board read of a project runs the same one and its one-pass-per-ticket
//! rule holds across reads.
//!
//! A green head goes to the project's review pass before the merge yes, so the gate is
//! built beside one [`Review`] of the same project ([`crate::review`]), which holds
//! `review.json` beside `gate.json`. Both are cached with the gate.
//!
//! The gate is cached in [`Workers`], so it holds [`Workers`] only weakly: a strong
//! hold would be a ring nothing drops.

use std::path::Path;
use std::sync::{Arc, Weak};

use kelpie_engine::github::Clock;
use kelpie_engine::session::Session;
use kelpie_engine::settings::Settings;
use kelpie_engine::ticket::{GithubTickets, Tickets};
use kelpie_engine::tools::Tools;
use kelpie_engine::worker::{Opening, Paths};

use super::{OpenOn, Workers, lock};
use crate::gate::{Choose, Fix, Gate, Parts, Stop};
use crate::review::{self, Make, Review, ReviewOpen};

/// Builds the session of a worker that has none live, as a fix turn needs
///
/// The app's is `super::open::open`, and a test's is the one of its own agent.
pub type Open =
    dyn for<'a, 'b> Fn(&OpenOn<'a>, &Opening<'b>) -> Result<Session, String> + Send + Sync;

/// What the app starts a fix turn's session with: `super::open::open`
#[must_use]
pub fn app_open() -> Arc<Open> {
    Arc::new(super::open::open)
}

impl Workers {
    /// The gate of the project `project_id` if one has been built, `None` before its board is watched
    #[must_use]
    pub fn gate_of(&self, project_id: &str) -> Option<Arc<Gate>> {
        lock(&self.gates).get(project_id).cloned()
    }

    /// The gate of the project `project_id`, whose checkout is `folder`, built on the first ask
    ///
    /// `tools` and `open` are what a fix turn starts a worker's session with, `clock`
    /// is where a pass reads the present moment from, and `reviewers` is what builds a
    /// reviewer for a read, `None` for kelpie's own, which starts a session of the
    /// reviewer the pass chose.
    ///
    /// # Errors
    ///
    /// The project has no forge to read (see `Workers::forge`), its records cannot be
    /// read, `gate.json` cannot, or `review.json` cannot.
    pub fn gate(
        self: &Arc<Self>,
        project_id: &str,
        folder: &Path,
        tools: Option<Arc<Tools>>,
        clock: Arc<dyn Clock>,
        open: Arc<Open>,
        reviewers: Option<Arc<Make>>,
    ) -> Result<Arc<Gate>, String> {
        if let Some(gate) = lock(&self.gates).get(project_id) {
            return Ok(Arc::clone(gate));
        }
        let forge = self.forge(project_id, folder)?;
        let records = self.records(project_id)?;
        let paths = Paths::new(&self.data_dir, project_id);
        let limits: Choose = {
            let (config, project) = (self.config.clone(), project_id.to_owned());
            Box::new(move || {
                let settings =
                    Settings::load(&config, &project).map_err(|error| error.to_string())?;
                Ok((&settings).into())
            })
        };
        // The gate and its review pass send a worker back the same way, each holding its own
        // copy of the hand
        let sent = || -> Fix {
            let workers = Arc::downgrade(self);
            let (project, folder) = (project_id.to_owned(), folder.to_owned());
            let (tickets, tools, open) = (
                Arc::clone(forge.tickets()),
                tools.clone(),
                Arc::clone(&open),
            );
            Box::new(move |ticket, prompt| {
                send_back(
                    &workers,
                    &project,
                    &folder,
                    tools.as_deref(),
                    &tickets,
                    &open,
                    ticket,
                    prompt,
                )
            })
        };
        let stop: Stop = {
            let (workers, project) = (Arc::downgrade(self), project_id.to_owned());
            Box::new(move |ticket| {
                if let Some(workers) = workers.upgrade() {
                    workers.forget_session(&project, ticket);
                }
            })
        };
        let make = reviewers.unwrap_or_else(|| {
            review::app_make(
                &ReviewOpen::new(&self.data_dir, &self.logins),
                &paths,
                folder,
                &self.tree,
            )
        });
        let review = Arc::new(Review::new(review::Parts {
            project_id: project_id.to_owned(),
            folder: folder.to_owned(),
            paths: paths.clone(),
            tree: Arc::clone(&self.tree),
            records: Arc::downgrade(&records),
            tickets: Arc::clone(forge.tickets()) as Arc<dyn Tickets>,
            config: self.config.clone(),
            clock: Arc::clone(&clock),
            fix: sent(),
            make,
        })?);
        let gate = Arc::new(Gate::new(Parts {
            project_id: project_id.to_owned(),
            folder: folder.to_owned(),
            paths,
            tree: Arc::clone(&self.tree),
            records: Arc::downgrade(&records),
            pulls: Arc::clone(forge.pulls()),
            tickets: Arc::clone(forge.tickets()) as Arc<dyn Tickets>,
            ci: Arc::clone(forge.ci()),
            queue: Arc::clone(forge.queue()),
            clock,
            limits,
            fix: sent(),
            stop,
            review,
        })?);
        // Two reads can race to build one: the first to insert is the project's
        let mut kept = lock(&self.gates);
        Ok(Arc::clone(
            kept.entry(project_id.to_owned()).or_insert(gate),
        ))
    }
}

/// Send the worker on `ticket` back with `prompt`, through the workers `workers` still holds
#[expect(
    clippy::too_many_arguments,
    reason = "a fix turn names the project, where it is, what starts a session, where a ticket is read, and what to say"
)]
fn send_back(
    workers: &Weak<Workers>,
    project: &str,
    folder: &Path,
    tools: Option<&Tools>,
    tickets: &Arc<dyn GithubTickets>,
    open: &Arc<Open>,
    ticket: &str,
    prompt: &str,
) -> Result<(), String> {
    let workers = workers
        .upgrade()
        .ok_or_else(|| "kelpie is shutting down".to_owned())?;
    let source: &dyn Tickets = &**tickets;
    workers
        .fix_turn(
            project,
            folder,
            tools,
            ticket,
            prompt,
            Ok(source),
            &|on, opening| open(on, opening),
        )
        .map(|_| ())
}
