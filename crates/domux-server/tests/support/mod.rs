//! Temporary git repositories for the M2 tests, from `domux_server::testing` so the harness
//! and the tests build them the same way. Each test builds its own, so no test touches the
//! author's checkouts.

#[allow(unused_imports)]
pub use domux_server::testing::{commit, git, repo_with_origin};
