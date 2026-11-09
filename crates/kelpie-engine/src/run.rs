//! Run ids: the mark every process of one run carries

use std::fmt;
use std::io;
use std::str::FromStr;

/// The environment variable a child carries its run id in
pub const ENV: &str = "KELPIE_RUN";

// 128 bits, shown as two hex digits per byte
const BYTES: usize = 16;
const HEX_DIGITS: usize = BYTES * 2;

/// One run's id: 128 random bits, shown as 32 lowercase hex digits
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunId(u128);

impl RunId {
    /// Draw a new run id from the system's random source
    ///
    /// # Errors
    ///
    /// The system's random source is unavailable.
    pub fn new() -> io::Result<RunId> {
        let mut bytes = [0u8; BYTES];
        getrandom::fill(&mut bytes).map_err(io::Error::from)?;
        Ok(Self(u128::from_be_bytes(bytes)))
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:0width$x}", self.0, width = HEX_DIGITS)
    }
}

/// Why a text is not a run id
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseRunIdError {
    /// The text is not 32 lowercase hex digits
    NotRunId,
}

impl fmt::Display for ParseRunIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotRunId => "expected 32 lowercase hex digits",
        })
    }
}

impl core::error::Error for ParseRunIdError {}

impl FromStr for RunId {
    type Err = ParseRunIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != HEX_DIGITS || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(ParseRunIdError::NotRunId);
        }
        u128::from_str_radix(s, 16)
            .map(Self)
            .map_err(|_| ParseRunIdError::NotRunId)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_id_round_trips_through_its_text() {
        let run = RunId::new().unwrap();
        let text = run.to_string();
        assert_eq!(text.len(), HEX_DIGITS, "{text}");
        assert_eq!(text.parse::<RunId>().unwrap(), run);
    }

    #[test]
    fn two_runs_never_share_an_id() {
        assert_ne!(
            RunId::new().unwrap().to_string(),
            RunId::new().unwrap().to_string()
        );
    }

    #[test]
    fn text_that_is_not_32_hex_digits_is_refused() {
        let hex = "deadbeef".repeat(4);
        let cases = [
            String::new(),
            "deadbeef".to_string(),
            hex.clone() + "0",
            hex.clone() + "g",
            "g".repeat(32),
            hex.to_uppercase(),
        ];
        for text in &cases {
            assert_eq!(
                text.parse::<RunId>().unwrap_err(),
                ParseRunIdError::NotRunId,
                "refused: {text:?}"
            );
        }
    }
}
