#![no_std]

extern crate alloc;

pub mod store;
mod v1;

pub use store::{Store, StoreError, MAX_NAME_LEN};
