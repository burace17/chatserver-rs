SELECT messages.id,user_id,time,nickname,content,url,mime FROM messages
LEFT JOIN message_attachments ON messages.id = message_attachments.message_id
WHERE channel_id = ? ORDER BY time DESC LIMIT ?;