use futures_util::{StreamExt, SinkExt};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::error::Error;
use tokio::sync::mpsc;
use super::server::{WebSocketStream, Message};

struct ClientStream {
    manager_rx: Pin<Box<dyn tokio_stream::Stream<Item = String> + Send>>,
    ws_rx: futures_util::stream::SplitStream<WebSocketStream>
}

enum ClientStreamMessage {
    WebSocketData(String),
    ManagerData(String)
}

impl tokio_stream::Stream for ClientStream {
    type Item = ClientStreamMessage;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Some(v)) = Pin::new(&mut self.manager_rx).poll_next(cx) {
            Poll::Ready(Some(ClientStreamMessage::ManagerData(v)))
        }
        else if let Poll::Ready(Some(v)) = Pin::new(&mut self.ws_rx).poll_next(cx) {
            match v {
                Ok(msg) => Poll::Ready(Some(ClientStreamMessage::WebSocketData(msg.to_string()))),
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
    let (mtx, mut mrx) = mpsc::channel::<String>(32);
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
            ClientStreamMessage::ManagerData(msg) => {
                println!("Got manager data: {}", msg);
                ws_tx.send(tungstenite::Message::Text(msg)).await?;
            }
            ClientStreamMessage::WebSocketData(data) => {
                println!("Got websocket data: {}", data);
                tx.send(Message::NewData((addr, data))).await?;
            }
        }
    }

    println!("Client exiting...");
    Ok(())
}