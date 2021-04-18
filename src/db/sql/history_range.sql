WITH messages_before(id, user_id, time, nickname, content, url, mime) AS (
SELECT messages.id, user_id, time, nickname, content, url, mime FROM messages
LEFT JOIN message_attachments ON messages.id = message_attachments.message_id
WHERE channel_id = ? AND messages.id < ? ORDER BY messages.id DESC LIMIT ?
),
messages_after(id, user_id, time, nickname, content, url, mime) AS (
SELECT messages.id, user_id, time, nickname, content, url, mime FROM messages
LEFT JOIN message_attachments ON messages.id = message_attachments.message_id
WHERE channel_id = ? AND messages.id >= ? LIMIT ?
)
SELECT * FROM messages_before UNION SELECT * FROM messages_after;