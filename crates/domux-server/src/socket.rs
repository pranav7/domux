//! One listener, two protocols. The first byte of a connection decides: `{` starts the
//! newline-delimited JSON control API; anything else is a length-prefixed attach frame.

use crate::core::CoreMsg;
use anyhow::Context;
use domux_core::api::{ApiError, Method, Request, Response};
use domux_core::ids::ClientId;
use domux_core::proto::{encode, is_control_api_first_byte, ClientMsg, Decoder, ServerMsg};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

pub async fn listen(path: &Path, core_tx: mpsc::Sender<CoreMsg>) -> anyhow::Result<JoinHandle<()>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    if path.exists() {
        // A stale file from a server that died. A live server answers a connect.
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            anyhow::bail!("a server is already listening on {}", path.display());
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path).with_context(|| format!("bind {}", path.display()))?;
    Ok(tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let tx = core_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle(stream, tx).await {
                            tracing::debug!("connection ended: {e}");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!("accept failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    }))
}

async fn handle(mut stream: UnixStream, core_tx: mpsc::Sender<CoreMsg>) -> anyhow::Result<()> {
    let mut first = [0u8; 1];
    if stream.read_exact(&mut first).await.is_err() {
        return Ok(());
    }
    if is_control_api_first_byte(first[0]) {
        control(stream, first[0], core_tx).await
    } else {
        attach(stream, first[0], core_tx).await
    }
}

async fn control(
    stream: UnixStream,
    first: u8,
    core_tx: mpsc::Sender<CoreMsg>,
) -> anyhow::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    let mut carry = Some(first);
    while let Some(mut line) = lines.next_line().await? {
        if let Some(c) = carry.take() {
            line.insert(0, c as char);
        }
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::err(
                    serde_json::Value::Null,
                    ApiError::invalid_params(format!("request is not valid JSON: {e}")),
                );
                w.write_all(format!("{}\n", serde_json::to_string(&resp)?).as_bytes())
                    .await?;
                continue;
            }
        };
        if request.method == "events.subscribe" {
            let filter = match Method::from_request(&request.method, request.params.clone()) {
                Ok(Method::EventsSubscribe(p)) => p.filter,
                Ok(_) => Vec::new(),
                Err(e) => {
                    w.write_all(
                        format!(
                            "{}\n",
                            serde_json::to_string(&Response::err(request.id, e))?
                        )
                        .as_bytes(),
                    )
                    .await?;
                    continue;
                }
            };
            let (tx, mut rx) = mpsc::channel(256);
            core_tx.send(CoreMsg::Subscribe { filter, tx }).await?;
            w.write_all(
                format!(
                    "{}\n",
                    serde_json::to_string(&Response::ok(
                        request.id,
                        serde_json::json!({"ok": true})
                    ))?
                )
                .as_bytes(),
            )
            .await?;
            while let Some(event) = rx.recv().await {
                w.write_all(format!("{}\n", serde_json::to_string(&event)?).as_bytes())
                    .await?;
            }
            return Ok(());
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        core_tx
            .send(CoreMsg::Api {
                request,
                reply: reply_tx,
            })
            .await?;
        let response = reply_rx.await?;
        w.write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())
            .await?;
    }
    Ok(())
}

/// Ends a connection's server-side state however the connection ends, a panic on the read
/// path included. Neither half goes away on its own: the writer task owns the socket's write
/// half and waits on a channel that only the core closes, and the core holds the other end
/// of that channel in its `ClientConn`. Without this, a read error would leave both alive
/// and the model would go on drawing frames for a client that is gone.
struct ClientCleanup {
    writer: JoinHandle<()>,
    core_tx: mpsc::Sender<CoreMsg>,
    client: ClientId,
}

impl Drop for ClientCleanup {
    fn drop(&mut self) {
        // `try_send`, because `Drop` cannot await. The core drains a 1024-deep channel
        // continuously, so the only way this queue is full is that the core has stopped,
        // which is the one case where the message has nothing left to do.
        let _ = self.core_tx.try_send(CoreMsg::ClientGone {
            client: self.client.clone(),
        });
        self.writer.abort();
    }
}

async fn attach(
    stream: UnixStream,
    first: u8,
    core_tx: mpsc::Sender<CoreMsg>,
) -> anyhow::Result<()> {
    let (mut r, mut w) = stream.into_split();
    let mut dec = Decoder::default();
    dec.push(&[first]);
    let mut buf = vec![0u8; 64 * 1024];
    // The first message must be the hello.
    let hello = loop {
        if let Some(msg) = dec.next::<ClientMsg>()? {
            match msg {
                ClientMsg::Hello(h) => break h,
                _ => anyhow::bail!("first message was not a hello"),
            }
        }
        let n = r.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        dec.push(&buf[..n]);
    };
    let (tx, mut rx) = mpsc::channel::<ServerMsg>(256);
    let (reply_tx, reply_rx) = oneshot::channel();
    core_tx
        .send(CoreMsg::ClientConnected {
            hello,
            tx,
            reply: reply_tx,
        })
        .await?;
    let client = match reply_rx.await? {
        Ok(id) => id,
        Err(reason) => {
            w.write_all(&encode(&ServerMsg::Refused { reason })?)
                .await?;
            return Ok(());
        }
    };
    let writer_client = client.clone();
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let done = matches!(msg, ServerMsg::Detached { .. });
            // A message that cannot be framed cannot be delivered, and skipping it would
            // desynchronise this client silently. End the stream instead, which is the
            // same path a write error takes, so the client reattaches and resyncs.
            let Ok(bytes) = encode(&msg) else {
                tracing::error!(client = %writer_client, "a message could not be framed, ending the stream");
                break;
            };
            if w.write_all(&bytes).await.is_err() {
                break;
            }
            if done {
                break;
            }
        }
    });
    let _cleanup = ClientCleanup {
        writer,
        core_tx: core_tx.clone(),
        client: client.clone(),
    };
    loop {
        while let Some(msg) = dec.next::<ClientMsg>()? {
            core_tx
                .send(CoreMsg::ClientInput {
                    client: client.clone(),
                    msg,
                })
                .await?;
        }
        let n = r.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        dec.push(&buf[..n]);
    }
    Ok(())
}
