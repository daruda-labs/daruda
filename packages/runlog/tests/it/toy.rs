//! A small machine with every shape a domain machine has: a bounded counter,
//! a set with add/remove preconditions, and tickets numbered by `assign`.

use std::collections::BTreeSet;

use daruda_runlog::Machine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToyState {
    pub total: i64,
    pub members: BTreeSet<String>,
    pub tickets: Vec<u32>,
    pub next_ticket: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToyEvent {
    Add {
        n: i64,
    },
    Insert {
        name: String,
    },
    Remove {
        name: String,
    },
    /// `number` is owned by the log: proposers send 0 and `assign` fills it.
    Ticket {
        number: u32,
    },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ToyRejected {
    #[error("the total would exceed {LIMIT}")]
    OverLimit,
    #[error("{0} is already a member")]
    Duplicate(String),
    #[error("{0} is not a member")]
    Missing(String),
}

pub const LIMIT: i64 = 100;

pub struct Toy;

impl Machine for Toy {
    type State = ToyState;
    type Event = ToyEvent;
    type Rejected = ToyRejected;

    fn check(state: &ToyState, event: &ToyEvent) -> Result<(), ToyRejected> {
        match event {
            ToyEvent::Add { n } if state.total + n > LIMIT => Err(ToyRejected::OverLimit),
            ToyEvent::Insert { name } if state.members.contains(name) => {
                Err(ToyRejected::Duplicate(name.clone()))
            }
            ToyEvent::Remove { name } if !state.members.contains(name) => {
                Err(ToyRejected::Missing(name.clone()))
            }
            _ => Ok(()),
        }
    }

    fn assign(state: &ToyState, event: &mut ToyEvent) {
        if let ToyEvent::Ticket { number } = event {
            *number = state.next_ticket + 1;
        }
    }

    fn apply(state: &mut ToyState, event: &ToyEvent) {
        match event {
            ToyEvent::Add { n } => state.total += n,
            ToyEvent::Insert { name } => {
                state.members.insert(name.clone());
            }
            ToyEvent::Remove { name } => {
                state.members.remove(name);
            }
            ToyEvent::Ticket { number } => {
                state.tickets.push(*number);
                state.next_ticket = *number;
            }
        }
    }
}

/// `Toy` as a later build declares it once its events have changed.
pub struct NewerToy;

impl Machine for NewerToy {
    type State = ToyState;
    type Event = ToyEvent;
    type Rejected = ToyRejected;
    const SCHEMA: u32 = Toy::SCHEMA + 1;

    fn check(state: &ToyState, event: &ToyEvent) -> Result<(), ToyRejected> {
        Toy::check(state, event)
    }

    fn assign(state: &ToyState, event: &mut ToyEvent) {
        Toy::assign(state, event);
    }

    fn apply(state: &mut ToyState, event: &ToyEvent) {
        Toy::apply(state, event);
    }
}

pub fn add(n: i64) -> ToyEvent {
    ToyEvent::Add { n }
}

pub fn insert(name: &str) -> ToyEvent {
    ToyEvent::Insert { name: name.into() }
}

pub fn remove(name: &str) -> ToyEvent {
    ToyEvent::Remove { name: name.into() }
}

pub fn ticket() -> ToyEvent {
    ToyEvent::Ticket { number: 0 }
}

/// Ten events, two of which the machine rejects at their point in the run.
pub fn scenario() -> Vec<ToyEvent> {
    vec![
        add(10),
        ticket(),
        insert("a"),
        insert("a"),
        ticket(),
        add(95),
        insert("b"),
        remove("a"),
        ticket(),
        add(-5),
    ]
}

pub const BUILD: &str = "test-build";
