//! One listener, two protocols. The first byte of a connection decides: `{` starts the
//! newline-delimited JSON control API; anything else is a length-prefixed attach frame.

use crate::core::CoreMsg;
use anyhow::Context;
use domux_core::api::{ApiError, Method, Request, Response};
use domux_core::ids::ClientId;
use domux_core::proto::{
    decode, encode, is_control_api_first_byte, ClientMsg, Decoder, Hello, ServerMsg,
    PROTOCOL_VERSION,
};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Binds the socket at `path`, private to this user. A stale file from a server that died is
/// removed; a live server's is refused.
pub fn bind(path: &Path) -> anyhow::Result<std::os::unix::net::UnixListener> {
    if let Some(dir) = path.parent() {
        // Only a directory this server creates is made private. The socket's parent is
        // whatever `DOMUX_SOCKET` points at, and narrowing a directory domux does not own is
        // a side effect on someone else's files: `DOMUX_SOCKET=$HOME/s.sock` used to chmod
        // the home directory to 0700, and `/tmp/s.sock` failed with a bare "Operation not
        // permitted" because /tmp belongs to root.
        let existed = dir.exists();
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        #[cfg(unix)]
        if !existed {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .with_context(|| format!("make {} private", dir.display()))?;
        }
    }
    if path.exists() {
        // A stale file from a server that died. A live server answers a connect.
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            anyhow::bail!("a server is already listening on {}", path.display());
        }
        std::fs::remove_file(path)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(path)
        .with_context(|| format!("bind {}", path.display()))?;
    // The socket is what access control rests on, not the directory around it: anyone who can
    // connect can spawn processes in this user's panes. `bind` takes its mode from the umask,
    // which is 022 on a stock shell, so without this the socket is srwxr-xr-x and any user on
    // the machine can drive the server.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("make {} private", path.display()))?;
    }
    Ok(listener)
}

enum Command {
    Pause,
    Resume,
    GiveBack(oneshot::Sender<std::os::unix::net::UnixListener>),
}

/// The task that accepts connections, and what an upgrade needs from it (decision 0045): to
/// stop accepting while connections already open finish, to start again when the upgrade does
/// not happen, and to give the listener back for the new server. A connection made while it is
/// not accepting waits in the kernel's backlog.
pub struct Acceptor {
    commands: mpsc::UnboundedSender<Command>,
    task: JoinHandle<()>,
    /// Control API connections open now, not counting event streams.
    pub open_control: Arc<AtomicUsize>,
    /// Attached clients whose writer has not yet ended, so whose last message may not yet be
    /// on the socket.
    pub open_writers: Arc<AtomicUsize>,
}

impl Acceptor {
    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    pub fn resume(&self) {
        let _ = self.commands.send(Command::Resume);
    }

    /// Stops accepting for good and answers the listener, still bound. `None` when the task
    /// has already ended.
    pub async fn give_back(&self) -> Option<std::os::unix::net::UnixListener> {
        let (tx, rx) = oneshot::channel();
        self.commands.send(Command::GiveBack(tx)).ok()?;
        rx.await.ok()
    }
}

impl Drop for Acceptor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Counts one open thing for as long as it lives.
struct Open(Arc<AtomicUsize>);

impl Open {
    fn new(gauge: &Arc<AtomicUsize>) -> Open {
        gauge.fetch_add(1, Ordering::SeqCst);
        Open(gauge.clone())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Accepts connections on `listener` and serves each on its own task.
pub fn serve(
    listener: std::os::unix::net::UnixListener,
    core_tx: mpsc::Sender<CoreMsg>,
) -> anyhow::Result<Acceptor> {
    listener
        .set_nonblocking(true)
        .context("make the listening socket non-blocking")?;
    let listener = UnixListener::from_std(listener).context("serve the listening socket")?;
    let (commands, mut rx) = mpsc::unbounded_channel();
    let open_control = Arc::new(AtomicUsize::new(0));
    let open_writers = Arc::new(AtomicUsize::new(0));
    let gauges = Gauges {
        control: open_control.clone(),
        writers: open_writers.clone(),
    };
    let task = tokio::spawn(async move {
        let mut paused = false;
        loop {
            tokio::select! {
                command = rx.recv() => match command {
                    None => return,
                    Some(Command::Pause) => paused = true,
                    Some(Command::Resume) => paused = false,
                    Some(Command::GiveBack(tx)) => {
                        match listener.into_std() {
                            Ok(listener) => {
                                let _ = tx.send(listener);
                            }
                            Err(e) => tracing::error!("the listening socket could not be given back: {e}"),
                        }
                        return;
                    }
                },
                accepted = listener.accept(), if !paused => match accepted {
                    Ok((stream, _)) => {
                        let tx = core_tx.clone();
                        let gauges = gauges.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle(stream, tx, gauges).await {
                                tracing::debug!("connection ended: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("accept failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                },
            }
        }
    });
    Ok(Acceptor {
        commands,
        task,
        open_control,
        open_writers,
    })
}

#[derive(Clone)]
struct Gauges {
    control: Arc<AtomicUsize>,
    writers: Arc<AtomicUsize>,
}

async fn handle(
    mut stream: UnixStream,
    core_tx: mpsc::Sender<CoreMsg>,
    gauges: Gauges,
) -> anyhow::Result<()> {
    let mut first = [0u8; 1];
    if stream.read_exact(&mut first).await.is_err() {
        return Ok(());
    }
    if is_control_api_first_byte(first[0]) {
        control(stream, first[0], core_tx, Open::new(&gauges.control)).await
    } else {
        attach(stream, first[0], core_tx, gauges.writers).await
    }
}

async fn control(
    stream: UnixStream,
    first: u8,
    core_tx: mpsc::Sender<CoreMsg>,
    open: Open,
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
            // A stream stays open for as long as its reader wants it, so it is not a call an
            // upgrade waits for.
            drop(open);
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
    /// Taken by `drop`, so the message goes exactly once.
    client: Option<ClientId>,
}

impl Drop for ClientCleanup {
    fn drop(&mut self) {
        self.writer.abort();
        let Some(client) = self.client.take() else {
            return;
        };
        // `Drop` cannot await, and `try_send` would throw the message away whenever the
        // core is momentarily behind, which is exactly when a client is leaking. The task
        // holds a sender and waits for room instead; when the core has stopped the channel
        // closes, the send fails and the task ends, so nothing is held open either way.
        let core_tx = self.core_tx.clone();
        tokio::spawn(async move {
            let _ = core_tx.send(CoreMsg::ClientGone { client }).await;
        });
    }
}

/// The head of a hello from a client built for another protocol or version, which the core then
/// refuses with the restart sentence. A hello whose head names this build's version and
/// protocol and still does not decode is corrupt, and gets `None`: the connection ends.
fn old_hello(body: &[u8]) -> Option<Hello> {
    Hello::from_head(body)
        .filter(|h| h.version != domux_core::VERSION || h.protocol != PROTOCOL_VERSION)
}

async fn attach(
    stream: UnixStream,
    first: u8,
    core_tx: mpsc::Sender<CoreMsg>,
    writers: Arc<AtomicUsize>,
) -> anyhow::Result<()> {
    let (mut r, mut w) = stream.into_split();
    let mut dec = Decoder::default();
    dec.push(&[first]);
    let mut buf = vec![0u8; 64 * 1024];
    // The first message must be the hello.
    let hello = loop {
        if let Some(body) = dec.next_frame()? {
            match decode::<ClientMsg>(&body) {
                Ok(ClientMsg::Hello(h)) => break h,
                Ok(_) => anyhow::bail!("first message was not a hello"),
                Err(e) => break old_hello(&body).ok_or(e)?,
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
    let open = Open::new(&writers);
    let writer = tokio::spawn(async move {
        let _open = open;
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
        client: Some(client.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cleanup_waits_for_room_to_send_client_gone() {
        let (core_tx, mut core_rx) = mpsc::channel(1);
        core_tx.send(CoreMsg::Tick).await.unwrap();
        let writer = tokio::spawn(std::future::pending::<()>());
        drop(ClientCleanup {
            writer,
            core_tx,
            client: Some(ClientId("c_0d77".into())),
        });
        assert!(matches!(core_rx.recv().await, Some(CoreMsg::Tick)));
        let message = tokio::time::timeout(std::time::Duration::from_secs(1), core_rx.recv())
            .await
            .expect("cleanup sends when room becomes available")
            .expect("cleanup keeps a sender until it sends");
        assert!(matches!(
            message,
            CoreMsg::ClientGone { client } if client == ClientId("c_0d77".into())
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), core_rx.recv())
                .await
                .expect("cleanup releases its sender after sending")
                .is_none(),
            "cleanup sends only once"
        );
    }
}
