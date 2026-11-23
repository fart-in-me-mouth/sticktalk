//! The built-in games' rules, run by the server. Players send moves
//! (`ActivitySignal::Action`); only the server writes the game (`s/game`), so a
//! tampered client cannot move out of turn, make an illegal move or see the
//! word it has to guess. Everyone else in the session watches.
use serde_json::{json, Value as Json};

#[derive(Default)]
pub(super) struct Effects {
    /// Keys the server writes (None deletes).
    pub writes: Vec<(String, Option<String>)>,
    /// Private messages: (participant, JSON text).
    pub private: Vec<(u16, String)>,
    /// What the session's summary keeps.
    pub result: Option<String>,
    /// Delete every key with this prefix first.
    pub clear: Option<String>,
}

pub(super) type Outcome = std::result::Result<Effects, &'static str>;

pub(super) trait Engine: Send + Sync {
    fn joined(&mut self, pid: u16, name: &str) -> Effects;
    fn left(&mut self, pid: u16) -> Effects;
    fn action(&mut self, pid: u16, name: &str, data: &Json) -> Outcome;
    /// A key a participant may write directly (the drawer's strokes).
    fn may_write(&self, _pid: u16, _key: &str) -> bool {
        false
    }
}

pub(super) fn engine(id: &str) -> Option<Box<dyn Engine>> {
    match id {
        "four-in-a-row" => Some(Box::new(Table::new(FourInARow::default()))),
        "checkers" => Some(Box::new(Table::new(Checkers::new()))),
        "sketch-guess" => Some(Box::new(Sketch::default())),
        _ => None,
    }
}

fn kind(data: &Json) -> &str {
    data.get("type").and_then(Json::as_str).unwrap_or_default()
}

// ---- two-seat board games ------------------------------------------------------

/// A two-player board game's own rules.
pub(super) trait Board: Send + Sync {
    fn reset(&mut self);
    /// Apply a move by `seat` (0 or 1). Ok(Some(winner seat or None for a draw)) ends the game.
    fn play(
        &mut self,
        seat: usize,
        data: &Json,
    ) -> std::result::Result<Option<Option<usize>>, &'static str>;
    /// Whose turn (0 or 1).
    fn turn(&self) -> usize;
    fn set_first(&mut self, seat: usize);
    fn state(&self) -> Json;
    fn title(&self) -> &'static str;
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Waiting,
    Playing,
    Over,
}

pub(super) struct Table<B: Board> {
    board: B,
    seats: [Option<(u16, String)>; 2],
    status: Status,
    winner: Option<usize>,
    first: usize,
    rematch: [bool; 2],
    games: u32,
}

impl<B: Board> Table<B> {
    fn new(board: B) -> Self {
        Self {
            board,
            seats: [None, None],
            status: Status::Waiting,
            winner: None,
            first: 0,
            rematch: [false; 2],
            games: 0,
        }
    }
    fn seat_of(&self, pid: u16) -> Option<usize> {
        self.seats
            .iter()
            .position(|s| s.as_ref().is_some_and(|(p, _)| *p == pid))
    }
    fn publish(&self, result: Option<String>) -> Effects {
        let seats: Vec<Json> = self
            .seats
            .iter()
            .map(|s| match s {
                Some((pid, name)) => json!({ "pid": pid, "name": name }),
                None => Json::Null,
            })
            .collect();
        let status = match self.status {
            Status::Waiting => "waiting",
            Status::Playing => "playing",
            Status::Over => "over",
        };
        let v = json!({
            "seats": seats,
            "status": status,
            "turn": self.board.turn(),
            "winner": self.winner,
            "rematch": self.rematch,
            "games": self.games,
            "board": self.board.state(),
        });
        Effects {
            writes: vec![("s/game".into(), Some(v.to_string()))],
            result,
            ..Default::default()
        }
    }
    fn start(&mut self) {
        self.board.reset();
        self.board.set_first(self.first);
        self.status = Status::Playing;
        self.winner = None;
        self.rematch = [false; 2];
    }
    fn finish(&mut self, winner: Option<usize>) -> Option<String> {
        self.status = Status::Over;
        self.winner = winner;
        self.games += 1;
        let name = |s: usize| {
            self.seats[s]
                .as_ref()
                .map_or("Someone".into(), |(_, n)| n.clone())
        };
        Some(match winner {
            Some(w) => format!("{} beat {} at {}", name(w), name(1 - w), self.board.title()),
            None => format!("{} and {} drew at {}", name(0), name(1), self.board.title()),
        })
    }
    fn stand(&mut self, seat: usize) -> Option<String> {
        let result = if self.status == Status::Playing {
            // Leaving a game in progress forfeits it.
            self.finish(Some(1 - seat))
        } else {
            None
        };
        self.seats[seat] = None;
        if self.status != Status::Playing {
            self.status = if self.status == Status::Over {
                Status::Over
            } else {
                Status::Waiting
            };
        }
        self.rematch = [false; 2];
        result
    }
}

impl<B: Board> Engine for Table<B> {
    fn joined(&mut self, _pid: u16, _name: &str) -> Effects {
        self.publish(None)
    }
    fn left(&mut self, pid: u16) -> Effects {
        match self.seat_of(pid) {
            Some(seat) => {
                let result = self.stand(seat);
                self.publish(result)
            }
            None => Effects::default(),
        }
    }
    fn action(&mut self, pid: u16, name: &str, data: &Json) -> Outcome {
        match kind(data) {
            "sit" => {
                let seat = data
                    .get("seat")
                    .and_then(Json::as_u64)
                    .ok_or("Choose a seat.")? as usize;
                if seat > 1 || self.seats[seat].is_some() {
                    return Err("That seat is taken.");
                }
                if self.seat_of(pid).is_some() {
                    return Err("You already have a seat.");
                }
                if self.status == Status::Playing {
                    return Err("Wait for this game to end.");
                }
                self.seats[seat] = Some((pid, name.to_owned()));
                if self.seats.iter().all(Option::is_some) && self.status == Status::Waiting {
                    self.start();
                }
                Ok(self.publish(None))
            }
            "stand" => {
                let seat = self.seat_of(pid).ok_or("You are watching.")?;
                let result = self.stand(seat);
                Ok(self.publish(result))
            }
            "resign" => {
                let seat = self.seat_of(pid).ok_or("You are watching.")?;
                if self.status != Status::Playing {
                    return Err("No game is in progress.");
                }
                let result = self.finish(Some(1 - seat));
                Ok(self.publish(result))
            }
            "move" => {
                let seat = self.seat_of(pid).ok_or("You are watching.")?;
                if self.status != Status::Playing {
                    return Err("No game is in progress.");
                }
                if self.board.turn() != seat {
                    return Err("It isn't your turn.");
                }
                let result = match self.board.play(seat, data)? {
                    Some(winner) => self.finish(winner),
                    None => None,
                };
                Ok(self.publish(result))
            }
            "rematch" => {
                let seat = self.seat_of(pid).ok_or("You are watching.")?;
                if self.status != Status::Over || self.seats.iter().any(Option::is_none) {
                    return Err("A rematch needs both players after a game.");
                }
                self.rematch[seat] = true;
                if self.rematch == [true, true] {
                    // The other player begins the next game.
                    self.first = 1 - self.first;
                    self.start();
                }
                Ok(self.publish(None))
            }
            _ => Err("Unknown move."),
        }
    }
}

/// Four in a Row: seven columns, six rows; line up four.
#[derive(Default)]
pub(super) struct FourInARow {
    /// Row-major from the top: 0 empty, 1 first seat, 2 second seat.
    cells: Vec<u8>,
    turn: usize,
    last: Option<usize>,
}

impl FourInARow {
    const W: usize = 7;
    const H: usize = 6;
    fn at(&self, r: i32, c: i32) -> u8 {
        if (0..Self::H as i32).contains(&r) && (0..Self::W as i32).contains(&c) {
            self.cells[r as usize * Self::W + c as usize]
        } else {
            0
        }
    }
    fn wins(&self, i: usize) -> bool {
        let (r, c) = ((i / Self::W) as i32, (i % Self::W) as i32);
        let me = self.cells[i];
        [(0, 1), (1, 0), (1, 1), (1, -1)].iter().any(|(dr, dc)| {
            let mut n = 1;
            for s in [1, -1] {
                let mut k = 1;
                while self.at(r + dr * k * s, c + dc * k * s) == me {
                    n += 1;
                    k += 1;
                }
            }
            n >= 4
        })
    }
}

impl Board for FourInARow {
    fn reset(&mut self) {
        self.cells = vec![0; Self::W * Self::H];
        self.last = None;
    }
    fn set_first(&mut self, seat: usize) {
        self.turn = seat;
    }
    fn turn(&self) -> usize {
        self.turn
    }
    fn title(&self) -> &'static str {
        "Four in a Row"
    }
    fn play(
        &mut self,
        seat: usize,
        data: &Json,
    ) -> std::result::Result<Option<Option<usize>>, &'static str> {
        let col = data
            .get("col")
            .and_then(Json::as_u64)
            .ok_or("Choose a column.")? as usize;
        if col >= Self::W {
            return Err("Choose a column.");
        }
        let row = (0..Self::H)
            .rev()
            .find(|r| self.cells[r * Self::W + col] == 0)
            .ok_or("That column is full.")?;
        let i = row * Self::W + col;
        self.cells[i] = seat as u8 + 1;
        self.last = Some(i);
        if self.wins(i) {
            return Ok(Some(Some(seat)));
        }
        if self.cells.iter().all(|c| *c != 0) {
            return Ok(Some(None));
        }
        self.turn = 1 - seat;
        Ok(None)
    }
    fn state(&self) -> Json {
        json!({ "cells": self.cells, "last": self.last, "width": Self::W, "height": Self::H })
    }
}

/// Checkers (English draughts): forced captures, multi-jumps, kings.
pub(super) struct Checkers {
    /// 0 empty; 1 first seat's man, 2 its king; 3 second seat's man, 4 its king.
    cells: [u8; 64],
    turn: usize,
    /// A capture sequence in progress: this piece must keep jumping.
    chain: Option<usize>,
    /// Moves by both sides since the last capture or crowning.
    quiet: u32,
    last: Option<(usize, usize)>,
}

impl Checkers {
    fn new() -> Self {
        let mut c = Self {
            cells: [0; 64],
            turn: 0,
            chain: None,
            quiet: 0,
            last: None,
        };
        c.reset();
        c
    }
    fn owner(p: u8) -> Option<usize> {
        match p {
            1 | 2 => Some(0),
            3 | 4 => Some(1),
            _ => None,
        }
    }
    fn king(p: u8) -> bool {
        p == 2 || p == 4
    }
    /// Row directions a piece may move in: the first seat moves up the board.
    fn dirs(p: u8) -> &'static [i32] {
        match p {
            1 => &[-1],
            3 => &[1],
            _ => &[-1, 1],
        }
    }
    fn step(i: usize, dr: i32, dc: i32, k: i32) -> Option<usize> {
        let (r, c) = ((i / 8) as i32 + dr * k, (i % 8) as i32 + dc * k);
        ((0..8).contains(&r) && (0..8).contains(&c)).then_some((r * 8 + c) as usize)
    }
    fn jumps(&self, i: usize) -> Vec<(usize, usize)> {
        let p = self.cells[i];
        let Some(me) = Self::owner(p) else {
            return vec![];
        };
        let mut out = vec![];
        for &dr in Self::dirs(p) {
            for dc in [-1, 1] {
                if let (Some(over), Some(to)) = (Self::step(i, dr, dc, 1), Self::step(i, dr, dc, 2))
                {
                    if Self::owner(self.cells[over]) == Some(1 - me) && self.cells[to] == 0 {
                        out.push((over, to));
                    }
                }
            }
        }
        out
    }
    fn steps(&self, i: usize) -> Vec<usize> {
        let p = self.cells[i];
        let mut out = vec![];
        for &dr in Self::dirs(p) {
            for dc in [-1, 1] {
                if let Some(to) = Self::step(i, dr, dc, 1) {
                    if self.cells[to] == 0 {
                        out.push(to);
                    }
                }
            }
        }
        out
    }
    fn must_jump(&self, seat: usize) -> bool {
        (0..64).any(|i| Self::owner(self.cells[i]) == Some(seat) && !self.jumps(i).is_empty())
    }
    fn can_move(&self, seat: usize) -> bool {
        (0..64).any(|i| {
            Self::owner(self.cells[i]) == Some(seat)
                && (!self.jumps(i).is_empty() || !self.steps(i).is_empty())
        })
    }
}

impl Board for Checkers {
    fn reset(&mut self) {
        self.cells = [0; 64];
        for i in 0..64 {
            let (r, c) = (i / 8, i % 8);
            if (r + c) % 2 == 1 {
                if r < 3 {
                    self.cells[i] = 3;
                } else if r > 4 {
                    self.cells[i] = 1;
                }
            }
        }
        self.chain = None;
        self.quiet = 0;
        self.last = None;
    }
    fn set_first(&mut self, seat: usize) {
        self.turn = seat;
    }
    fn turn(&self) -> usize {
        self.turn
    }
    fn title(&self) -> &'static str {
        "Checkers"
    }
    fn play(
        &mut self,
        seat: usize,
        data: &Json,
    ) -> std::result::Result<Option<Option<usize>>, &'static str> {
        let from = data
            .get("from")
            .and_then(Json::as_u64)
            .ok_or("Choose a piece.")? as usize;
        let to = data
            .get("to")
            .and_then(Json::as_u64)
            .ok_or("Choose a square.")? as usize;
        if from >= 64 || to >= 64 || Self::owner(self.cells[from]) != Some(seat) {
            return Err("That is not your piece.");
        }
        if self.chain.is_some_and(|c| c != from) {
            return Err("Keep jumping with the same piece.");
        }
        let jumps = self.jumps(from);
        let captured = if let Some((over, _)) = jumps.iter().find(|(_, t)| *t == to) {
            Some(*over)
        } else if self.must_jump(seat) || self.chain.is_some() {
            return Err("You must capture.");
        } else if self.steps(from).contains(&to) {
            None
        } else {
            return Err("That move is not allowed.");
        };
        let piece = self.cells[from];
        self.cells[from] = 0;
        self.cells[to] = piece;
        self.last = Some((from, to));
        if let Some(over) = captured {
            self.cells[over] = 0;
        }
        let crowned = !Self::king(piece) && ((piece == 1 && to < 8) || (piece == 3 && to >= 56));
        if crowned {
            self.cells[to] = piece + 1;
        }
        if captured.is_some() || crowned {
            self.quiet = 0;
        } else {
            self.quiet += 1;
        }
        // Crowning ends the turn; otherwise a capture continues while it can.
        if captured.is_some() && !crowned && !self.jumps(to).is_empty() {
            self.chain = Some(to);
            return Ok(None);
        }
        self.chain = None;
        let other = 1 - seat;
        if !self.can_move(other) {
            return Ok(Some(Some(seat)));
        }
        if self.quiet >= 80 {
            return Ok(Some(None));
        }
        self.turn = other;
        Ok(None)
    }
    fn state(&self) -> Json {
        json!({ "cells": self.cells.to_vec(), "chain": self.chain, "last": self.last, "quiet": self.quiet })
    }
}

// ---- Sketch & Guess --------------------------------------------------------------

/// Plain, public-domain words to draw.
const WORDS: &[&str] = &[
    "apple",
    "anchor",
    "airplane",
    "balloon",
    "banana",
    "basket",
    "bicycle",
    "bridge",
    "broom",
    "butterfly",
    "cactus",
    "camera",
    "candle",
    "carrot",
    "castle",
    "cat",
    "chair",
    "cloud",
    "clock",
    "comb",
    "compass",
    "cookie",
    "crown",
    "cup",
    "dog",
    "dolphin",
    "door",
    "dragon",
    "drum",
    "duck",
    "eagle",
    "egg",
    "elephant",
    "envelope",
    "feather",
    "fence",
    "fish",
    "flag",
    "flower",
    "fork",
    "frog",
    "ghost",
    "giraffe",
    "glasses",
    "guitar",
    "hammer",
    "hat",
    "heart",
    "helicopter",
    "horse",
    "house",
    "iceberg",
    "island",
    "jellyfish",
    "kangaroo",
    "key",
    "kite",
    "ladder",
    "lamp",
    "leaf",
    "lemon",
    "lighthouse",
    "lion",
    "lock",
    "map",
    "mermaid",
    "moon",
    "mountain",
    "mouse",
    "mushroom",
    "nest",
    "octopus",
    "owl",
    "paintbrush",
    "palm tree",
    "parachute",
    "pencil",
    "penguin",
    "piano",
    "pizza",
    "planet",
    "pumpkin",
    "rabbit",
    "rainbow",
    "robot",
    "rocket",
    "sailboat",
    "sandwich",
    "scissors",
    "shark",
    "sheep",
    "shoe",
    "snail",
    "snake",
    "snowman",
    "spider",
    "spoon",
    "star",
    "sun",
    "sunflower",
    "sword",
    "teapot",
    "tent",
    "tiger",
    "toothbrush",
    "tractor",
    "train",
    "tree",
    "trophy",
    "turtle",
    "umbrella",
    "unicorn",
    "violin",
    "volcano",
    "watermelon",
    "whale",
    "windmill",
    "wizard",
    "zebra",
];

const MAX_CHAT: usize = 30;

#[derive(Default)]
pub(super) struct Sketch {
    status: &'static str,
    names: Vec<(u16, String)>,
    order: Vec<u16>,
    scores: Vec<(u16, u32)>,
    round: usize,
    rounds: usize,
    drawer: Option<u16>,
    word: String,
    guessed: Vec<u16>,
    chat: std::collections::VecDeque<Json>,
    used: Vec<usize>,
}

fn normal(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn close(a: &str, b: &str) -> bool {
    // One letter wrong, missing or extra (for words of five letters or more).
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if b.len() < 5 || a.len().abs_diff(b.len()) > 1 || a == b {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        match a.len().cmp(&b.len()) {
            std::cmp::Ordering::Greater => i += 1,
            std::cmp::Ordering::Less => j += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    edits + (a.len() - i) + (b.len() - j) <= 1
}

impl Sketch {
    fn name(&self, pid: u16) -> String {
        self.names
            .iter()
            .find(|(p, _)| *p == pid)
            .map_or_else(|| "Someone".into(), |(_, n)| n.clone())
    }
    fn add_score(&mut self, pid: u16, n: u32) {
        match self.scores.iter_mut().find(|(p, _)| *p == pid) {
            Some(s) => s.1 += n,
            None => self.scores.push((pid, n)),
        }
    }
    fn say(&mut self, who: String, text: String, ok: bool) {
        if self.chat.len() >= MAX_CHAT {
            self.chat.pop_front();
        }
        self.chat
            .push_back(json!({ "who": who, "text": text, "ok": ok }));
    }
    fn hint(&self) -> String {
        self.word
            .chars()
            .map(|c| if c == ' ' { ' ' } else { '_' })
            .collect()
    }
    fn publish(&self, mut fx: Effects) -> Effects {
        let reveal = matches!(self.status, "reveal" | "over");
        let v = json!({
            "status": if self.status.is_empty() { "lobby" } else { self.status },
            "round": self.round,
            "rounds": self.rounds,
            "drawer": self.drawer,
            "hint": if self.status == "drawing" { self.hint() } else { String::new() },
            "word": if reveal { self.word.clone() } else { String::new() },
            "guessed": self.guessed,
            "scores": self.scores.iter().map(|(p, s)| json!({ "pid": p, "name": self.name(*p), "score": s })).collect::<Vec<_>>(),
            "chat": self.chat,
        });
        fx.writes.push(("s/game".into(), Some(v.to_string())));
        fx
    }
    fn pick(&mut self) -> String {
        if self.used.len() >= WORDS.len() {
            self.used.clear();
        }
        loop {
            let b = crate::state::random(2);
            let i = (usize::from(b[0]) << 8 | usize::from(b[1])) % WORDS.len();
            if !self.used.contains(&i) {
                self.used.push(i);
                return WORDS[i].to_owned();
            }
        }
    }
    /// The next drawer's round, or the end of the game.
    fn next_round(&mut self) -> Effects {
        let mut fx = Effects {
            clear: Some("d/".into()),
            ..Default::default()
        };
        self.guessed.clear();
        if self.round >= self.rounds || self.order.is_empty() {
            self.status = "over";
            self.drawer = None;
            let best = self.scores.iter().max_by_key(|(_, s)| *s).copied();
            fx.result = Some(match best {
                Some((p, s)) if s > 0 => {
                    format!("{} won Sketch & Guess with {s} points", self.name(p))
                }
                _ => "Sketch & Guess ended with no winner".into(),
            });
            return self.publish(fx);
        }
        let drawer = self.order[self.round % self.order.len()];
        self.round += 1;
        self.drawer = Some(drawer);
        self.word = self.pick();
        self.status = "drawing";
        fx.private.push((
            drawer,
            json!({ "type": "word", "word": self.word }).to_string(),
        ));
        self.publish(fx)
    }
    fn reveal(&mut self) -> Effects {
        self.status = "reveal";
        let word = self.word.clone();
        self.say(String::new(), format!("The word was “{word}”."), true);
        self.publish(Effects::default())
    }
    fn guessers(&self) -> usize {
        self.names
            .iter()
            .filter(|(p, _)| Some(*p) != self.drawer)
            .count()
    }
}

impl Engine for Sketch {
    fn joined(&mut self, pid: u16, name: &str) -> Effects {
        self.names.push((pid, name.to_owned()));
        if self.status == "drawing" || self.status == "reveal" {
            self.order.push(pid);
        }
        self.publish(Effects::default())
    }
    fn left(&mut self, pid: u16) -> Effects {
        self.names.retain(|(p, _)| *p != pid);
        self.order.retain(|p| *p != pid);
        if self.drawer == Some(pid) && self.status == "drawing" {
            return self.reveal();
        }
        if self.status == "drawing" && self.guessed.len() >= self.guessers() && self.guessers() > 0
        {
            return self.reveal();
        }
        self.publish(Effects::default())
    }
    fn may_write(&self, pid: u16, key: &str) -> bool {
        self.status == "drawing" && self.drawer == Some(pid) && key.starts_with("d/")
    }
    fn action(&mut self, pid: u16, _name: &str, data: &Json) -> Outcome {
        match kind(data) {
            "start" => {
                if matches!(self.status, "drawing" | "reveal") {
                    return Err("A game is in progress.");
                }
                if self.names.len() < 2 {
                    return Err("Sketch & Guess needs at least two people.");
                }
                self.order = self.names.iter().map(|(p, _)| *p).collect();
                self.scores = self.order.iter().map(|p| (*p, 0)).collect();
                self.rounds = self.order.len();
                self.round = 0;
                self.chat.clear();
                Ok(self.next_round())
            }
            "guess" => {
                if self.status != "drawing" {
                    return Err("Nothing to guess right now.");
                }
                if self.drawer == Some(pid) {
                    return Err("You are drawing.");
                }
                if self.guessed.contains(&pid) {
                    return Err("You already guessed it.");
                }
                let text = data.get("text").and_then(Json::as_str).unwrap_or_default();
                let text: String = text.trim().chars().take(60).collect();
                if text.is_empty() || text.chars().any(char::is_control) {
                    return Err("Type a guess.");
                }
                let who = self.name(pid);
                if normal(&text) == self.word {
                    let n = self.guessed.len() as u32;
                    self.guessed.push(pid);
                    self.add_score(pid, 10u32.saturating_sub(2 * n).max(2));
                    if let Some(d) = self.drawer {
                        self.add_score(d, 2);
                    }
                    self.say(who.clone(), format!("{who} guessed it!"), true);
                    if self.guessed.len() >= self.guessers() {
                        return Ok(self.reveal());
                    }
                    return Ok(self.publish(Effects::default()));
                }
                let mut fx = Effects::default();
                if close(&normal(&text), &self.word) {
                    fx.private
                        .push((pid, json!({ "type": "close" }).to_string()));
                }
                self.say(who, text, false);
                Ok(self.publish(fx))
            }
            "skip" => {
                if self.status != "drawing" || self.drawer != Some(pid) {
                    return Err("Only the person drawing can skip.");
                }
                Ok(self.reveal())
            }
            "next" => {
                if self.status != "reveal" {
                    return Err("The round isn't over.");
                }
                Ok(self.next_round())
            }
            _ => Err("Unknown move."),
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    fn act(e: &mut dyn Engine, pid: u16, v: Json) -> Outcome {
        e.action(pid, &format!("p{pid}"), &v)
    }

    #[test]
    fn four_in_a_row_enforces_turns_and_finds_a_win() {
        let mut t = Table::new(FourInARow::default());
        t.joined(1, "Ana");
        t.joined(2, "Ben");
        assert!(
            act(&mut t, 1, json!({"type":"move","col":0})).is_err(),
            "nobody seated yet"
        );
        act(&mut t, 1, json!({"type":"sit","seat":0})).unwrap();
        assert!(
            act(&mut t, 2, json!({"type":"sit","seat":0})).is_err(),
            "seat taken"
        );
        act(&mut t, 2, json!({"type":"sit","seat":1})).unwrap();
        assert!(
            act(&mut t, 2, json!({"type":"move","col":0})).is_err(),
            "not Ben's turn"
        );
        assert!(
            act(&mut t, 3, json!({"type":"move","col":0})).is_err(),
            "spectators cannot move"
        );
        for _ in 0..3 {
            act(&mut t, 1, json!({"type":"move","col":0})).unwrap();
            act(&mut t, 2, json!({"type":"move","col":1})).unwrap();
        }
        let fx = act(&mut t, 1, json!({"type":"move","col":0})).unwrap();
        assert_eq!(fx.result.as_deref(), Some("p1 beat p2 at Four in a Row"));
        assert!(
            act(&mut t, 2, json!({"type":"move","col":1})).is_err(),
            "game over"
        );
        act(&mut t, 1, json!({"type":"rematch"})).unwrap();
        act(&mut t, 2, json!({"type":"rematch"})).unwrap();
        // Ben starts the rematch.
        assert!(act(&mut t, 1, json!({"type":"move","col":3})).is_err());
        act(&mut t, 2, json!({"type":"move","col":3})).unwrap();
    }

    #[test]
    fn a_full_column_and_bad_columns_are_refused() {
        let mut b = FourInARow::default();
        b.reset();
        for i in 0..6 {
            b.play(i % 2, &json!({"col": 2})).unwrap();
        }
        assert!(b.play(0, &json!({"col": 2})).is_err());
        assert!(b.play(0, &json!({"col": 7})).is_err());
        assert!(b.play(0, &json!({})).is_err());
    }

    #[test]
    fn checkers_forces_captures_and_continues_jumps() {
        let mut b = Checkers::new();
        b.cells = [0; 64];
        // First seat's man at row 5 col 2 (42); opponents at 33 and 17.
        b.cells[42] = 1;
        b.cells[33] = 3;
        b.cells[17] = 3;
        b.cells[49] = 3; // keeps the opponent able to move
        b.turn = 0;
        // A plain step is refused while a capture exists.
        assert_eq!(
            b.play(0, &json!({"from":42,"to":35})),
            Err("You must capture.")
        );
        assert_eq!(b.play(0, &json!({"from":42,"to":24})), Ok(None));
        assert_eq!(b.chain, Some(24));
        assert_eq!(b.cells[33], 0);
        // The same piece must keep jumping.
        assert_eq!(b.play(0, &json!({"from":24,"to":10})), Ok(None));
        assert_eq!(b.cells[17], 0);
        assert_eq!(b.turn, 1);
        // Only the second seat's pieces may move now, and only forward.
        assert!(b.play(1, &json!({"from":49,"to":40})).is_err());
        assert_eq!(b.play(1, &json!({"from":49,"to":58})), Ok(None));
    }

    #[test]
    fn checkers_crowns_and_detects_a_win() {
        let mut b = Checkers::new();
        b.cells = [0; 64];
        b.cells[9] = 1;
        b.cells[50] = 3;
        b.turn = 0;
        assert_eq!(b.play(0, &json!({"from":9,"to":0})), Ok(None));
        assert_eq!(b.cells[0], 2, "crowned");
        b.cells = [0; 64];
        b.cells[42] = 1;
        b.cells[33] = 3;
        b.turn = 0;
        assert_eq!(b.play(0, &json!({"from":42,"to":24})), Ok(Some(Some(0))));
    }

    #[test]
    fn sketch_keeps_the_word_private_and_scores_guesses() {
        let mut s = Sketch::default();
        s.joined(1, "Ana");
        s.joined(2, "Ben");
        s.joined(3, "Cy");
        let fx = act(&mut s, 2, json!({"type":"start"})).unwrap();
        let drawer = s.drawer.unwrap();
        assert_eq!(fx.private.len(), 1);
        assert_eq!(fx.private[0].0, drawer);
        let state = &fx.writes.iter().find(|(k, _)| k == "s/game").unwrap().1;
        assert!(
            !state
                .as_ref()
                .unwrap()
                .contains(&format!("\"word\":\"{}\"", s.word)),
            "the word is not shared"
        );
        assert!(s.may_write(drawer, "d/1"));
        let guesser = [1, 2, 3].into_iter().find(|p| *p != drawer).unwrap();
        assert!(!s.may_write(guesser, "d/1"));
        assert!(act(&mut s, drawer, json!({"type":"guess","text":"x"})).is_err());
        let word = s.word.clone();
        act(&mut s, guesser, json!({"type":"guess","text":"wrong"})).unwrap();
        act(
            &mut s,
            guesser,
            json!({"type":"guess","text": word.to_uppercase()}),
        )
        .unwrap();
        assert!(act(&mut s, guesser, json!({"type":"guess","text":"again"})).is_err());
        let other = [1, 2, 3]
            .into_iter()
            .find(|p| *p != drawer && *p != guesser)
            .unwrap();
        act(&mut s, other, json!({"type":"guess","text": word})).unwrap();
        assert_eq!(s.status, "reveal");
        let fx = act(&mut s, other, json!({"type":"next"})).unwrap();
        assert_eq!(fx.clear.as_deref(), Some("d/"));
        assert_ne!(s.drawer, Some(drawer));
    }

    #[test]
    fn close_guesses_are_one_edit_away() {
        assert!(close("elephnt", "elephant"));
        assert!(close("elephamt", "elephant"));
        assert!(close("castl", "castle"));
        assert!(!close("cat", "cot"));
        assert!(!close("castle", "castle"));
        assert!(!close("dolphins!", "dolphin"));
    }
}
