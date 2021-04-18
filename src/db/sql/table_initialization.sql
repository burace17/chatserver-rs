CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT UNIQUE NOT NULL,
                                 nickname TEXT UNIQUE NOT NULL, password TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS channels(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL);
CREATE TABLE IF NOT EXISTS user_channels(user_id INTEGER NOT NULL,
                                 channel_id INTEGER NOT NULL,
                                 UNIQUE(user_id, channel_id),
                                 FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
                                 FOREIGN KEY(channel_id) REFERENCES channels(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY AUTOINCREMENT,
                                 user_id INTEGER NOT NULL,
                                 channel_id INTEGER NOT NULL,
                                 time INTEGER NOT NULL,
                                 nickname TEXT NOT NULL,
                                 content TEXT NOT NULL,
                                 FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
                                 FOREIGN KEY(channel_id) REFERENCES channels(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS user_last_read_messages(user_id INTEGER NOT NULL,
                                 channel_id INTEGER NOT NULL,
                                 message_id INTEGER,
                                 UNIQUE(user_id, channel_id, message_id),
                                 FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE,
                                 FOREIGN KEY(user_id, channel_id) REFERENCES user_channels(user_id, channel_id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS message_attachments(id INTEGER PRIMARY KEY AUTOINCREMENT,
                                 message_id INTEGER NOT NULL,
                                 url TEXT NOT NULL,
                                 mime TEXT NOT NULL,
                                 FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE);
