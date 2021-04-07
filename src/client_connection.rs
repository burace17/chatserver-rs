/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use futures_util::{StreamExt, SinkExt};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::error::Error;
use tokio::sync::{mpsc, watch};
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use super::server::{WebSocketStream, Message, ServerCommandResponse};
use super::commands::CommandError;
use tokio_stream::wrappers::WatchStream;

struct ClientStream {
    manager_rx: Pin<Box<dyn tokio_stream::Stream<Item = ServerCommandResponse> + Send>>,
    ws_rx: futures_util::stream::SplitStream<WebSocketStream>,
    shutdown_rx: WatchStream<bool>
}

enum ClientStreamMessage {
    WebSocketText(String),
    WebSocketPing(Vec<u8>),
    WebSocketClose,
    ManagerData(ServerCommandResponse),
    ShutdownStatus(bool)
}

impl tokio_stream::Stream for ClientStream {
    type Item = ClientStreamMessage;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Some(v)) = Pin::new(&mut self.manager_rx).poll_next(cx) {
            Poll::Ready(Some(ClientStreamMessage::ManagerData(v)))
        }
        else if let Poll::Ready(Some(v)) = Pin::new(&mut self.ws_rx).poll_next(cx) {
            match v {
                Ok(packet) => {
                    match packet {
                        tungstenite::Message::Text(msg) => Poll::Ready(Some(ClientStreamMessage::WebSocketText(msg))),
                        tungstenite::Message::Ping(data) => Poll::Ready(Some(ClientStreamMessage::WebSocketPing(data))),
                        tungstenite::Message::Close(_) => Poll::Ready(Some(ClientStreamMessage::WebSocketClose)),
                        _ => Poll::Ready(None),
                    }
                },
                Err(_) => Poll::Ready(None)
            }
        }
        else if let Poll::Ready(Some(shutdown)) = Pin::new(&mut self.shutdown_rx).poll_next(cx) {
            Poll::Ready(Some(ClientStreamMessage::ShutdownStatus(shutdown)))
        }
        else {
            Poll::Pending
        }
    }
}

fn error_to_close_frame<'a>(error: CommandError) -> CloseFrame<'a> {
    fn to_code(code: u16) -> CloseCode {
        CloseCode::Library(code)
    }
    let reason = std::borrow::Cow::from(error.to_string());
    let code = match error {
        CommandError::MissingCommand => to_code(4000),
        CommandError::InvalidJSON(_) => to_code(4001),
        CommandError::InvalidArguments => to_code(4002),
        CommandError::InvalidUsername => to_code(4003),
        CommandError::SendFailed(_) => to_code(4004),
        CommandError::LoginDBMSError(_) => to_code(4005),
        CommandError::LoginFailed => to_code(4006),
        CommandError::NeedAuth => to_code(4007),
        CommandError::NotInChannel => to_code(4008),
        CommandError::TimeError(_) => to_code(4009),
        CommandError::DidNotAuth => to_code(4010),
        CommandError::AlreadyRegistered => to_code(4011),
    };

    CloseFrame{ code, reason }
}

fn get_normal_close_frame<'a>() -> CloseFrame<'a> {
    CloseFrame {
        code: CloseCode::Normal,
        reason: "Bye".into()
    }
}

pub async fn process_client(addr: SocketAddr, websocket: WebSocketStream, tx: mpsc::Sender<Message>,
                            shutdown_receiver: watch::Receiver<bool>) -> Result<(), Box<dyn Error>> {
    // Make a channel that the manager task can use to send us messages.
    // This is data that we need to send back to the client over the websocket.
    let (mtx, mut mrx) = mpsc::channel::<ServerCommandResponse>(32);

    // Wrap the receiver in a stream.
    let mrx_stream = Box::pin(async_stream::stream! {
        while let Some(item) = mrx.recv().await {
            yield item;
        }
    });

    // Tell the manager we have a new client, and send them the IP and transmit channel for this client.
    tx.send(Message::NewConnection((addr, mtx))).await?;

    // Get the websocket channels and construct the stream object we use to wait on results from both the
    // websocket and the manager.
    let (mut ws_tx, ws_rx) = websocket.split();

    let shutdown_stream = WatchStream::new(shutdown_receiver);
    let mut client_stream = ClientStream{ manager_rx: mrx_stream, ws_rx, shutdown_rx: shutdown_stream };

    // This will loop as long as the websocket is still connected.
    while let Some(result) = client_stream.next().await {
        match result {
            ClientStreamMessage::ManagerData(response) => {
                match response {
                    ServerCommandResponse::Text(msg) => ws_tx.send(tungstenite::Message::Text(msg)).await?,
                    ServerCommandResponse::Disconnect(e) => {
                        ws_tx.send(tungstenite::Message::Close(Some(error_to_close_frame(e)))).await?;
                        break;
                    }
                }
            }
            ClientStreamMessage::WebSocketPing(data) => ws_tx.send(tungstenite::Message::Pong(data)).await?,
            ClientStreamMessage::WebSocketText(data) => tx.send(Message::NewData((addr, data))).await?,
            ClientStreamMessage::WebSocketClose => {
                tx.send(Message::Disconnected(addr)).await?;
                ws_tx.send(tungstenite::Message::Close(Some(get_normal_close_frame()))).await?;
            }
            ClientStreamMessage::ShutdownStatus(shutdown) => {
                if shutdown {
                    ws_tx.send(tungstenite::Message::Close(Some(get_normal_close_frame()))).await?;
                    break;
                }
            }
        }
    }

    tx.send(Message::Disconnected(addr)).await?;
    Ok(())
}