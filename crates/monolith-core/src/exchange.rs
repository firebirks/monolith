//! What one session knows of file transfers (`docs/PROTOCOL.md` section
//! 13.1, `docs/adr/0013-file-transfer.md`): the transfers offered on it, by
//! either side, how far each went on it, and the last
//! [`MAX_ENDED_TRANSFER_IDS`] that ended on it.
//!
//! [`Exchange`] decides what a file message that arrived is, before
//! anything durable is looked at, and keeps what this side wrote: plain
//! data and synchronous steps, no I/O; the time is an argument. Its rules:
//!
//! - a file message names a transfer offered on the session, by either
//!   side; any other is a violation, and so is a second offer of one, or
//!   an offer of the peer that names a transfer this side offered;
//! - FileAccept and FileReject come from the side an offer was made to,
//!   FileChunk from the side that made it, after a FileAccept on the
//!   session, from the offset of that FileAccept on, with no gap and no
//!   overlap, each exactly `min(L, bytes left)`; FileComplete from the
//!   offerer only when every byte arrived;
//! - the offsets of the FileAccepts of one transfer on the session never
//!   go down, are whole chunks or the size, and never pass the bytes
//!   written on the session from the first;
//! - a transfer that ended on the session is remembered with the bytes
//!   the peer may still send for it, and whether a FileComplete may still
//!   come: a file message that names it is discarded, but a chunk beyond
//!   those bytes, a FileComplete where none can come (the peer was never
//!   told to send), and a FileAccept or FileReject for an offer of the
//!   peer are violations;
//! - the offerer writes no more than `FILE_WINDOW_CHUNKS` chunks beyond the
//!   last offset acknowledged on the session, for the oldest transfer
//!   accepted first, one transfer at a time.

use std::collections::{HashMap, VecDeque};

use monolith_protocol::body::Message;
use monolith_protocol::limits::{
    FILE_PROGRESS_CHUNKS, FILE_PROGRESS_INTERVAL, FILE_WINDOW_CHUNKS, MAX_ENDED_TRANSFER_IDS,
    MAX_FILE_CHUNK_LEN, TRANSFER_ID_LEN,
};
use monolith_protocol::text::Filename;
use monolith_storage::transfers::Direction;
use tokio::time::Instant;

/// L, the full chunk size: the bytes of every chunk but the last.
const CHUNK: u64 = MAX_FILE_CHUNK_LEN as u64;

type Id = [u8; TRANSFER_ID_LEN];

/// What a file message that arrived is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Received {
    /// It names a transfer that ended on the session, within what may
    /// still come: dropped. Not activity of the session.
    Discard,
    /// It breaks the rules: the session ends without a Close.
    Violation,
    /// An offer of the peer, new on the session.
    Offer { size: u64, name: Filename },
    /// The peer accepted an offer of this side, holding `offset` bytes;
    /// the first acceptance of it on the session if `first`.
    Accept { offset: u64, first: bool },
    /// The peer rejected an offer of this side.
    Reject,
    /// The next chunk of an offer of the peer, at `offset`.
    Chunk { offset: u64 },
    /// The peer sent every byte of its offer: to verify against `digest`.
    Verify { digest: [u8; 32] },
    /// The peer completed an offer of this side with `digest`, also one
    /// this side cancelled on the session.
    Completed { digest: [u8; 32] },
    /// The peer aborted the transfer.
    Abort,
}

/// The transfer a file message names, the direction of that transfer, and
/// what the message is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) transfer: Id,
    pub(crate) direction: Direction,
    pub(crate) received: Received,
}

/// A transfer this side offered on the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Out {
    size: u64,
    /// The offset of the last FileAccept of the peer on the session.
    acked: Option<u64>,
    /// The next byte to write.
    sent: u64,
    /// FileComplete was written.
    completed: bool,
    /// When it was first accepted on the session, among the others.
    order: u64,
}

/// A transfer the peer offered on the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct In {
    size: u64,
    /// The offset of the last FileAccept this side wrote on the session.
    accepted: Option<u64>,
    /// The next byte expected.
    next: u64,
    /// Chunks taken since the last FileAccept.
    unacked: u64,
    /// When the last FileAccept was written.
    at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Live {
    Out(Out),
    In(In),
}

/// Whether a FileComplete of the peer may still come for a transfer that
/// ended on the session, and what it is then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Completion {
    /// It cannot: the peer was never told to send. A violation.
    Never,
    /// It may have crossed the end: dropped.
    Discard,
    /// It may have crossed a cancellation of this side, and still counts:
    /// a receiver that holds the file holds it.
    Deliver,
}

/// A transfer that ended on the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ended {
    transfer: Id,
    direction: Direction,
    /// The bytes the peer may still send for it.
    remaining: u64,
    completion: Completion,
}

/// The file transfers of one session. See the module documentation.
#[derive(Debug, Default)]
pub(crate) struct Exchange {
    live: HashMap<Id, Live>,
    ended: VecDeque<Ended>,
    /// The number of acceptances so far, which orders the transfers sent.
    accepted: u64,
}

/// Whether `offset` can be an offset of a FileAccept of a transfer of
/// `size` bytes: whole chunks, or the size.
const fn valid_offset(offset: u64, size: u64) -> bool {
    offset == size || (offset < size && offset % CHUNK == 0)
}

/// The bytes of the chunk at `offset` of a transfer of `size` bytes.
fn chunk_at(offset: u64, size: u64) -> u64 {
    size.saturating_sub(offset).min(CHUNK)
}

impl Exchange {
    fn ended(&self, transfer: &Id) -> Option<usize> {
        self.ended
            .iter()
            .position(|ended| ended.transfer == *transfer)
    }

    /// What `message` is, if it is a file message; `None` for any other.
    pub(crate) fn receive(&mut self, message: &Message) -> Option<Step> {
        let transfer = *match message {
            Message::FileOffer { transfer, .. }
            | Message::FileAccept { transfer, .. }
            | Message::FileReject(transfer)
            | Message::FileComplete { transfer, .. }
            | Message::FileAbort(transfer) => transfer,
            Message::FileChunk(chunk) => chunk.transfer(),
            _ => return None,
        }
        .as_bytes();
        if let Some(index) = self.ended(&transfer) {
            return Some(self.after_end(index, message));
        }
        let Some(live) = self.live.get_mut(&transfer) else {
            // Only an offer can name a transfer the session does not know.
            let Message::FileOffer { size, filename, .. } = message else {
                return Some(Step {
                    transfer,
                    direction: Direction::Incoming,
                    received: Received::Violation,
                });
            };
            self.live.insert(
                transfer,
                Live::In(In {
                    size: *size,
                    accepted: None,
                    next: 0,
                    unacked: 0,
                    at: None,
                }),
            );
            return Some(Step {
                transfer,
                direction: Direction::Incoming,
                received: Received::Offer {
                    size: *size,
                    name: filename.clone(),
                },
            });
        };
        let (direction, received) = match live {
            Live::Out(out) => (
                Direction::Outgoing,
                Self::to_offerer(out, message, &mut self.accepted),
            ),
            Live::In(held) => (Direction::Incoming, Self::to_receiver(held, message)),
        };
        Some(Step {
            transfer,
            direction,
            received,
        })
    }

    /// A file message of the receiver of a transfer this side offered.
    fn to_offerer(out: &mut Out, message: &Message, accepted: &mut u64) -> Received {
        match message {
            Message::FileAccept { offset, .. } => {
                let offset = *offset;
                if !valid_offset(offset, out.size) {
                    return Received::Violation;
                }
                match out.acked {
                    None => {
                        out.acked = Some(offset);
                        out.sent = offset;
                        out.order = *accepted;
                        *accepted = accepted.saturating_add(1);
                        Received::Accept {
                            offset,
                            first: true,
                        }
                    }
                    Some(acked) if offset >= acked && offset <= out.sent => {
                        out.acked = Some(offset);
                        Received::Accept {
                            offset,
                            first: false,
                        }
                    }
                    Some(_) => Received::Violation,
                }
            }
            Message::FileReject(_) => Received::Reject,
            Message::FileComplete { digest, .. } => Received::Completed { digest: *digest },
            Message::FileAbort(_) => Received::Abort,
            // A chunk from the receiver, or an offer of the same transfer
            // back.
            _ => Received::Violation,
        }
    }

    /// A file message of the offerer of a transfer the peer offered.
    fn to_receiver(held: &mut In, message: &Message) -> Received {
        match message {
            Message::FileChunk(chunk) => {
                let len = u64::try_from(chunk.data().len()).unwrap_or(u64::MAX);
                if held.accepted.is_none()
                    || held.next >= held.size
                    || len != chunk_at(held.next, held.size)
                {
                    return Received::Violation;
                }
                let offset = held.next;
                held.next = held.next.saturating_add(len);
                held.unacked = held.unacked.saturating_add(1);
                Received::Chunk { offset }
            }
            Message::FileComplete { digest, .. } => {
                if held.accepted.is_some() && held.next == held.size {
                    Received::Verify { digest: *digest }
                } else {
                    Received::Violation
                }
            }
            Message::FileAbort(_) => Received::Abort,
            // A second offer of it, or an answer to an offer this side
            // never made.
            _ => Received::Violation,
        }
    }

    /// A file message that names a transfer that ended on the session.
    fn after_end(&mut self, index: usize, message: &Message) -> Step {
        let Some(ended) = self.ended.get_mut(index) else {
            return Step {
                transfer: [0; TRANSFER_ID_LEN],
                direction: Direction::Incoming,
                received: Received::Violation,
            };
        };
        let outgoing = ended.direction == Direction::Outgoing;
        let received = match message {
            Message::FileAbort(_) => Received::Discard,
            Message::FileOffer { .. } => {
                if outgoing {
                    // The peer offers a transfer this side offered.
                    Received::Violation
                } else {
                    Received::Discard
                }
            }
            Message::FileAccept { .. } | Message::FileReject(_) => {
                if outgoing {
                    Received::Discard
                } else {
                    Received::Violation
                }
            }
            Message::FileChunk(chunk) => {
                let len = u64::try_from(chunk.data().len()).unwrap_or(u64::MAX);
                if len <= ended.remaining {
                    ended.remaining = ended.remaining.saturating_sub(len);
                    Received::Discard
                } else {
                    Received::Violation
                }
            }
            Message::FileComplete { digest, .. } => match ended.completion {
                Completion::Never => Received::Violation,
                Completion::Discard => Received::Discard,
                Completion::Deliver => {
                    // Counted once: another is dropped.
                    ended.completion = Completion::Discard;
                    Received::Completed { digest: *digest }
                }
            },
            _ => Received::Violation,
        };
        Step {
            transfer: ended.transfer,
            direction: ended.direction,
            received,
        }
    }

    /// Whether this side may offer `transfer` on the session: it was not
    /// offered on it, by either side, and did not end on it.
    pub(crate) fn may_offer(&self, transfer: &Id) -> bool {
        !self.live.contains_key(transfer) && self.ended(transfer).is_none()
    }

    /// This side wrote an offer of `transfer`, of `size` bytes.
    pub(crate) fn offered(&mut self, transfer: Id, size: u64) {
        if self.may_offer(&transfer) {
            self.live.insert(
                transfer,
                Live::Out(Out {
                    size,
                    acked: None,
                    sent: 0,
                    completed: false,
                    order: 0,
                }),
            );
        }
    }

    /// Whether `transfer` is offered on the session in `direction`, and has
    /// not ended on it.
    pub(crate) fn is_live(&self, transfer: &Id, direction: Direction) -> bool {
        match self.live.get(transfer) {
            Some(Live::Out(_)) => direction == Direction::Outgoing,
            Some(Live::In(_)) => direction == Direction::Incoming,
            None => false,
        }
    }

    /// The transfers the peer offered on the session that have not ended
    /// on it.
    pub(crate) fn incoming(&self) -> Vec<Id> {
        self.live
            .iter()
            .filter(|(_, live)| matches!(live, Live::In(_)))
            .map(|(transfer, _)| *transfer)
            .collect()
    }

    /// The transfers this side offered on the session that have not ended
    /// on it.
    pub(crate) fn outgoing(&self) -> Vec<Id> {
        self.live
            .iter()
            .filter(|(_, live)| matches!(live, Live::Out(_)))
            .map(|(transfer, _)| *transfer)
            .collect()
    }

    /// The offset of the last FileAccept this side wrote for the offer
    /// `transfer` of the peer, if it wrote one.
    pub(crate) fn accepted(&self, transfer: &Id) -> Option<u64> {
        match self.live.get(transfer) {
            Some(Live::In(held)) => held.accepted,
            _ => None,
        }
    }

    /// The next byte expected of the offer `transfer` of the peer.
    pub(crate) fn expected(&self, transfer: &Id) -> Option<u64> {
        match self.live.get(transfer) {
            Some(Live::In(held)) => Some(held.next),
            _ => None,
        }
    }

    /// This side wrote `FileAccept(transfer, offset)` at `now`, `offset`
    /// being the bytes it holds durably. The first one on the session sets
    /// where the chunks start.
    pub(crate) fn accepting(&mut self, transfer: &Id, offset: u64, now: Instant) {
        if let Some(Live::In(held)) = self.live.get_mut(transfer) {
            if held.accepted.is_none() {
                held.next = offset;
            }
            held.accepted = Some(offset);
            held.unacked = 0;
            held.at = Some(now);
        }
    }

    /// The offers of the peer whose progress is to be acknowledged at
    /// `now`: `FILE_PROGRESS_CHUNKS` chunks taken since the last FileAccept,
    /// or some taken and `FILE_PROGRESS_INTERVAL` passed.
    pub(crate) fn progress_due(&self, now: Instant) -> Vec<Id> {
        self.live
            .iter()
            .filter_map(|(transfer, live)| match live {
                Live::In(held)
                    if held.unacked >= FILE_PROGRESS_CHUNKS
                        || (held.unacked > 0
                            && held.at.is_some_and(|at| {
                                now >= at.checked_add(FILE_PROGRESS_INTERVAL).unwrap_or(at)
                            })) =>
                {
                    Some(*transfer)
                }
                _ => None,
            })
            .collect()
    }

    /// When the next acknowledgement of progress is due by its interval.
    pub(crate) fn progress_deadline(&self) -> Option<Instant> {
        self.live
            .values()
            .filter_map(|live| match live {
                Live::In(held) if held.unacked > 0 => held
                    .at
                    .map(|at| at.checked_add(FILE_PROGRESS_INTERVAL).unwrap_or(at)),
                _ => None,
            })
            .min()
    }

    /// The chunk to write next, if the window has room: of the transfer
    /// accepted first on the session among those not fully written, its
    /// offset and its length. Only that transfer sends.
    pub(crate) fn next_chunk(&self) -> Option<(Id, u64, u64)> {
        let (transfer, out) = self
            .live
            .iter()
            .filter_map(|(transfer, live)| match live {
                Live::Out(out) if out.acked.is_some() && out.sent < out.size => {
                    Some((*transfer, *out))
                }
                _ => None,
            })
            .min_by_key(|(_, out)| out.order)?;
        let acked = out.acked.unwrap_or(out.sent);
        let in_flight = out.sent.saturating_sub(acked).div_ceil(CHUNK);
        if in_flight >= FILE_WINDOW_CHUNKS {
            return None;
        }
        Some((transfer, out.sent, chunk_at(out.sent, out.size)))
    }

    /// This side wrote `len` bytes of `transfer`.
    pub(crate) fn chunk_written(&mut self, transfer: &Id, len: u64) {
        if let Some(Live::Out(out)) = self.live.get_mut(transfer) {
            out.sent = out.sent.saturating_add(len).min(out.size);
        }
    }

    /// The offers of this side whose FileComplete is due: accepted on the
    /// session, every byte written, no FileComplete yet.
    pub(crate) fn completion_due(&self) -> Vec<Id> {
        self.live
            .iter()
            .filter_map(|(transfer, live)| match live {
                Live::Out(out) if out.acked.is_some() && out.sent == out.size && !out.completed => {
                    Some(*transfer)
                }
                _ => None,
            })
            .collect()
    }

    /// This side wrote FileComplete for `transfer`.
    pub(crate) fn completion_written(&mut self, transfer: &Id) {
        if let Some(Live::Out(out)) = self.live.get_mut(transfer) {
            out.completed = true;
        }
    }

    /// The bytes the offerer may still have on the way for the offer
    /// `held` of the peer: what its window allows beyond the last offset
    /// acknowledged on the session, less what arrived.
    fn in_flight(held: &In) -> u64 {
        held.accepted.map_or(0, |accepted| {
            let window = CHUNK.saturating_mul(FILE_WINDOW_CHUNKS);
            accepted
                .saturating_add(window)
                .min(held.size)
                .saturating_sub(held.next)
        })
    }

    /// `transfer` ended on the session: what this side wrote last for it
    /// ended it if `by_us` (a cancellation, a rejection, a final answer, or
    /// a binding that moved to another session), otherwise what the peer
    /// sent. It is remembered among the last `MAX_ENDED_TRANSFER_IDS`, with
    /// what the peer may still send.
    pub(crate) fn end(&mut self, transfer: &Id, by_us: bool) {
        let Some(live) = self.live.remove(transfer) else {
            return;
        };
        let (direction, remaining, completion) = match live {
            Live::Out(_) => (
                Direction::Outgoing,
                0,
                if by_us {
                    Completion::Deliver
                } else {
                    Completion::Discard
                },
            ),
            Live::In(held) => {
                let told = held.accepted.is_some();
                let complete = told && held.next == held.size;
                (
                    Direction::Incoming,
                    if by_us && !complete {
                        Self::in_flight(&held)
                    } else {
                        0
                    },
                    if told {
                        Completion::Discard
                    } else {
                        Completion::Never
                    },
                )
            }
        };
        if self.ended.len() >= MAX_ENDED_TRANSFER_IDS {
            self.ended.pop_front();
        }
        self.ended.push_back(Ended {
            transfer: *transfer,
            direction,
            remaining,
            completion,
        });
    }
}

#[cfg(test)]
mod tests;
