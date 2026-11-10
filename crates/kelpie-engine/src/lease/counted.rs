//! A counted lease: several holders at once, up to its capacity
//!
//! `cargo-test` is one, and a project can name another (`lease-capacity`,
//! `docs/design/workers.md`, Slots, pacing and leases). Each holder is one
//! command run under it, from a worker's `kelpie lease run`, known by the
//! connection it asked on. Waiters queue in arrival order and nobody is
//! preempted. A holder or waiter whose connection closes, because its command
//! ended or its run died, gives its place back ([`crate::lease`]).
//!
//! Lifted from SK's `core/lease/counted.rs` at `1dc51c040`.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::num::NonZeroU32;

use serde::Serialize;

use crate::pacer::Timestamp;
use crate::settings::CARGO_TEST;

/// Which connection asked, numbered as kelpie accepts them
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Connection(pub u64);

/// One command holding or waiting for a counted lease
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Taker {
    /// Its connection, which status leaves out
    #[serde(skip)]
    pub connection: Connection,
    /// The process that asked, as it said
    pub pid: u32,
    /// What it runs, as it said
    pub what: String,
}

/// What asking for a counted lease came to
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// Granted now
    Granted,
    /// Waiting, with this many waiters ahead
    Queued {
        /// Waiters served first
        ahead: usize,
    },
}

/// A holder and since when
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Held {
    /// Who holds it
    #[serde(flatten)]
    pub taker: Taker,
    /// Since when
    pub since: Timestamp,
}

/// A counted lease's line in kelpie's lease status
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CountedStatus {
    /// What it is for
    pub kind: Cow<'static, str>,
    /// How many may hold it at once
    pub capacity: NonZeroU32,
    /// Who holds it, longest first
    pub holders: Vec<Held>,
    /// Who waits, next first
    pub queue: Vec<Taker>,
}

/// A lease several commands hold at once
#[derive(Debug)]
pub struct CountedLease {
    kind: Cow<'static, str>,
    capacity: NonZeroU32,
    holders: Vec<Held>,
    queue: VecDeque<Taker>,
}

impl CountedLease {
    /// The `cargo-test` lease, held by up to `capacity` commands at once
    #[must_use]
    pub fn cargo_test(capacity: NonZeroU32) -> Self {
        Self::named(Cow::Borrowed(CARGO_TEST), capacity)
    }

    /// A lease called `kind`, held by up to `capacity` commands at once
    #[must_use]
    pub fn named(kind: impl Into<Cow<'static, str>>, capacity: NonZeroU32) -> Self {
        Self {
            kind: kind.into(),
            capacity,
            holders: Vec::new(),
            queue: VecDeque::new(),
        }
    }

    /// The lease's name
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Asks for the lease on behalf of `taker`, at `now`
    pub fn ask(&mut self, taker: Taker, now: Timestamp) -> Taken {
        if self.queue.is_empty() && self.has_room() {
            self.holders.push(Held { taker, since: now });
            return Taken::Granted;
        }
        self.queue.push_back(taker);
        Taken::Queued {
            ahead: self.queue.len() - 1,
        }
    }

    /// Seats `taker` at `now` whatever the room, for a command already
    /// running under a grant from a kelpie that went away
    ///
    /// Counting it keeps the next grants honest: the lease fills back to its
    /// capacity only as such runs end.
    pub fn seat(&mut self, taker: Taker, now: Timestamp) -> Taken {
        self.holders.push(Held { taker, since: now });
        Taken::Granted
    }

    /// Gives the lease back, or leaves its queue, for the connection
    /// `connection` at `now`, and returns the waiters granted the room it made
    pub fn leave(&mut self, connection: Connection, now: Timestamp) -> Vec<Connection> {
        self.queue.retain(|t| t.connection != connection);
        self.holders.retain(|h| h.taker.connection != connection);
        self.grant_waiters(now)
    }

    /// Who holds and waits for it
    #[must_use]
    pub fn status(&self) -> CountedStatus {
        CountedStatus {
            kind: self.kind.clone(),
            capacity: self.capacity,
            holders: self.holders.clone(),
            queue: self.queue.iter().cloned().collect(),
        }
    }

    /// Lets up to `capacity` commands hold it, keeping who holds and waits, and
    /// returns the waiters granted the room a raise made, oldest first
    ///
    /// A lower capacity revokes nothing: it only holds off new grants.
    pub fn set_capacity(&mut self, capacity: NonZeroU32, now: Timestamp) -> Vec<Connection> {
        self.capacity = capacity;
        self.grant_waiters(now)
    }

    // Seats waiters, oldest first, while there is room.
    fn grant_waiters(&mut self, now: Timestamp) -> Vec<Connection> {
        let mut granted = Vec::new();
        while self.has_room() {
            let Some(taker) = self.queue.pop_front() else {
                break;
            };
            granted.push(taker.connection);
            self.holders.push(Held { taker, since: now });
        }
        granted
    }

    fn has_room(&self) -> bool {
        // A capacity past usize is room for everyone.
        usize::try_from(self.capacity.get()).map_or(true, |cap| self.holders.len() < cap)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A fixed moment to ask and leave at, so no test reads the clock
    const EPOCH: u64 = 1_700_000_000;

    /// The lease under test, with the moment its asks and leaves are at
    struct Lease {
        lease: CountedLease,
        now: Timestamp,
    }

    impl Lease {
        fn ask(&mut self, n: u64) -> Taken {
            self.lease.ask(taker(n), self.now)
        }

        fn leave(&mut self, n: u64) -> Vec<Connection> {
            self.lease.leave(Connection(n), self.now)
        }

        /// Raises the capacity `secs` from now, and says where the clock landed
        /// and which waiters the raise granted
        fn resize(&mut self, capacity: u32) -> (Timestamp, Vec<Connection>) {
            let capacity = NonZeroU32::new(capacity).unwrap();
            let then = self.advance(40);
            (then, self.lease.set_capacity(capacity, then))
        }

        /// Moves the clock on `secs` and says where it landed
        fn advance(&mut self, secs: u64) -> Timestamp {
            self.now = Timestamp::from_secs(self.now.secs() + secs);
            self.now
        }
    }

    fn lease(capacity: u32) -> Lease {
        Lease {
            lease: CountedLease::cargo_test(NonZeroU32::new(capacity).unwrap()),
            now: Timestamp::from_secs(EPOCH),
        }
    }

    fn taker(n: u64) -> Taker {
        Taker {
            connection: Connection(n),
            pid: 1000 + u32::try_from(n).unwrap(),
            what: format!("cargo test {n}"),
        }
    }

    fn status(lease: &Lease) -> serde_json::Value {
        serde_json::to_value(lease.lease.status()).unwrap()
    }

    #[test]
    fn a_fourth_holder_waits_while_three_hold_it() {
        let mut lease = lease(3);
        for n in 1..=3 {
            assert_eq!(lease.ask(n), Taken::Granted, "holder {n}");
        }
        assert_eq!(lease.ask(4), Taken::Queued { ahead: 0 });
        assert_eq!(lease.ask(5), Taken::Queued { ahead: 1 });
        assert_eq!(
            status(&lease),
            json!({
                "kind": "cargo-test",
                "capacity": 3,
                "holders": [
                    { "pid": 1001, "what": "cargo test 1", "since": EPOCH },
                    { "pid": 1002, "what": "cargo test 2", "since": EPOCH },
                    { "pid": 1003, "what": "cargo test 3", "since": EPOCH },
                ],
                "queue": [
                    { "pid": 1004, "what": "cargo test 4" },
                    { "pid": 1005, "what": "cargo test 5" },
                ],
            })
        );
    }

    #[test]
    fn a_holder_that_leaves_lets_the_next_waiter_in_from_then() {
        let mut lease = lease(2);
        lease.ask(1);
        lease.ask(2);
        lease.ask(3);
        lease.ask(4);
        let then = lease.advance(40);
        assert_eq!(lease.leave(2), [Connection(3)]);
        let line = status(&lease);
        assert_eq!(line["holders"][1]["pid"], 1003);
        assert_eq!(line["holders"][1]["since"], then.secs());
        assert_eq!(
            line["queue"],
            json!([{ "pid": 1004, "what": "cargo test 4" }])
        );
    }

    #[test]
    fn a_run_seated_over_capacity_holds_off_the_next_grant_until_it_ends() {
        let mut lease = lease(1);
        lease.ask(1);
        let now = lease.now;
        assert_eq!(lease.lease.seat(taker(2), now), Taken::Granted);
        lease.ask(3);
        assert_eq!(lease.leave(1), [], "two holders on one place");
        assert_eq!(lease.leave(2), [Connection(3)]);
    }

    #[test]
    fn a_waiter_that_leaves_loses_its_place_and_grants_nothing() {
        let mut lease = lease(1);
        lease.ask(1);
        lease.ask(2);
        lease.ask(3);
        assert_eq!(lease.leave(2), []);
        assert_eq!(lease.leave(1), [Connection(3)]);
        assert_eq!(lease.leave(9), [], "a stranger changes nothing");
    }

    #[test]
    fn a_lease_nobody_asked_for_is_empty() {
        let lease = lease(3);
        assert_eq!(
            status(&lease),
            json!({ "kind": "cargo-test", "capacity": 3, "holders": [], "queue": [] })
        );
    }

    #[test]
    fn a_raised_capacity_grants_the_queued_waiters_in_order_and_no_more() {
        let mut lease = lease(1);
        for n in 1..=5 {
            lease.ask(n);
        }
        let (then, granted) = lease.resize(3);
        assert_eq!(granted, [Connection(2), Connection(3)], "in arrival order");
        let line = status(&lease);
        assert_eq!(line["capacity"], 3);
        assert_eq!(line["holders"][1]["since"], then.secs());
        assert_eq!(line["holders"][2]["since"], then.secs());
        assert_eq!(
            line["queue"],
            json!([
                { "pid": 1004, "what": "cargo test 4" },
                { "pid": 1005, "what": "cargo test 5" },
            ])
        );
        let (_, granted) = lease.resize(3);
        assert_eq!(granted, [], "the same capacity grants nobody");
    }

    #[test]
    fn a_lowered_capacity_keeps_every_holder_and_grants_nothing_until_fewer_hold_it() {
        let mut lease = lease(3);
        for n in 1..=4 {
            lease.ask(n);
        }
        let (_, granted) = lease.resize(1);
        assert_eq!(granted, [], "a lower capacity revokes nothing");
        let line = status(&lease);
        assert_eq!(line["holders"].as_array().unwrap().len(), 3);
        assert_eq!(line["queue"].as_array().unwrap().len(), 1);
        assert_eq!(lease.ask(5), Taken::Queued { ahead: 1 });
        assert_eq!(lease.leave(1), []);
        assert_eq!(lease.leave(2), []);
        assert_eq!(lease.leave(3), [Connection(4)]);
    }
}
