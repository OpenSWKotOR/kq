use serde::{Deserialize, Serialize};

/// Which game an installation belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Game {
    /// Knights of the Old Republic.
    K1,
    /// The Sith Lords.
    K2,
}

impl Game {
    pub fn as_str(self) -> &'static str {
        match self {
            Game::K1 => "k1",
            Game::K2 => "k2",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Game::K1 => "Star Wars: Knights of the Old Republic",
            Game::K2 => "Star Wars: Knights of the Old Republic II - The Sith Lords",
        }
    }

    pub fn parse(s: &str) -> Option<Game> {
        match s.trim().to_ascii_lowercase().as_str() {
            "k1" | "1" | "kotor" | "kotor1" => Some(Game::K1),
            "k2" | "2" | "tsl" | "kotor2" => Some(Game::K2),
            _ => None,
        }
    }
}
