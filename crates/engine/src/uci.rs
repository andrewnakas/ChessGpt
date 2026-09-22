//! Minimal UCI output parser. We parse `info` and `bestmove` ourselves because
//! we need Stockfish's `wdl` and bound fields.

use chess_core::Score;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Exact,
    Lower,
    Upper,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Info {
    pub depth: Option<u32>,
    pub seldepth: Option<u32>,
    pub multipv: Option<u32>,
    /// Side-to-move POV, as UCI reports it.
    pub score: Option<Score>,
    pub bound: Option<Bound>,
    pub wdl: Option<[u32; 3]>,
    pub nodes: Option<u64>,
    pub nps: Option<u64>,
    pub time_ms: Option<u64>,
    pub hashfull: Option<u32>,
    pub pv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UciLine {
    Id { name: String },
    UciOk,
    ReadyOk,
    Info(Info),
    BestMove { best: Option<String>, ponder: Option<String> },
    Other,
}

pub fn parse_line(line: &str) -> UciLine {
    let mut it = line.split_whitespace();
    match it.next() {
        Some("uciok") => UciLine::UciOk,
        Some("readyok") => UciLine::ReadyOk,
        Some("id") => match it.next() {
            Some("name") => UciLine::Id { name: it.collect::<Vec<_>>().join(" ") },
            _ => UciLine::Other,
        },
        Some("bestmove") => {
            let best = it.next().filter(|m| *m != "(none)").map(str::to_string);
            let ponder = match it.next() {
                Some("ponder") => it.next().map(str::to_string),
                _ => None,
            };
            UciLine::BestMove { best, ponder }
        }
        Some("info") => parse_info(it.collect()),
        _ => UciLine::Other,
    }
}

fn parse_info(toks: Vec<&str>) -> UciLine {
    let mut info = Info::default();
    let mut i = 0;
    let num = |i: usize| toks.get(i).and_then(|t| t.parse::<i64>().ok());
    while i < toks.len() {
        match toks[i] {
            "depth" => {
                info.depth = num(i + 1).map(|v| v as u32);
                i += 2;
            }
            "seldepth" => {
                info.seldepth = num(i + 1).map(|v| v as u32);
                i += 2;
            }
            "multipv" => {
                info.multipv = num(i + 1).map(|v| v as u32);
                i += 2;
            }
            "nodes" => {
                info.nodes = num(i + 1).map(|v| v as u64);
                i += 2;
            }
            "nps" => {
                info.nps = num(i + 1).map(|v| v as u64);
                i += 2;
            }
            "time" => {
                info.time_ms = num(i + 1).map(|v| v as u64);
                i += 2;
            }
            "hashfull" => {
                info.hashfull = num(i + 1).map(|v| v as u32);
                i += 2;
            }
            "score" => {
                let kind = toks.get(i + 1).copied();
                let v = num(i + 2).map(|v| v as i32);
                info.score = match (kind, v) {
                    (Some("cp"), Some(v)) => Some(Score::Cp(v)),
                    (Some("mate"), Some(v)) => Some(Score::Mate(v)),
                    _ => None,
                };
                i += 3;
                info.bound = Some(Bound::Exact);
                match toks.get(i).copied() {
                    Some("lowerbound") => {
                        info.bound = Some(Bound::Lower);
                        i += 1;
                    }
                    Some("upperbound") => {
                        info.bound = Some(Bound::Upper);
                        i += 1;
                    }
                    _ => {}
                }
            }
            "wdl" => {
                if let (Some(w), Some(d), Some(l)) = (num(i + 1), num(i + 2), num(i + 3)) {
                    info.wdl = Some([w as u32, d as u32, l as u32]);
                }
                i += 4;
            }
            "pv" => {
                info.pv = toks[i + 1..].iter().map(|s| s.to_string()).collect();
                break;
            }
            "string" => break,
            "currmove" | "currmovenumber" | "tbhits" | "sbhits" | "cpuload" => i += 2,
            _ => i += 1,
        }
    }
    UciLine::Info(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stockfish_19_info() {
        let l = "info depth 12 seldepth 4 multipv 1 score mate 2 wdl 1000 0 0 nodes 964 nps 160666 hashfull 0 tbhits 0 time 6 pv d5f6 g7f6 c4f7";
        let UciLine::Info(i) = parse_line(l) else { panic!() };
        assert_eq!(i.depth, Some(12));
        assert_eq!(i.multipv, Some(1));
        assert_eq!(i.score, Some(Score::Mate(2)));
        assert_eq!(i.bound, Some(Bound::Exact));
        assert_eq!(i.wdl, Some([1000, 0, 0]));
        assert_eq!(i.time_ms, Some(6));
        assert_eq!(i.pv, vec!["d5f6", "g7f6", "c4f7"]);
    }

    #[test]
    fn bounds_and_negative() {
        let UciLine::Info(i) =
            parse_line("info depth 20 seldepth 30 multipv 2 score cp -35 upperbound nodes 1 pv e7e5")
        else {
            panic!()
        };
        assert_eq!(i.score, Some(Score::Cp(-35)));
        assert_eq!(i.bound, Some(Bound::Upper));
        assert_eq!(i.pv, vec!["e7e5"]);
    }

    #[test]
    fn bestmove_variants() {
        assert_eq!(
            parse_line("bestmove d5f6 ponder g7f6"),
            UciLine::BestMove { best: Some("d5f6".into()), ponder: Some("g7f6".into()) }
        );
        assert_eq!(parse_line("bestmove (none)"), UciLine::BestMove { best: None, ponder: None });
        assert_eq!(parse_line("id name Stockfish 19"), UciLine::Id { name: "Stockfish 19".into() });
        assert!(matches!(parse_line("info string NNUE evaluation using nn.nnue"), UciLine::Info(_)));
    }
}
