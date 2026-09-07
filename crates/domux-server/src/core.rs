//! The core task. Task 14 adds `Core` and `run`; this file starts with the message type
//! every producer sends.

use domux_core::api::{Event, Request, Response};
use domux_core::ids::{ClientId, PaneId};
use domux_core::model::Model;
use domux_core::proto::{ClientMsg, Hello, ServerMsg};
use tokio::sync::{mpsc, oneshot};

pub enum CoreMsg {
    PaneOutput {
        pane: PaneId,
        bytes: Vec<u8>,
    },
    PaneExited {
        pane: PaneId,
        status: Option<i32>,
    },
    ClientConnected {
        hello: Hello,
        tx: mpsc::Sender<ServerMsg>,
        reply: oneshot::Sender<Result<ClientId, String>>,
    },
    ClientInput {
        client: ClientId,
        msg: ClientMsg,
    },
    ClientGone {
        client: ClientId,
    },
    Api {
        request: Request,
        reply: oneshot::Sender<Response>,
    },
    Subscribe {
        filter: Vec<String>,
        tx: mpsc::Sender<Event>,
    },
    /// Once a second: the process inspector, the clock, exited-pane cleanup.
    Tick,
    Snapshot {
        reply: oneshot::Sender<Model>,
    },
    Shutdown,
}
