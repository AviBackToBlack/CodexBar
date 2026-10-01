//! Count-only diagnostics for stored process environments (upstream 0.70.0, #4106).
//!
//! A process environment carries API keys, tokens and cookies. When a struct that stores one
//! derives `Debug`, `{:?}`, `dbg!`, a `tracing` field capture or an `assert_eq!` failure prints
//! every value. [`ProcessEnvironment`] keeps the original values for the child process and
//! renders only how many entries it holds.
//!
//! Every stored environment uses the wrapper. The `storage_guard` test scans the shipped Rust
//! sources and fails when an environment-named field holds a plain string map or string pair
//! list.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::ops::{Deref, DerefMut};

/// A stored process environment whose `Debug` output is `ProcessEnvironment(N entries; redacted)`.
///
/// Explicit access (deref, iteration, `get`, `insert`) still returns the original values for
/// subprocess use, so never log what it returns. Wrapping an `Option` keeps `None` distinct from
/// an empty map, and equality compares the original contents.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProcessEnvironment<T>(T);

impl<T> ProcessEnvironment<T> {
    /// Wraps an environment for storage.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Returns the original environment, for handing to a child process.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> From<T> for ProcessEnvironment<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T> Deref for ProcessEnvironment<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for ProcessEnvironment<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: EnvironmentEntries> fmt::Debug for ProcessEnvironment<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ProcessEnvironment({} entries; redacted)",
            self.0.environment_entries()
        )
    }
}

impl<T: IntoIterator> IntoIterator for ProcessEnvironment<T> {
    type Item = T::Item;
    type IntoIter = T::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a ProcessEnvironment<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        (&self.0).into_iter()
    }
}

/// The entry count of a stored environment, the only detail [`ProcessEnvironment`] renders.
pub trait EnvironmentEntries {
    /// Number of variables held; an absent environment counts as zero.
    fn environment_entries(&self) -> usize;
}

impl<K, V, S> EnvironmentEntries for HashMap<K, V, S> {
    fn environment_entries(&self) -> usize {
        self.len()
    }
}

impl<K, V> EnvironmentEntries for BTreeMap<K, V> {
    fn environment_entries(&self) -> usize {
        self.len()
    }
}

impl<K, V> EnvironmentEntries for Vec<(K, V)> {
    fn environment_entries(&self) -> usize {
        self.len()
    }
}

impl<T: EnvironmentEntries> EnvironmentEntries for Option<T> {
    fn environment_entries(&self) -> usize {
        self.as_ref()
            .map_or(0, EnvironmentEntries::environment_entries)
    }
}

#[cfg(test)]
mod storage_guard;
#[cfg(test)]
mod tests;
