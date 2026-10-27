//! Typed events. Every state change emits exactly one of these.
//!
//! Each event's first topic is its name in snake case (e.g. `deposit`), and
//! events about a user carry that user's address as the second topic, so an
//! indexer can filter by contract + event name + address.
