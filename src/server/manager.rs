/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::attachments::{start_attachment_manager, AttachmentInfo};
use super::{ChatServer, Sender, ServerCommandResponse};
use crate::commands;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::WatchStream;

type PinnedStream<T> = Pin<Box<dyn tokio_stream::Stream<Item = T> + Send>>;

#[derive(Debug)]
pub enum Message {
    NewConnection((SocketAddr, mpsc::Sender<ServerCommandResponse>)),
    NewData((SocketAddr, String)),
    Disconnected(SocketAddr),
    CheckUnauthUsers,
    GotAttachment(AttachmentInfo),
    ShutdownStatus(bool),
}

struct ManagerStream {
    receiver: PinnedStream<Message>,
    timer: tokio::time::Interval,
    attachment_receiver: PinnedStream<Message>,
    shutdown_receiver: WatchStream<bool>,
}

impl tokio_stream::Stream for ManagerStream {
    type Item = Message;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Some(v)) = Pin::new(&mut self.receiver).poll_next(cx) {
            Poll::Ready(Some(v))
        } else if let Poll::Ready(_) = Pin::new(&mut self.timer).poll_tick(cx) {
            Poll::Ready(Some(Message::CheckUnauthUsers))
        } else if let Poll::Ready(Some(v)) = Pin::new(&mut self.attachment_receiver).poll_next(cx) {
            Poll::Ready(Some(v))
        } else if let Poll::Ready(Some(shutdown)) =
            Pin::new(&mut self.shutdown_receiver).poll_next(cx)
        {
            Poll::Ready(Some(Message::ShutdownStatus(shutdown)))
        } else {
            Poll::Pending
        }
    }
}

pub async fn start(
    mut receiver: mpsc::Receiver<Message>,
    db_path: &str,
    shutdown_receiver: watch::Receiver<bool>,
) {
    let shutdown_clone = shutdown_receiver.clone();
    let shutdown_stream = WatchStream::new(shutdown_receiver);
    let (mut attachment_receiver, url_sender, attachment_task) =
        start_attachment_manager(shutdown_clone);
    let mut server = ChatServer::new(db_path, url_sender).await;

    // Construct a stream that receives data from either the client thread, or a
    // notification from the timer to check for non-responsive unauthenticated users,
    // or an attachment from the attachment manager task.
    let mut worker_stream = {
        let receiver_pin = Box::pin(async_stream::stream! {
            while let Some(item) = receiver.recv().await {
                yield item;
            }
        });
        let attachment_receiver_pin = Box::pin(async_stream::stream! {
            while let Some(item) = attachment_receiver.recv().await {
                yield item;
            }
        });

        ManagerStream {
            receiver: receiver_pin,
            timer: tokio::time::interval(std::time::Duration::from_secs(30)),
            attachment_receiver: attachment_receiver_pin,
            shutdown_receiver: shutdown_stream,
        }
    };

    let mut all_connections: HashMap<SocketAddr, Sender> = HashMap::new();

    while let Some(message) = worker_stream.next().await {
        match message {
            Message::NewConnection((addr, tx)) => {
                println!("New connection from: {}", addr);
                all_connections.insert(addr, tx.clone());
                server.add_unauth_connection(addr, tx);
            }
            Message::NewData((addr, data)) => {
                println!("Received data from {}: {}", addr, data);
                if let Err(e) = commands::process_command(&mut server, addr, &data).await {
                    println!("Disconnecting client due to invalid command: {}", e);
                    if let Err(e2) = all_connections[&addr]
                        .send(ServerCommandResponse::Disconnect(e))
                        .await
                    {
                        println!("Failed to disconnect client?! {}", e2); // should never happen, hopefully.
                    }
                }
            }
            Message::Disconnected(addr) => {
                println!("{} disconnected", addr);
                server.remove_unauth_connection(addr);
                server.remove_connection(addr).await;
                all_connections.remove(&addr);
            }
            Message::CheckUnauthUsers => {
                server.disconnect_unauth_users().await;
            }
            Message::GotAttachment(media_info) => {
                if let Err(e) = server.add_attachment_to_message(&media_info).await {
                    println!("Failed to send ADDATTACHMENT command: {}", e);
                }
            }
            Message::ShutdownStatus(do_it) => {
                if do_it {
                    break;
                }
            }
        }
    }

    attachment_task
        .await
        .expect("Attachment manager task panicked before shutdown.");
}
