use futures_util::{StreamExt, SinkExt};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::error::Error;
use tokio::sync::mpsc;
use super::server::{WebSocketStream, Message, ServerCommandResponse};
use super::commands::{error_to_close_frame, get_normal_close_frame};

struct ClientStream {
    manager_rx: Pin<Box<dyn tokio_stream::Stream<Item = ServerCommandResponse> + Send>>,
    ws_rx: futures_util::stream::SplitStream<WebSocketStream>
}

enum ClientStreamMessage {
    WebSocketText(String),
    WebSocketPing(Vec<u8>),
    WebSocketClose,
    ManagerData(ServerCommandResponse)
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
        else {
            Poll::Pending
        }
    }
}

pub async fn process_client(addr: SocketAddr, websocket: WebSocketStream, tx: mpsc::Sender<Message>) -> Result<(), Box<dyn Error>> {
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
    let mut client_stream = ClientStream{ manager_rx: mrx_stream, ws_rx };

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
            ClientStreamMessage::WebSocketClose => ws_tx.send(tungstenite::Message::Close(Some(get_normal_close_frame()))).await?
        }
    }

    tx.send(Message::Disconnected(addr)).await?;
    Ok(())
}