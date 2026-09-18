-- The grouped list asks, for every message in a folder, "is this the newest
-- message of its thread in this folder?" With only (thread_id) and
-- (folder_id, date) to choose from, SQLite answered it by walking the folder
-- index once per message — quadratic in folder size, over a minute for a
-- 15k-message inbox, with every other read queued behind it on the one reader
-- connection. Keyed on the thread first, the answer is a single index seek.
--
-- The new index starts with thread_id, so it serves every lookup the old
-- single-column one did; keeping both would only cost an extra write per
-- synced message.
CREATE INDEX idx_messages_thread_folder ON messages(thread_id, folder_id, date);
DROP INDEX idx_messages_thread;
