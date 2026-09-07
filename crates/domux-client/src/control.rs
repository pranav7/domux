//! The control API from the client side: one JSON line out, one in.

use domux_core::api::{ApiError, Request, Response};
use serde_json::Value;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Whether a server is listening. A socket file left behind by a server that died does not
/// answer a connect, so the file existing is not the question.
pub fn is_live(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

/// One call: the request line out, the response line in. The outer result is the transport,
/// the inner one is the server's own answer.
pub async fn call(
    socket: &Path,
    method: &str,
    params: Value,
) -> anyhow::Result<Result<Value, ApiError>> {
    let stream = UnixStream::connect(socket).await?;
    let (r, mut w) = stream.into_split();
    let req = Request {
        id: Value::from(1),
        method: method.to_string(),
        params,
    };
    w.write_all(format!("{}\n", serde_json::to_string(&req)?).as_bytes())
        .await?;
    let mut line = String::new();
    BufReader::new(r).read_line(&mut line).await?;
    let resp: Response = serde_json::from_str(&line)?;
    Ok(match (resp.result, resp.error) {
        (Some(v), _) => Ok(v),
        (None, Some(e)) => Err(e),
        // Neither half is a server bug. The caller gets an error to print rather than an
        // empty success that reads as "it worked".
        (None, None) => Err(ApiError::internal(
            "the server sent neither a result nor an error",
        )),
    })
}

/// Streams events until the connection closes or `on_event` returns false.
pub async fn subscribe(
    socket: &Path,
    filter: Vec<String>,
    mut on_event: impl FnMut(Value) -> bool,
) -> anyhow::Result<()> {
    let stream = UnixStream::connect(socket).await?;
    let (r, mut w) = stream.into_split();
    let req = Request {
        id: Value::from(1),
        method: "events.subscribe".into(),
        params: serde_json::json!({ "filter": filter }),
    };
    w.write_all(format!("{}\n", serde_json::to_string(&req)?).as_bytes())
        .await?;
    let mut lines = BufReader::new(r).lines();
    let ack = lines
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("the server closed the connection"))?;
    let ack: Response = serde_json::from_str(&ack)?;
    if let Some(e) = ack.error {
        anyhow::bail!("{}", e.message);
    }
    while let Some(line) = lines.next_line().await? {
        if !on_event(serde_json::from_str(&line)?) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::api::{ErrorCode, Request};
    use serde_json::json;
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixListener;

    /// A server that answers one connection with canned lines and hands back what it read.
    fn spawn_server(
        path: std::path::PathBuf,
        replies: Vec<String>,
    ) -> tokio::task::JoinHandle<String> {
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = tokio::io::BufReader::new(r).lines();
            let request = lines.next_line().await.unwrap().unwrap();
            for reply in replies {
                w.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
            }
            request
        })
    }

    #[tokio::test]
    async fn call_sends_one_request_line_and_reads_one_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s");
        let server = spawn_server(
            path.clone(),
            vec![r#"{"id":1,"result":{"ok":true}}"#.into()],
        );
        let got = call(&path, "tab.create", json!({ "name": "x" }))
            .await
            .unwrap();
        assert_eq!(got, Ok(json!({ "ok": true })));
        let request: Request = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(request.method, "tab.create");
        assert_eq!(request.params, json!({ "name": "x" }));
    }

    #[tokio::test]
    async fn call_hands_back_the_error_the_server_sent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s");
        let server = spawn_server(
            path.clone(),
            vec![r#"{"id":1,"error":{"code":"not_found","message":"no tab named x"}}"#.into()],
        );
        let got = call(&path, "tab.select", json!({})).await.unwrap();
        let err = got.unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(err.message, "no tab named x");
        server.await.unwrap();
    }

    /// A response with neither half is a server bug. It becomes an error the caller can
    /// print, never an empty success.
    #[tokio::test]
    async fn call_refuses_a_response_with_neither_a_result_nor_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s");
        let server = spawn_server(path.clone(), vec![r#"{"id":1}"#.into()]);
        let got = call(&path, "server.status", json!({})).await.unwrap();
        assert_eq!(got.unwrap_err().code, ErrorCode::Internal);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn subscribe_stops_when_the_handler_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s");
        let server = spawn_server(
            path.clone(),
            vec![
                r#"{"id":1,"result":{"ok":true}}"#.into(),
                r#"{"event":"tab.created","workspace":"w_1","tab":"t_1"}"#.into(),
                r#"{"event":"tab.closed","workspace":"w_1","tab":"t_1"}"#.into(),
                r#"{"event":"server.stopping"}"#.into(),
            ],
        );
        let mut seen: Vec<String> = Vec::new();
        subscribe(&path, vec!["tab.*".into()], |event| {
            seen.push(event["event"].as_str().unwrap().to_string());
            seen.len() < 2
        })
        .await
        .unwrap();
        assert_eq!(seen, vec!["tab.created", "tab.closed"]);
        let request: Request = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(request.method, "events.subscribe");
        assert_eq!(request.params, json!({ "filter": ["tab.*"] }));
    }

    #[tokio::test]
    async fn subscribe_reports_a_refused_subscription() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s");
        let server = spawn_server(
            path.clone(),
            vec![r#"{"id":1,"error":{"code":"invalid_params","message":"no such event"}}"#.into()],
        );
        let err = subscribe(&path, vec!["nope".into()], |_| true)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "no such event");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn is_live_answers_for_a_socket_with_and_without_a_server() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s");
        assert!(!is_live(&path), "no socket file at all");
        let _listener = UnixListener::bind(&path).unwrap();
        assert!(is_live(&path));
    }
}
