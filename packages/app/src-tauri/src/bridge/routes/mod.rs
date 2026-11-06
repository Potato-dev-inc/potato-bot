//! Route groups of the local bridge. The Potato Bot brain and its sandbox screen are answered by
//! `http`, and this router only serves health for the rest.

pub(super) mod potato_brain;
pub(super) mod potato_screen;
#[cfg(test)]
pub(super) mod potato_screen_tests;

