//! The board's commands: what the Board panel calls
//!
//! [`board_watch`] starts the project's poller and streams a [`BoardEvent`] to
//! the window whenever the board changes. The [`Poller`](poll::Poller) reads
//! GitHub once a minute and places the tickets again every two seconds from the
//! workers' records, so a Start moves its ticket to Building at once
//! (`docs/design/tickets.md`, The board and GitHub). [`board_refresh`] reads at
//! once, and [`board_move`] runs what a drop on a column does
//! ([`kelpie_engine::board::plan_move`]). [`open_link::open_link`] sends a
//! ticket's page to the OS's own broker, so the browser never descends from
//! kelpie (ADR 0001).
//!
//! The window's types are here, mapped from the engine's [`kelpie_engine::board`]
//! ones, so the engine gains no specta dependency. Kelpie polls while it runs
//! and nothing polls once it quits: [`BoardHost::stop`] ends the poller.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use kelpie_engine::board::{self as engine, Board, Move, plan_move};
use kelpie_engine::github::gh::GithubSource;
use kelpie_engine::github::{Clock, ForgeSlug, SystemClock, Timestamp};
use kelpie_engine::settings::Settings;
use kelpie_engine::ticket::ForgeError;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;
use tauri::ipc::Channel;

use crate::state::AppState;
use crate::workers::{self, WorkerStarted};
use poll::{Cadence, Choices, Command, Feed, Poller};

pub mod link;
pub mod poll;

#[cfg(test)]
mod tests;

/// One of the board's five columns, as the window names it
// wire format: the Board panel names a column by these lower-case words
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    /// Its issue is open without the ready label
    Draft,
    /// Its issue carries the ready label
    Ready,
    /// A worker on it is live
    Building,
    /// A pull request closing it is open
    Review,
    /// Its issue is closed
    Done,
}

impl From<Column> for engine::Column {
    fn from(column: Column) -> Self {
        match column {
            Column::Draft => Self::Draft,
            Column::Ready => Self::Ready,
            Column::Building => Self::Building,
            Column::Review => Self::Review,
            Column::Done => Self::Done,
        }
    }
}

impl From<engine::Column> for Column {
    fn from(column: engine::Column) -> Self {
        match column {
            engine::Column::Draft => Self::Draft,
            engine::Column::Ready => Self::Ready,
            engine::Column::Building => Self::Building,
            engine::Column::Review => Self::Review,
            engine::Column::Done => Self::Done,
        }
    }
}

/// One ticket on the board, with the pages it links out to
// wire format: the Board panel reads these fields
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct TicketView {
    /// Its number
    pub id: String,
    /// Its title
    pub title: String,
    /// The number of its priority label, `None` when it has none
    pub priority: Option<u8>,
    /// Where it sits
    pub column: Column,
    /// Its issue's page on GitHub
    pub url: String,
    /// The page of the pull request closing it, `None` when none is open
    pub pull_url: Option<String>,
}

/// Every ticket of the project, column by column, as the window shows them
// wire format: the Board panel reads these fields
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
pub struct BoardView {
    /// The tickets, in the order the board shows them
    pub tickets: Vec<TicketView>,
    /// While a rate limit is used up, the Unix second the hold lifts
    pub held_until: Option<u64>,
}

impl BoardView {
    /// The view of `board`, whose tickets live in `repo`
    #[must_use]
    pub fn of(board: &Board, repo: &ForgeSlug, held_until: Option<Timestamp>) -> Self {
        let page = |kind: &str, number: &dyn std::fmt::Display| {
            format!("https://github.com/{repo}/{kind}/{number}")
        };
        let tickets = board
            .tickets()
            .iter()
            .map(|placed| TicketView {
                id: placed.id.to_string(),
                title: placed.title.clone(),
                priority: placed.priority,
                column: placed.column.into(),
                url: page("issues", &placed.id),
                pull_url: placed.pull.map(|number| page("pull", &number)),
            })
            .collect();
        Self {
            tickets,
            held_until: held_until.map(Timestamp::secs),
        }
    }
}

/// What the board streams to the window
// wire format: the Board panel switches on `kind`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoardEvent {
    /// The board as it stands now
    Board(BoardView),
    /// GitHub cannot be read, and what to do about it
    Unavailable {
        /// What to tell the maintainer
        message: String,
    },
}

/// What a drop on a column did
// wire format: the Board panel switches on `kind`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MoveView {
    /// A worker started on the ticket, or is in line for a slot when `waiting`
    Started {
        /// No slot was free, so nothing began
        waiting: bool,
    },
    /// The ticket is already in that column
    Stay,
    /// The ticket goes back where it was, and the window shows the line
    SnapBack {
        /// Why the move does nothing
        line: String,
    },
}

impl Feed for GithubSource {
    fn repo(&self) -> &ForgeSlug {
        GithubSource::repo(self)
    }

    fn viewer(&self) -> Result<String, ForgeError> {
        GithubSource::viewer(self)
    }

    fn lifts_at(&self) -> Option<Timestamp> {
        GithubSource::lifts_at(self)
    }
}

/// A poller that runs, and the board it last sent
#[derive(Debug)]
struct Watching {
    commands: Sender<Command>,
    last: Arc<Mutex<Option<BoardView>>>,
}

/// The one poller of the app, `None` before the window asks to watch
///
/// Dropping the poller's handle ends its thread within one placing, so a new
/// watch replaces the old, and [`BoardHost::stop`] ends it on a quit.
#[derive(Debug, Default)]
pub struct BoardHost {
    watching: Mutex<Option<Watching>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl BoardHost {
    /// Run `poller` on a thread of its own, in place of the one before
    ///
    /// Each event goes to `send`, and a `false` from it ends the poller.
    pub fn watch<F: Feed + 'static>(
        &self,
        mut poller: Poller<F>,
        cadence: Cadence,
        mut send: impl FnMut(BoardEvent) -> bool + Send + 'static,
    ) {
        let (commands, inbox) = mpsc::channel();
        let last = Arc::new(Mutex::new(None));
        let kept = Arc::clone(&last);
        thread::spawn(move || {
            poll::run(&mut poller, &inbox, cadence, &mut |event| {
                *lock(&kept) = match &event {
                    BoardEvent::Board(view) => Some(view.clone()),
                    BoardEvent::Unavailable { .. } => None,
                };
                send(event)
            });
        });
        *lock(&self.watching) = Some(Watching { commands, last });
    }

    /// Ask the poller to read GitHub now
    pub fn refresh(&self) {
        if let Some(watching) = lock(&self.watching).as_ref() {
            // A poller that ended has dropped its inbox, and there is nothing to read for
            let _ = watching.commands.send(Command::Refresh);
        }
    }

    /// Where `ticket` is on the board the window last got
    ///
    /// # Errors
    ///
    /// No board has been sent yet, or `ticket` is not on it.
    pub fn column_of(&self, ticket: &str) -> Result<Column, String> {
        let watching = lock(&self.watching);
        let last = watching.as_ref().map(|watching| lock(&watching.last));
        let board = last
            .as_ref()
            .and_then(|last| last.as_ref())
            .ok_or("The board has not been read yet.")?;
        board
            .tickets
            .iter()
            .find(|placed| placed.id == ticket)
            .map(|placed| placed.column)
            .ok_or_else(|| format!("Ticket {ticket} is not on the board."))
    }

    /// End the poller, so nothing polls once kelpie quits
    pub fn stop(&self) {
        lock(&self.watching).take();
    }
}

/// What dropping `ticket` on the column `to` does
///
/// `start` is what a Start does for a ticket: the move runs it only for a drop
/// from Ready on Building, and every other drop changes nothing.
///
/// # Errors
///
/// The ticket is not on the board, or `start` fails: its text.
pub fn move_ticket(
    host: &BoardHost,
    ticket: &str,
    to: Column,
    start: impl FnOnce(&str) -> Result<WorkerStarted, String>,
) -> Result<MoveView, String> {
    let from = host.column_of(ticket)?;
    Ok(match plan_move(from.into(), to.into()) {
        Move::Start => MoveView::Started {
            waiting: start(ticket)?.waiting,
        },
        Move::Stay => MoveView::Stay,
        Move::SnapBack(line) => MoveView::SnapBack {
            line: line.to_owned(),
        },
    })
}

/// The poller of the project `project`, whose tickets `source` reads
fn poller_of(
    state: &AppState,
    project: &str,
    source: Arc<GithubSource>,
) -> Result<Poller<GithubSource>, String> {
    let records = state.workers().records(project)?;
    let config = state.config().clone();
    let project = project.to_owned();
    let choose = Box::new(move || {
        let settings = Settings::load(&config, &project).map_err(|error| error.to_string())?;
        Ok(Choices {
            pick_up: settings.pick_up.value,
            ready_label: settings.ready_label.value,
            done_days: settings.done_window.value.days(),
        })
    });
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    Ok(Poller::new(source, choose, records, clock))
}

/// Stream the project's board to the window, and keep it current
///
/// The first event is the board, or why there is none. It starts one poller for
/// the project and replaces the one before. The poller ends when a send to the
/// window fails, and when kelpie quits.
///
/// # Errors
///
/// No project is picked, or its records cannot be read. A project that is not
/// on GitHub, or a missing `gh`, is sent to the window as an Unavailable.
#[tauri::command(async)]
#[specta::specta]
pub fn board_watch(state: State<'_, AppState>, on_board: Channel<BoardEvent>) -> Result<(), String> {
    let project = state.project_id(None)?;
    let source = match state.source(&project) {
        Ok(source) => source,
        Err(message) => {
            let _ = on_board.send(BoardEvent::Unavailable { message });
            return Ok(());
        }
    };
    let poller = poller_of(&state, &project, source)?;
    state.board().watch(poller, Cadence::LIVE, move |event| {
        on_board.send(event).is_ok()
    });
    Ok(())
}

/// Read GitHub now, instead of at the next minute
#[tauri::command]
#[specta::specta]
pub fn board_refresh(state: State<'_, AppState>) {
    state.board().refresh();
}

/// What dropping `ticket` on the column `to` does
///
/// A drop from Ready on Building starts a worker through the one function the
/// Start button calls. Any other drop changes nothing, and a snap back carries
/// the line the window shows.
///
/// # Errors
///
/// The ticket is not on the board, or the worker could not start. The text says why.
#[tauri::command(async)]
#[specta::specta]
pub fn board_move(
    state: State<'_, AppState>,
    ticket: String,
    to: Column,
) -> Result<MoveView, String> {
    move_ticket(state.board(), &ticket, to, |ticket| {
        workers::start_ticket(&state, ticket)
    })
}
