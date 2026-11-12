//! The disputes no reader was shown, posted on the pull request as one comment
//!
//! A fix turn that shows a finding wrong leaves its reason beside the findings file, and the
//! pass shows that reason to the next reader. When none follows, the reasons go on the pull
//! request as one comment at the merge yes, as SK's `post_disputes` does
//! (`docs/design/review.md`, What the maintainer sees): nothing else a pass finds goes to
//! GitHub, no findings, no review, no ready mark.
//!
//! The comment is the engine's ([`review::comment`]), which names a file of the ticket's build
//! folder by its name alone. Kelpie's own check for what must not leave this machine
//! ([`LocalPaths`]) reads it before it is asked for: a reason naming a path of another folder
//! is a ruling, since asking again would be refused the same. The ruling tells the worker to
//! write its reason again in the disputed findings file, and [`review::refresh`] reads that
//! file back, so the next yes posts what the worker wrote and not the reason that was refused.
//! A post the forge refuses for any other reason leaves the disputes unshown and the head off
//! the yes, and the next read of GitHub tries again, which it does because the gate gives the
//! yes again on every read.
//!
//! The comment goes through [`Tickets::comment`] with the pull request's number, which GitHub
//! reads as an issue's own: a pull request is an issue there, so it lands on the pull request
//! and not on the ticket. The disputes are marked shown once posted, so a later pass posts none
//! twice.

use std::path::Path;

use kelpie_engine::local_paths::{LocalPaths, Surface};
use kelpie_engine::review::{self, Dispute};
use kelpie_engine::ticket::TicketId;

use super::{At, Review, lock};

/// How posting a pass's disputes came to
pub(crate) enum Posted {
    /// They are on the pull request, or there were none to post
    Told,
    /// A dispute names a path of this machine that is not the build folder's, so nothing is
    /// posted and the ticket is ruled
    Refused(review::Ruling),
    /// The forge would not take them just now: they stay unshown, and the next read tries again
    Retry,
}

impl Review {
    /// Post the disputes of `at`'s pass that no reader was shown, as one comment on its pull
    /// request
    ///
    /// The ruling is [`review::Ruling::DisputesLocal`], which is what the gate records when a
    /// reason names this machine: the disputes stay unshown, and the ticket waits on the
    /// maintainer rather than going round the same refused post.
    pub(crate) fn posted(&self, at: &At<'_>) -> Posted {
        let Ok(id) = at.ticket.parse::<TicketId>() else {
            return Posted::Told;
        };
        let build = self.paths.build_dir(at.ticket);
        let Some(disputes) = self.refreshed(id, at.ticket, &build) else {
            return Posted::Told;
        };
        let Some(body) = review::comment(&disputes, &build) else {
            return Posted::Told;
        };
        // The whole data folder, as `GithubPulls` checks its posts: a reason naming another
        // project's folder names this machine as much as one naming this project's checkout
        let local = LocalPaths::new([self.folder.as_path(), self.paths.data_dir.as_path()]);
        if let Some(leak) = local.find(&body, Surface::Prose) {
            let count = disputes.iter().filter(|d| !d.shown).count();
            eprintln!(
                "kelpie: ticket {}'s disputes were not posted: {leak}",
                at.ticket
            );
            return Posted::Refused(review::Ruling::DisputesLocal {
                count,
                what: leak.to_string(),
            });
        }
        // A pull request is an issue on GitHub, so its number is what its comments are kept under
        let Some(pull) = TicketId::from_issue(at.number) else {
            eprintln!(
                "kelpie: ticket {}'s disputes were not posted: pull request {} is no ticket",
                at.ticket, at.number
            );
            return Posted::Retry;
        };
        match self.tickets.comment(&pull, &body) {
            Ok(()) => {
                self.shown(at.ticket, id);
                Posted::Told
            }
            Err(why) => {
                eprintln!(
                    "kelpie: ticket {}'s disputes were not posted on pull request {}: {why}",
                    at.ticket, at.number
                );
                Posted::Retry
            }
        }
    }

    /// The disputes of the pass of ticket `ticket` no reader was shown, each with the reason
    /// its file holds now, `None` when the pass has none to post
    ///
    /// The worker's file is read back before the comment is built ([`review::refresh`]), so a
    /// reason rewritten after a ruling is what goes on the pull request. A file that cannot be
    /// read refreshes nothing, and what the pass kept stands.
    fn refreshed(&self, id: TicketId, ticket: &str, build: &Path) -> Option<Vec<Dispute>> {
        let mut book = lock(&self.book);
        book.get(id)?;
        let from_file = match review::disputed(build) {
            Ok(read) => read,
            Err(why) => {
                eprintln!("kelpie: ticket {ticket}'s disputed findings did not read: {why}");
                Vec::new()
            }
        };
        let kept = book.entry(id);
        let changed = review::refresh(&mut kept.disputes, &from_file);
        let disputes = kept.disputes.clone();
        if changed && let Err(why) = book.save() {
            eprintln!(
                "kelpie: the review's rewritten disputes of ticket {ticket} did not keep: {why}"
            );
        }
        (disputes.iter().any(|d| !d.shown)).then_some(disputes)
    }

    /// Mark every dispute of ticket `ticket` shown, now that they are on the pull request
    ///
    /// They are kept under the ticket's own id, whatever id their comment was posted under.
    fn shown(&self, ticket: &str, id: TicketId) {
        let mut book = lock(&self.book);
        review::mark_shown(&mut book.entry(id).disputes);
        if let Err(why) = book.save() {
            eprintln!(
                "kelpie: the review's posted disputes of ticket {ticket} did not keep: {why}"
            );
        }
    }
}
