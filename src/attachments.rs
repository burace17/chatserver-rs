/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use futures_util::StreamExt;
use super::server::Message;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct AttachmentInfo {
    pub channel_id: i64,
    pub message_id: i64,
    pub url: String,
    pub mime: String,
}

impl AttachmentInfo {
    pub fn new(channel_id: i64, message_id: i64, url: &str) -> Self {
        AttachmentInfo {
            channel_id,
            message_id,
            url: url.to_string(),
            mime: "".into()
        }
    }
}

struct UrlStream {
    receiver: Pin<Box<dyn tokio_stream::Stream<Item = AttachmentInfo> + Send>>
}

impl tokio_stream::Stream for UrlStream {
    type Item = AttachmentInfo;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Some(v)) = Pin::new(&mut self.receiver).poll_next(cx) {
            Poll::Ready(Some(v))
        }
        else {
            Poll::Pending
        }
    }
}

struct AttachmentManager {
    url_receiver: UrlStream,
    attachment_sender: mpsc::Sender::<Message>,
}

impl AttachmentManager {
    fn new() -> (Self, mpsc::Receiver::<Message>, mpsc::Sender::<AttachmentInfo>) {
        // make a channel that the server task can use to send us new URLs
        let (url_sender, mut url_receiver) = mpsc::channel::<AttachmentInfo>(32);
        let url_recv_pinned = Box::pin(async_stream::stream! {
            while let Some(url) = url_receiver.recv().await {
                yield url;
            }
        });

        let url_stream = UrlStream{ receiver: url_recv_pinned };

        // make a channel that we can use to send data back to the server
        let (attachment_sender, attachment_receiver) = mpsc::channel::<Message>(32);

        (AttachmentManager {
            url_receiver: url_stream,
            attachment_sender
        }, attachment_receiver, url_sender)
    }

    fn get_content_type(&mut self, res: &reqwest::Response) -> anyhow::Result<String> {
        let headers = res.headers();
        let content_type_data = headers.get(reqwest::header::CONTENT_TYPE).ok_or(anyhow::anyhow!(""))?;
        let content_type = content_type_data.to_str()?;
        Ok(content_type.to_string())
    }

    async fn process_response(&mut self, mut info: AttachmentInfo, res: &reqwest::Response) {
        if let Ok(content_type) = self.get_content_type(res) {
            if content_type.contains("image") {
                info.mime = content_type;
                if let Err(e) = self.attachment_sender.send(Message::GotAttachment(info)).await {
                    println!("Could not send attachment result back to server task: {}", e);
                }
            }
        }
    }

    async fn start(&mut self) {
        let client = reqwest::Client::new();
        while let Some(attachment_request) = self.url_receiver.next().await {
            if let Ok(res) = client.head(&attachment_request.url).send().await {
                self.process_response(attachment_request, &res).await;
            }
        }
    }
}

pub fn start_attachment_manager() -> (mpsc::Receiver::<Message>, mpsc::Sender<AttachmentInfo>) {
    let (mut attachment_manager, attachment_receiver, url_sender) = AttachmentManager::new();
    tokio::spawn(async move {
        attachment_manager.start().await;
    });
    (attachment_receiver, url_sender)
}