#![crate_type = "lib"]
#![cfg_attr(not(feature = "cargo-clippy"), allow(unknown_lints))]
#![deny(trivial_numeric_casts, warnings)]
#![allow(broken_intra_doc_links)]
#![allow(
    clippy::too_many_arguments,
    clippy::implicit_hasher,
    clippy::module_inception,
    clippy::new_without_default
)]

#[allow(unused)]
#[macro_use]
extern crate log;

#[macro_use]
extern crate serde;

#[macro_use]
extern crate quickcheck_derive;

// mod ephemeral;
// mod friend;
// mod funder;
// mod handler;
// mod liveness;
mod mutual_credit;
// pub mod report;
// mod state;
#[allow(unused)]
mod token_channel;

mod switch;
//
// For testing:

pub mod types;

// #[cfg(test)]
// mod tests;

// pub use self::funder::{funder_loop, FunderError};
// pub use self::state::{FunderMutation, FunderState};
