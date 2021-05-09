//! Synchronization primitives for inter-thread and inter-core communication and locking.
//!
//! This module contains a number of useful high-level synchronization primitives for managing data that must be shared across multiple
//! cores or threads of execution. This is necessary for ensuring that kernel data structures remain consistent and avoiding race conditions
//! between different threads/cores running kernel code.

pub mod future;
pub mod uninterruptible;

pub use future::Future;
pub use uninterruptible::UninterruptibleSpinlock;
