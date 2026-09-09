//! Temporary git repositories for the M2 tests, from `domux_server::testing` so the harness
//! and the tests build them the same way. Each test builds its own, so no test touches the
//! author's checkouts.

#[allow(unused_imports)]
pub use domux_server::testing::{commit, git, repo_with_origin};

use serde_json::json;
use std::time::Duration;

/// `Harness::api` over a socket path rather than a borrow of the harness, so two calls can be
/// in flight at the same time. `Harness::api` takes `&mut self` and would serialise them into
/// two calls that never overlap, which is a test that proves nothing.
#[allow(dead_code)]
pub async fn api_at(
    socket: &std::path::Path,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, domux_core::api::ApiError> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect");
    let (r, mut w) = stream.into_split();
    let request = json!({"id": 1, "method": method, "params": params});
    w.write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut line = String::new();
    // Longer than `Harness::api`'s own bound: these calls fetch and add a worktree, and two
    // of them are running at once on a machine that is also running the rest of the suite.
    tokio::time::timeout(
        Duration::from_secs(30),
        BufReader::new(r).read_line(&mut line),
    )
    .await
    .unwrap_or_else(|_| panic!("{method} was not answered"))
    .unwrap();
    let response: domux_core::api::Response = serde_json::from_str(&line).unwrap();
    match (response.result, response.error) {
        (Some(v), None) => Ok(v),
        (None, Some(e)) => Err(e),
        other => panic!("malformed response {other:?}"),
    }
}

/// An `events.subscribe` stream over its own connection, and the lines it produces.
#[allow(dead_code)]
pub async fn subscribe(
    socket: &std::path::Path,
    filter: &[&str],
) -> tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect");
    let (r, mut w) = stream.into_split();
    let request = json!({"id": 1, "method": "events.subscribe", "params": {"filter": filter}});
    w.write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut lines = BufReader::new(r).lines();
    let ack = next_line(&mut lines).await;
    let ack: serde_json::Value = serde_json::from_str(&ack).unwrap();
    assert!(
        ack["result"].is_object() || ack["result"].is_boolean(),
        "{ack}"
    );
    lines
}

#[allow(dead_code)]
pub async fn next_event(
    lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>,
) -> serde_json::Value {
    serde_json::from_str(&next_line(lines).await).unwrap()
}

#[allow(dead_code)]
pub async fn next_line(
    lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>,
) -> String {
    tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("the stream said nothing within 5s")
        .expect("read")
        .expect("the stream closed")
}
