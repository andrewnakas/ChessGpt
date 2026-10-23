-- "Sign in with Chess.com" (OpenID Connect): `chesscom_id` is the id_token `sub`.
ALTER TABLE users ADD COLUMN chesscom_id TEXT;
ALTER TABLE users ADD COLUMN chesscom_username TEXT;
CREATE UNIQUE INDEX users_chesscom ON users(chesscom_id) WHERE chesscom_id IS NOT NULL;
