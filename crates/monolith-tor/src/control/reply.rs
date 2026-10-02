//! Reading Tor control replies, with bounds.
//!
//! A reply is one or more lines `<3 digits><separator><text>CRLF`. Lines
//! with `-` continue the reply; the line with a space ends it. Monolith
//! sends no command whose reply has a data block, and subscribes to no
//! event, so a `+` separator or an asynchronous `650` reply is refused.
//!
//! Every limit is checked before a byte is kept: the length of a line
//! (`MAX_CONTROL_LINE_LEN`), the number of lines
//! (`MAX_CONTROL_REPLY_LINES`) and the size of the whole reply
//! (`MAX_CONTROL_REPLY_LEN`). A bare CR or LF is refused; the
//! specification requires CRLF. A reply may contain a private key, so every
//! buffer is erased when it is dropped.

use core::fmt;

use monolith_protocol::limits::{
    MAX_CONTROL_LINE_LEN, MAX_CONTROL_REPLY_LEN, MAX_CONTROL_REPLY_LINES,
};
use zeroize::Zeroizing;

use crate::TorError;

/// One line of a reply, without its status code, separator and CRLF.
pub struct ReplyLine {
    code: u16,
    text: Zeroizing<Vec<u8>>,
}

impl ReplyLine {
    /// The status code of the line.
    pub const fn code(&self) -> u16 {
        self.code
    }

    /// The text after the separator.
    pub fn text(&self) -> &[u8] {
        &self.text
    }
}

impl fmt::Debug for ReplyLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The text can hold a private key.
        write!(f, "ReplyLine({}, {} bytes)", self.code, self.text.len())
    }
}

/// A complete reply.
pub struct Reply {
    lines: Vec<ReplyLine>,
}

impl Reply {
    /// The status code of the reply. Every line has the same.
    pub fn code(&self) -> u16 {
        self.lines.last().map_or(0, ReplyLine::code)
    }

    /// The lines, in order. The last one ended the reply.
    pub fn lines(&self) -> &[ReplyLine] {
        &self.lines
    }
}

impl fmt::Debug for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Reply({}, {} lines)", self.code(), self.lines.len())
    }
}

/// Assembles replies from bytes as they arrive.
pub struct ReplyParser {
    /// The line being read, with its status code and separator.
    line: Zeroizing<Vec<u8>>,
    /// A CR was the last byte read.
    after_cr: bool,
    lines: Vec<ReplyLine>,
    /// Bytes of the current reply so far, line ends included.
    total: usize,
    failed: bool,
}

impl Default for ReplyParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplyParser {
    /// A parser with nothing read.
    pub fn new() -> Self {
        Self {
            line: Zeroizing::new(Vec::with_capacity(MAX_CONTROL_LINE_LEN.saturating_add(4))),
            after_cr: false,
            lines: Vec::new(),
            total: 0,
            failed: false,
        }
    }

    /// Takes bytes from `input` up to the end of one reply. Returns how many
    /// were taken and the reply, if they completed one. After an error the
    /// parser takes nothing more.
    pub fn feed(&mut self, input: &[u8]) -> Result<(usize, Option<Reply>), TorError> {
        if self.failed {
            return Err(TorError::InvalidTorResponse);
        }
        for (index, byte) in input.iter().enumerate() {
            match self.byte(*byte) {
                Ok(None) => {}
                Ok(Some(reply)) => return Ok((index.saturating_add(1), Some(reply))),
                Err(error) => {
                    self.failed = true;
                    return Err(error);
                }
            }
        }
        Ok((input.len(), None))
    }

    fn byte(&mut self, byte: u8) -> Result<Option<Reply>, TorError> {
        self.total = self.total.saturating_add(1);
        if self.total > MAX_CONTROL_REPLY_LEN {
            return Err(TorError::InvalidTorResponse);
        }
        if self.after_cr {
            // CR must be followed by LF.
            if byte != b'\n' {
                return Err(TorError::InvalidTorResponse);
            }
            self.after_cr = false;
            return self.end_of_line();
        }
        match byte {
            b'\r' => {
                self.after_cr = true;
                Ok(None)
            }
            // LF without CR.
            b'\n' => Err(TorError::InvalidTorResponse),
            _ => {
                // Status code and separator, then at most the line limit.
                if self.line.len() >= MAX_CONTROL_LINE_LEN.saturating_add(4) {
                    return Err(TorError::InvalidTorResponse);
                }
                self.line.push(byte);
                Ok(None)
            }
        }
    }

    fn end_of_line(&mut self) -> Result<Option<Reply>, TorError> {
        let line = core::mem::replace(
            &mut self.line,
            Zeroizing::new(Vec::with_capacity(MAX_CONTROL_LINE_LEN.saturating_add(4))),
        );
        let (head, text) = line.split_at_checked(4).ok_or(TorError::InvalidTorResponse)?;
        let [a, b, c, separator] = *head else {
            return Err(TorError::InvalidTorResponse);
        };
        if text.len() > MAX_CONTROL_LINE_LEN {
            return Err(TorError::InvalidTorResponse);
        }
        let code = status_code(a, b, c).ok_or(TorError::InvalidTorResponse)?;
        // Asynchronous events: Monolith subscribes to none.
        if (600..700).contains(&code) {
            return Err(TorError::InvalidTorResponse);
        }
        if self.lines.first().is_some_and(|first| first.code != code) {
            return Err(TorError::InvalidTorResponse);
        }
        if self.lines.len() >= MAX_CONTROL_REPLY_LINES {
            return Err(TorError::InvalidTorResponse);
        }
        let line = ReplyLine {
            code,
            text: Zeroizing::new(text.to_vec()),
        };
        match separator {
            b'-' => {
                self.lines.push(line);
                Ok(None)
            }
            b' ' => {
                self.lines.push(line);
                self.total = 0;
                let lines = core::mem::take(&mut self.lines);
                Ok(Some(Reply { lines }))
            }
            // A data block (`+`) or anything else.
            _ => Err(TorError::InvalidTorResponse),
        }
    }
}

/// A three-digit status code from 200 to 599.
fn status_code(a: u8, b: u8, c: u8) -> Option<u16> {
    if !(b'1'..=b'6').contains(&a) || !b.is_ascii_digit() || !c.is_ascii_digit() {
        return None;
    }
    let digit = |byte: u8| u16::from(byte.wrapping_sub(b'0'));
    digit(a)
        .checked_mul(100)?
        .checked_add(digit(b).checked_mul(10)?)?
        .checked_add(digit(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(input: &[u8]) -> Result<Vec<Reply>, TorError> {
        let mut parser = ReplyParser::new();
        let mut rest = input;
        let mut replies = Vec::new();
        while !rest.is_empty() {
            let (used, reply) = parser.feed(rest)?;
            rest = &rest[used..];
            replies.extend(reply);
        }
        Ok(replies)
    }

    #[test]
    fn replies_are_split_at_their_last_line() {
        let input = b"250-PROTOCOLINFO 1\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n250 OK\r\n";
        let replies = parse_all(input).unwrap();
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0].code(), 250);
        assert_eq!(replies[0].lines().len(), 3);
        assert_eq!(replies[0].lines()[0].text(), b"PROTOCOLINFO 1");
        assert_eq!(replies[0].lines()[2].text(), b"OK");
        assert_eq!(replies[1].lines().len(), 1);

        // One call never goes past the end of a reply.
        let mut parser = ReplyParser::new();
        let (used, reply) = parser.feed(b"250 OK\r\n250 OK\r\n").unwrap();
        assert_eq!(used, 8);
        assert!(reply.is_some());
    }

    #[test]
    fn any_fragmentation_gives_the_same_replies() {
        let input = b"250-ServiceID=abc\r\n250-PrivateKey=ED25519-V3:xyz\r\n250 OK\r\n515 Authentication failed\r\n";
        for piece in 1..input.len() {
            let mut parser = ReplyParser::new();
            let mut replies = Vec::new();
            for chunk in input.chunks(piece) {
                let mut rest = chunk;
                while !rest.is_empty() {
                    let (used, reply) = parser.feed(rest).unwrap();
                    rest = &rest[used..];
                    replies.extend(reply);
                }
            }
            assert_eq!(replies.len(), 2, "{piece}");
            assert_eq!(replies[0].lines().len(), 3);
            assert_eq!(replies[1].code(), 515);
        }
    }

    #[test]
    fn malformed_lines_end_the_parser() {
        for bad in [
            &b"250 OK\n"[..],
            b"250 OK\r\r\n",
            b"250 OK\rX\n",
            b"25 OK\r\n",
            b"250\r\n",
            b"2500 OK\r\n",
            b"abc OK\r\n",
            b"250+data\r\n",
            b"250_OK\r\n",
            b"650 CIRC 1 BUILT\r\n",
            b"050 OK\r\n",
            b"750 OK\r\n",
            b"250-a\r\n251 b\r\n",
        ] {
            let mut parser = ReplyParser::new();
            assert_eq!(
                parser.feed(bad).map(|(_, reply)| reply.is_some()),
                Err(TorError::InvalidTorResponse),
                "{bad:?}"
            );
            // Failed for good.
            assert!(parser.feed(b"250 OK\r\n").is_err());
        }
    }

    #[test]
    fn every_limit_is_enforced() {
        // The longest line passes; one byte more does not.
        let mut longest = b"250 ".to_vec();
        longest.extend(core::iter::repeat_n(b'x', MAX_CONTROL_LINE_LEN));
        longest.extend_from_slice(b"\r\n");
        assert_eq!(parse_all(&longest).unwrap().len(), 1);
        let mut too_long = b"250 ".to_vec();
        too_long.extend(core::iter::repeat_n(b'x', MAX_CONTROL_LINE_LEN + 1));
        too_long.extend_from_slice(b"\r\n");
        assert!(parse_all(&too_long).is_err());
        // A line that never ends is refused once it is too long, before
        // its end arrives.
        let endless = vec![b'2'; MAX_CONTROL_LINE_LEN + 10];
        let mut parser = ReplyParser::new();
        assert!(parser.feed(&endless).is_err());

        // Too many lines.
        let mut many = Vec::new();
        for _ in 0..MAX_CONTROL_REPLY_LINES {
            many.extend_from_slice(b"250-x\r\n");
        }
        many.extend_from_slice(b"250 OK\r\n");
        assert!(parse_all(&many).is_err());
        let mut enough = Vec::new();
        for _ in 0..MAX_CONTROL_REPLY_LINES - 1 {
            enough.extend_from_slice(b"250-x\r\n");
        }
        enough.extend_from_slice(b"250 OK\r\n");
        assert_eq!(parse_all(&enough).unwrap().len(), 1);

        // Too many bytes in one reply, with lines that are each allowed.
        let mut large = Vec::new();
        for _ in 0..5 {
            large.extend_from_slice(b"250-");
            large.extend(core::iter::repeat_n(b'y', 1000));
            large.extend_from_slice(b"\r\n");
        }
        large.extend_from_slice(b"250 OK\r\n");
        assert!(parse_all(&large).is_err());
    }

    #[test]
    fn debug_output_shows_no_text() {
        let replies = parse_all(b"250-PrivateKey=ED25519-V3:secret\r\n250 OK\r\n").unwrap();
        let printed = format!("{:?} {:?}", replies[0], replies[0].lines()[0]);
        assert!(!printed.contains("secret"));
        assert_eq!(printed, "Reply(250, 2 lines) ReplyLine(250, 28 bytes)");
    }
}
