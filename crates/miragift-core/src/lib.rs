//! Miracle Gift Parade penlight ("ミラギフ棒") acoustic control protocol.
//!
//! Protocol (as used by ohtake/miracle-light-controller, see `PROTOCOL.md`):
//!
//! * 5 near-ultrasonic tones, 250 Hz apart: `F0=18500, F1=18750, F2=19000, F3=19250, F4=19500` Hz.
//! * A packet is a preamble `F0` for 190 ms, followed by 7 symbols in 100 ms slots
//!   (90 ms tone + 10 ms gap) starting 200 ms after the preamble start.
//! * Even positions (0,2,4,6) are `F1` or `F3`, odd positions (1,3,5) are `F2` or `F4`,
//!   so each slot carries one bit (low / high tone) and neighbouring slots never share a tone.
//! * Commands are written as 7-digit strings of tone indices, e.g. `"3234321"` (red).

pub mod decoder;
pub mod synth;
mod table;

pub use decoder::{DecodedPacket, Decoder, DecoderConfig};
pub use synth::{synthesize, SynthConfig};

/// Tone frequencies in Hz. Index 0 is the preamble tone.
pub const TONES_HZ: [f64; 5] = [18500.0, 18750.0, 19000.0, 19250.0, 19500.0];
/// Number of data symbols per packet.
pub const SYMBOLS: usize = 7;
/// Preamble tone length (s).
pub const PREAMBLE_SEC: f64 = 0.19;
/// Offset of the first data slot from the preamble start (s).
pub const FIRST_SLOT_SEC: f64 = 0.20;
/// Data slot period (s).
pub const SLOT_SEC: f64 = 0.10;
/// Tone length within a data slot (s).
pub const SYMBOL_SEC: f64 = 0.09;
/// Length of one packet from preamble start to end of the last tone (s).
pub const PACKET_SEC: f64 = FIRST_SLOT_SEC + SLOT_SEC * (SYMBOLS as f64 - 1.0) + SYMBOL_SEC;

/// A 7-bit command. Bit `i` set = position `i` uses the *high* tone of its pair.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Command(u8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Length(usize),
    BadDigit { pos: usize, ch: char },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Length(n) => write!(f, "command must be 7 digits, got {n}"),
            ParseError::BadDigit { pos, ch } => write!(
                f,
                "invalid digit '{ch}' at position {pos} (even positions take 1/3, odd positions take 2/4)"
            ),
        }
    }
}
impl std::error::Error for ParseError {}

impl Command {
    /// Build from the raw 7-bit mask (bit i = high tone at position i).
    pub const fn from_bits(bits: u8) -> Self {
        Command(bits & 0x7f)
    }
    pub const fn bits(self) -> u8 {
        self.0
    }
    /// Tone index (1..=4) used at slot `pos`.
    pub const fn tone_at(self, pos: usize) -> usize {
        let low = if pos % 2 == 0 { 1 } else { 2 };
        if (self.0 >> pos) & 1 == 1 {
            low + 2
        } else {
            low
        }
    }
    /// Tone indices for the 7 data slots.
    pub fn tones(self) -> [usize; SYMBOLS] {
        std::array::from_fn(|i| self.tone_at(i))
    }
    /// The numeric ID shown in the reference controller's "all" table
    /// (`convertCommandToId(cmd, true, true)`: bit i = 1 when position i is the *low* tone).
    pub const fn reference_id(self) -> u8 {
        !self.0 & 0x7f
    }
    pub const fn from_reference_id(id: u8) -> Self {
        Command(!id & 0x7f)
    }
    /// Look up names from the reference table.
    pub fn named(self) -> impl Iterator<Item = &'static NamedCommand> {
        table::NAMED.iter().filter(move |n| n.command() == self)
    }
    pub fn known(self) -> Option<&'static KnownCommand> {
        table::ALL.iter().find(|k| k.command() == self)
    }
}

impl std::str::FromStr for Command {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, ParseError> {
        let chars: Vec<char> = s.trim().chars().collect();
        if chars.len() != SYMBOLS {
            return Err(ParseError::Length(chars.len()));
        }
        let mut bits = 0u8;
        for (pos, &ch) in chars.iter().enumerate() {
            let high = match (pos % 2, ch) {
                (0, '1') | (1, '2') => false,
                (0, '3') | (1, '4') => true,
                _ => return Err(ParseError::BadDigit { pos, ch }),
            };
            bits |= (high as u8) << pos;
        }
        Ok(Command(bits))
    }
}

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for t in self.tones() {
            write!(f, "{t}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    Red,
    Orange,
    Pink,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    White,
    Black,
}

impl Color {
    pub const fn name(self) -> &'static str {
        match self {
            Color::Red => "red",
            Color::Orange => "orange",
            Color::Pink => "pink",
            Color::Yellow => "yellow",
            Color::Green => "green",
            Color::Cyan => "cyan",
            Color::Blue => "blue",
            Color::Purple => "purple",
            Color::White => "white",
            Color::Black => "black",
        }
    }
    pub const fn css(self) -> &'static str {
        match self {
            Color::Red => "#F44336",
            Color::Orange => "#FF9800",
            Color::Pink => "#F48FB1",
            Color::Yellow => "#FFEB3B",
            Color::Green => "#4CAF50",
            Color::Cyan => "#00BCD4",
            Color::Blue => "#2196F3",
            Color::Purple => "#9C27B0",
            Color::White => "#FAFAFA",
            Color::Black => "#424242",
        }
    }
}

/// A command with a human name (from the reference controller's curated sections).
#[derive(Debug)]
pub struct NamedCommand {
    pub group: &'static str,
    pub name: &'static str,
    pub code: &'static str,
    /// Blink tempo hint (only used for display); 10 = steady.
    pub bpm: u32,
    pub colors: &'static [Color],
}

/// An entry of the full 128-command table. `colors` is empty when the effect is unknown.
#[derive(Debug)]
pub struct KnownCommand {
    pub code: &'static str,
    pub bpm: u32,
    pub colors: &'static [Color],
    /// Only Miracle *Strawberry* Light reacts (per reference notes).
    pub strawberry_only: bool,
}

impl NamedCommand {
    pub fn command(&self) -> Command {
        self.code.parse().expect("table entry")
    }
}
impl KnownCommand {
    pub fn command(&self) -> Command {
        self.code.parse().expect("table entry")
    }
}

pub fn named_commands() -> &'static [NamedCommand] {
    table::NAMED
}
pub fn all_commands() -> &'static [KnownCommand] {
    table::ALL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_codes() {
        for b in 0..128u8 {
            let c = Command::from_bits(b);
            let s = c.to_string();
            assert_eq!(s.parse::<Command>().unwrap(), c);
            assert_eq!(Command::from_reference_id(c.reference_id()), c);
        }
    }

    #[test]
    fn reference_id_matches_mlc_ts() {
        // convertCommandToId('3212341', true, true) == 78 in the reference implementation.
        assert_eq!("3212341".parse::<Command>().unwrap().reference_id(), 78);
        assert_eq!("1212121".parse::<Command>().unwrap().reference_id(), 127);
        assert_eq!("3434343".parse::<Command>().unwrap().reference_id(), 0);
    }

    #[test]
    fn rejects_bad_codes() {
        assert!("2212341".parse::<Command>().is_err());
        assert!("321234".parse::<Command>().is_err());
        assert!("3312341".parse::<Command>().is_err());
    }

    #[test]
    fn table_is_complete() {
        assert_eq!(all_commands().len(), 128);
        let mut seen = std::collections::HashSet::new();
        for k in all_commands() {
            assert!(seen.insert(k.command()));
        }
        for n in named_commands() {
            assert!(n.command().known().is_some());
        }
    }

    #[test]
    fn packet_length() {
        assert!((PACKET_SEC - 0.89).abs() < 1e-9);
    }
}
