//! Crypto shared by teams and Spaces. Moved from team/crypto.rs in Spaces D1.1; bytes are unchanged
//! (see golden_tests.rs).
pub mod b64;
pub mod crypto;
#[allow(dead_code)]
pub mod grant;
#[allow(dead_code)]
pub mod sign;

#[cfg(test)]
mod golden_tests;
