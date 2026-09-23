//! Fetch games from Lichess and Chess.com public APIs as raw PGN.

use api_types::GameSource;
use serde::Deserialize;

pub const USER_AGENT: &str = concat!(
    "chessgpt/",
    env!("CARGO_PKG_VERSION"),
    " (+https://chessgpt.com; open-source chess coach)"
);

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("user {0:?} not found")]
    UserNotFound(String),
    #[error("game {0:?} not found")]
    GameNotFound(String),
    #[error("rate limited by {0}; try again in a minute")]
    RateLimited(&'static str),
    #[error("{0}: {1}")]
    Http(&'static str, String),
    #[error("could not read a game id from {0:?}")]
    BadGameId(String),
    #[error(
        "Lichess no longer serves a user's game list without a login (lichess-org/api#667). \
         Add a Lichess personal API token in Settings, or import single games by URL."
    )]
    LichessTokenRequired,
    #[error("Lichess rejected the API token; create a new one at lichess.org/account/oauth/token")]
    LichessBadToken,
}

#[derive(Debug, Clone)]
pub struct ImportedGame {
    pub source: GameSource,
    pub source_id: String,
    pub pgn: String,
}

#[derive(Clone)]
pub struct Importers {
    client: reqwest::Client,
    lichess_base: String,
    chesscom_base: String,
    explorer_base: String,
}

impl Default for Importers {
    fn default() -> Self {
        Self::new("https://lichess.org", "https://api.chess.com")
    }
}

impl Importers {
    pub fn new(lichess_base: &str, chesscom_base: &str) -> Importers {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("http client");
        Importers {
            client,
            lichess_base: lichess_base.trim_end_matches('/').into(),
            chesscom_base: chesscom_base.trim_end_matches('/').into(),
            explorer_base: "https://explorer.lichess.org".into(),
        }
    }

    pub fn with_explorer(mut self, base: &str) -> Importers {
        self.explorer_base = base.trim_end_matches('/').into();
        self
    }

    /// The most recent `max` games of a Lichess user (standard chess only).
    pub async fn lichess_user(
        &self,
        username: &str,
        max: u32,
        token: Option<&str>,
    ) -> Result<Vec<ImportedGame>, ImportError> {
        let username = username.trim();
        let url = format!("{}/api/games/user/{}", self.lichess_base, username);
        let mut req = self.client.get(&url).header("Accept", "application/x-chess-pgn");
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let resp = req
            .query(&[
                ("max", max.clamp(1, 300).to_string()),
                ("clocks", "true".into()),
                ("opening", "true".into()),
                ("perfType", "ultraBullet,bullet,blitz,rapid,classical,correspondence".into()),
            ])
            .send()
            .await
            .map_err(|e| ImportError::Http("lichess", e.to_string()))?;
        let html = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html"));
        match resp.status().as_u16() {
            200 if !html => {}
            // Since Aug 2026 Lichess answers anonymous exports with its HTML
            // 404 page or a redirect to /signup (lichess-org/api#667).
            200 | 303 | 404 if html && token.is_none() => return Err(ImportError::LichessTokenRequired),
            401 | 403 => return Err(ImportError::LichessBadToken),
            404 => return Err(ImportError::UserNotFound(username.into())),
            429 => return Err(ImportError::RateLimited("lichess")),
            s => return Err(ImportError::Http("lichess", format!("HTTP {s}"))),
        }
        let body = resp.text().await.map_err(|e| ImportError::Http("lichess", e.to_string()))?;
        Ok(split_pgns(&body)
            .into_iter()
            .filter_map(|pgn| {
                let id = pgn_tag(&pgn, "Site").and_then(|s| lichess_id(&s))?;
                Some(ImportedGame { source: GameSource::Lichess, source_id: id, pgn })
            })
            .collect())
    }

    /// One Lichess game by id or URL.
    pub async fn lichess_game(&self, id_or_url: &str) -> Result<ImportedGame, ImportError> {
        let id = lichess_id(id_or_url).ok_or_else(|| ImportError::BadGameId(id_or_url.into()))?;
        let url = format!("{}/game/export/{}", self.lichess_base, id);
        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/x-chess-pgn")
            .query(&[("clocks", "true"), ("evals", "true"), ("opening", "true")])
            .send()
            .await
            .map_err(|e| ImportError::Http("lichess", e.to_string()))?;
        match resp.status().as_u16() {
            200 => {}
            404 => return Err(ImportError::GameNotFound(id)),
            429 => return Err(ImportError::RateLimited("lichess")),
            s => return Err(ImportError::Http("lichess", format!("HTTP {s}"))),
        }
        let pgn = resp.text().await.map_err(|e| ImportError::Http("lichess", e.to_string()))?;
        Ok(ImportedGame { source: GameSource::Lichess, source_id: id, pgn })
    }

    /// A Chess.com account's canonical username, or `UserNotFound`.
    pub async fn chesscom_player(&self, username: &str) -> Result<String, ImportError> {
        #[derive(Deserialize)]
        struct Player {
            username: String,
        }
        let username = username.trim().to_lowercase();
        let url = format!("{}/pub/player/{}", self.chesscom_base, username);
        let p: Player = self.chesscom_json(&url, &username).await?;
        Ok(p.username)
    }

    /// The most recent `max` standard games of a Chess.com user. Archives are
    /// fetched one at a time, newest first (Chess.com rejects parallel calls).
    pub async fn chesscom_user(&self, username: &str, max: u32) -> Result<Vec<ImportedGame>, ImportError> {
        let username = username.trim().to_lowercase();
        let url = format!("{}/pub/player/{}/games/archives", self.chesscom_base, username);
        let archives: Archives = self.chesscom_json(&url, &username).await?;
        let max = max.clamp(1, 300) as usize;
        let mut out = Vec::new();
        for month in archives.archives.iter().rev() {
            let month: MonthGames = self.chesscom_json(month, &username).await?;
            let mut games: Vec<ChesscomGame> = month
                .games
                .into_iter()
                .filter(|g| g.rules.as_deref().unwrap_or("chess") == "chess")
                .filter(|g| g.pgn.as_ref().is_some_and(|p| !p.trim().is_empty()))
                .collect();
            games.sort_by_key(|g| std::cmp::Reverse(g.end_time.unwrap_or(0)));
            for g in games {
                out.push(ImportedGame { source: GameSource::Chesscom, source_id: g.url, pgn: g.pgn.unwrap_or_default() });
                if out.len() >= max {
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }

    /// Lichess opening explorer (`masters` or `lichess` database). Lichess
    /// requires a token for the explorer since 2026.
    pub async fn explorer(
        &self,
        fen: &str,
        db: ExplorerDb,
        ratings: Option<&str>,
        token: &str,
    ) -> Result<Explorer, ImportError> {
        let path = match db {
            ExplorerDb::Masters => "masters",
            ExplorerDb::Lichess => "lichess",
        };
        let mut q: Vec<(&str, String)> =
            vec![("fen", fen.to_string()), ("moves", "12".into()), ("topGames", "0".into())];
        if db == ExplorerDb::Lichess {
            q.push(("speeds", "blitz,rapid,classical".into()));
            q.push(("ratings", ratings.unwrap_or("1600,1800,2000,2200,2500").into()));
            q.push(("recentGames", "0".into()));
        }
        let resp = self
            .client
            .get(format!("{}/{path}", self.explorer_base))
            .bearer_auth(token)
            .query(&q)
            .send()
            .await
            .map_err(|e| ImportError::Http("lichess explorer", e.to_string()))?;
        match resp.status().as_u16() {
            200 => {}
            401 | 403 => return Err(ImportError::LichessBadToken),
            429 => return Err(ImportError::RateLimited("lichess explorer")),
            s => return Err(ImportError::Http("lichess explorer", format!("HTTP {s}"))),
        }
        resp.json().await.map_err(|e| ImportError::Http("lichess explorer", e.to_string()))
    }

    async fn chesscom_json<T: for<'de> Deserialize<'de>>(&self, url: &str, user: &str) -> Result<T, ImportError> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| ImportError::Http("chess.com", e.to_string()))?;
        match resp.status().as_u16() {
            200 => {}
            404 | 410 => return Err(ImportError::UserNotFound(user.into())),
            429 => return Err(ImportError::RateLimited("chess.com")),
            s => return Err(ImportError::Http("chess.com", format!("HTTP {s}"))),
        }
        resp.json().await.map_err(|e| ImportError::Http("chess.com", e.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExplorerDb {
    Masters,
    Lichess,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct ExplorerMove {
    pub uci: String,
    pub san: String,
    pub white: u64,
    pub draws: u64,
    pub black: u64,
    #[serde(rename = "averageRating")]
    pub average_rating: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct ExplorerOpening {
    pub eco: String,
    pub name: String,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct Explorer {
    pub white: u64,
    pub draws: u64,
    pub black: u64,
    pub moves: Vec<ExplorerMove>,
    pub opening: Option<ExplorerOpening>,
}

#[derive(Deserialize)]
struct Archives {
    archives: Vec<String>,
}

#[derive(Deserialize)]
struct MonthGames {
    games: Vec<ChesscomGame>,
}

#[derive(Deserialize)]
struct ChesscomGame {
    url: String,
    pgn: Option<String>,
    rules: Option<String>,
    end_time: Option<i64>,
}

/// Lichess game ids are 8 characters; URLs may carry a 12-char player id,
/// `/black`, or an anchor.
pub fn lichess_id(s: &str) -> Option<String> {
    let s = s.trim();
    let path = s
        .split("lichess.org/")
        .nth(1)
        .unwrap_or(s)
        .split(['#', '?'])
        .next()
        .unwrap_or("");
    let first = path.split('/').find(|p| !p.is_empty())?;
    let first = if first == "game" || first == "embed" {
        path.split('/').filter(|p| !p.is_empty()).nth(if first == "embed" { 2 } else { 2 })?
    } else {
        first
    };
    if first.len() < 8 || !first.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(first[..8].to_string())
}

/// Split a multi-game PGN on blank lines that precede a tag section.
pub fn split_pgns(body: &str) -> Vec<String> {
    let body = body.replace("\r\n", "\n");
    let mut games = Vec::new();
    let mut cur = String::new();
    let mut in_moves = false;
    for line in body.lines() {
        let is_tag = line.starts_with('[');
        if is_tag && in_moves {
            if !cur.trim().is_empty() {
                games.push(cur.trim().to_string());
            }
            cur.clear();
            in_moves = false;
        }
        if !is_tag && !line.trim().is_empty() {
            in_moves = true;
        }
        cur.push_str(line);
        cur.push('\n');
    }
    if !cur.trim().is_empty() {
        games.push(cur.trim().to_string());
    }
    games
}

pub fn pgn_tag(pgn: &str, name: &str) -> Option<String> {
    let prefix = format!("[{name} \"");
    pgn.lines()
        .find(|l| l.starts_with(&prefix))
        .and_then(|l| l[prefix.len()..].strip_suffix("\"]").map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn lichess_ids() {
        assert_eq!(lichess_id("https://lichess.org/abcdEFGH").as_deref(), Some("abcdEFGH"));
        assert_eq!(lichess_id("https://lichess.org/abcdEFGHijkl").as_deref(), Some("abcdEFGH"));
        assert_eq!(lichess_id("lichess.org/abcdEFGH/black#12").as_deref(), Some("abcdEFGH"));
        assert_eq!(lichess_id("abcdEFGH").as_deref(), Some("abcdEFGH"));
        assert_eq!(lichess_id("https://lichess.org/game/export/abcdEFGH").as_deref(), Some("abcdEFGH"));
        assert_eq!(lichess_id("nope"), None);
    }

    #[test]
    fn splits_multi_game_pgn() {
        let body = "[Event \"a\"]\n[Site \"https://lichess.org/aaaaaaaa\"]\n\n1. e4 e5 1-0\n\n\n[Event \"b\"]\n[Site \"https://lichess.org/bbbbbbbb\"]\n\n1. d4 d5\n2. c4 0-1\n";
        let g = split_pgns(body);
        assert_eq!(g.len(), 2);
        assert!(g[1].contains("2. c4"));
        assert_eq!(pgn_tag(&g[0], "Site").as_deref(), Some("https://lichess.org/aaaaaaaa"));
    }

    #[tokio::test]
    async fn lichess_user_import() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/games/user/magnus"))
            .and(header("Accept", "application/x-chess-pgn"))
            .and(query_param("max", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "[Site \"https://lichess.org/aaaaaaaa\"]\n\n1. e4 e5 *\n\n[Site \"https://lichess.org/bbbbbbbb\"]\n\n1. d4 *\n",
            ))
            .mount(&server)
            .await;
        let imp = Importers::new(&server.uri(), &server.uri());
        let games = imp.lichess_user("magnus", 2, None).await.unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(games[1].source_id, "bbbbbbbb");
        let err = imp.lichess_user("nobody", 2, Some("tok")).await.unwrap_err();
        assert!(matches!(err, ImportError::Http(..) | ImportError::UserNotFound(_)));
    }

    #[tokio::test]
    async fn lichess_html_404_means_token_required() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/games/user/someone"))
            .respond_with(
                ResponseTemplate::new(404).set_body_raw("<html>", "text/html; charset=utf-8"),
            )
            .mount(&server)
            .await;
        let imp = Importers::new(&server.uri(), &server.uri());
        let err = imp.lichess_user("someone", 5, None).await.unwrap_err();
        assert!(matches!(err, ImportError::LichessTokenRequired), "{err}");
    }

    #[tokio::test]
    async fn explorer_sends_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/masters"))
            .and(header("Authorization", "Bearer lip_x"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "white": 10, "draws": 20, "black": 5,
                "moves": [{"uci": "e7e5", "san": "e5", "white": 5, "draws": 10, "black": 2, "averageRating": 2600}],
                "opening": {"eco": "B00", "name": "King's Pawn Game"}
            })))
            .mount(&server)
            .await;
        let imp = Importers::new(&server.uri(), &server.uri()).with_explorer(&server.uri());
        let e = imp.explorer("fen", ExplorerDb::Masters, None, "lip_x").await.unwrap();
        assert_eq!(e.moves[0].san, "e5");
        assert_eq!(e.opening.unwrap().eco, "B00");
    }

    #[tokio::test]
    async fn chesscom_newest_first_and_variants_skipped() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(path("/pub/player/hikaru/games/archives"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "archives": [format!("{base}/pub/player/hikaru/games/2026/08"), format!("{base}/pub/player/hikaru/games/2026/09")]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/pub/player/hikaru/games/2026/09"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"games": [
                {"url": "https://www.chess.com/game/live/1", "pgn": "1. e4 *", "rules": "chess", "end_time": 1},
                {"url": "https://www.chess.com/game/live/2", "pgn": "1. d4 *", "rules": "chess", "end_time": 2},
                {"url": "https://www.chess.com/game/live/3", "pgn": "1. e4 *", "rules": "chess960", "end_time": 3}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/pub/player/hikaru/games/2026/08"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"games": [
                {"url": "https://www.chess.com/game/live/0", "pgn": "1. c4 *", "rules": "chess", "end_time": 0}
            ]})))
            .mount(&server)
            .await;
        let imp = Importers::new(&base, &base);
        let games = imp.chesscom_user("Hikaru", 3).await.unwrap();
        let ids: Vec<&str> = games.iter().map(|g| g.source_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["https://www.chess.com/game/live/2", "https://www.chess.com/game/live/1", "https://www.chess.com/game/live/0"]
        );
    }
}
