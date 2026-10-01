//! Message bodies.
//!
//! [`Message`] is one decoded protocol message. The layouts are those of
//! `docs/PROTOCOL.md` section 8. Decoding a body checks every field against
//! its limit and rejects anything left over, so a body has exactly one
//! encoding and `decode` followed by `encode_body` gives back the input.
//!
//! This module does not decide whether a message is allowed at a given
//! moment. That is the job of the state gate in the frame decoder and of
//! [`crate::session`].

use core::fmt;

use monolith_identity::redact::REDACTED;

use crate::card::{ContactCard, InvitationCapability};
use crate::codec::{Reader, Writer};
use crate::limits::{
    DIGEST_LEN, MAX_CHAT_TEXT_LEN, MAX_DISPLAY_NAME_LEN, MAX_FILE_CHUNK_LEN, MAX_FILE_SIZE,
    MAX_FILENAME_LEN, MAX_INTRODUCTION_TEXT_LEN, MAX_PROFILE_TEXT_LEN, MESSAGE_ID_LEN,
    PING_NONCE_LEN, TRANSFER_ID_LEN,
};
use crate::text::{ChatText, DisplayName, Filename, IntroductionText, ProfileText};
use crate::{MessageType, ProtocolError};

/// Identifier of a chat message, chosen at random by its sender.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId([u8; MESSAGE_ID_LEN]);

impl MessageId {
    /// Wraps identifier bytes. They must come from a CSPRNG.
    pub const fn from_bytes(bytes: [u8; MESSAGE_ID_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the identifier bytes.
    pub const fn as_bytes(&self) -> &[u8; MESSAGE_ID_LEN] {
        &self.0
    }
}

impl fmt::Debug for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MessageId({REDACTED})")
    }
}

/// Identifier of a file transfer, chosen at random by the offerer.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransferId([u8; TRANSFER_ID_LEN]);

impl TransferId {
    /// Wraps identifier bytes. They must come from a CSPRNG.
    pub const fn from_bytes(bytes: [u8; TRANSFER_ID_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the identifier bytes.
    pub const fn as_bytes(&self) -> &[u8; TRANSFER_ID_LEN] {
        &self.0
    }
}

impl fmt::Debug for TransferId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransferId({REDACTED})")
    }
}

/// A request to become a contact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactRequest {
    /// The sender's own card. It carries no invitation capability. The
    /// receiver must check it against the handshake of the session; that
    /// is done by [`crate::session::Session`].
    pub card: ContactCard,
    /// The invitation capability copied from the receiver's card, if any.
    pub invitation: Option<InvitationCapability>,
    /// The name the sender suggests for itself. Untrusted.
    pub display_name: DisplayName,
    /// A short text for the receiving user. Untrusted.
    pub introduction: IntroductionText,
}

/// One piece of file data.
#[derive(Clone, PartialEq, Eq)]
pub struct FileChunk {
    transfer: TransferId,
    data: Vec<u8>,
}

impl FileChunk {
    /// Builds a chunk. The data must be 1 to [`MAX_FILE_CHUNK_LEN`] bytes.
    pub fn new(transfer: TransferId, data: Vec<u8>) -> Result<Self, ProtocolError> {
        if data.is_empty() {
            return Err(ProtocolError::InvalidValue);
        }
        if data.len() > MAX_FILE_CHUNK_LEN {
            return Err(ProtocolError::FieldTooLong);
        }
        Ok(Self { transfer, data })
    }

    /// Returns the transfer this chunk belongs to.
    pub const fn transfer(&self) -> &TransferId {
        &self.transfer
    }

    /// Returns the file data.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

impl fmt::Debug for FileChunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileChunk({REDACTED})")
    }
}

/// One decoded protocol message.
///
/// The `Debug` output is the message type and nothing of the content.
#[derive(Clone, PartialEq, Eq)]
pub enum Message {
    /// The sender is closing the session.
    Close,
    /// Keepalive probe.
    Ping([u8; PING_NONCE_LEN]),
    /// Answer to a Ping with the same nonce.
    Pong([u8; PING_NONCE_LEN]),
    /// Request to become a contact.
    ContactRequest(Box<ContactRequest>),
    /// The sender holds the receiver as an accepted contact.
    ContactAccept,
    /// Chat text.
    ChatMessage {
        /// Identifier of the message.
        id: MessageId,
        /// The text.
        text: ChatText,
    },
    /// The message with this identifier was received and handed on.
    MessageAck(MessageId),
    /// The sender's profile.
    Profile {
        /// Suggested display name. Untrusted.
        display_name: DisplayName,
        /// Profile text. Untrusted.
        profile_text: ProfileText,
    },
    /// The sender's current card, without invitation capability.
    EndpointUpdate(Box<ContactCard>),
    /// Offer to send a file.
    FileOffer {
        /// Identifier of the transfer.
        transfer: TransferId,
        /// Exact size of the file in bytes.
        size: u64,
        /// Display name of the file. Never a path.
        filename: Filename,
    },
    /// The receiver accepts an offer.
    FileAccept(TransferId),
    /// The receiver declines an offer.
    FileReject(TransferId),
    /// File data.
    FileChunk(FileChunk),
    /// End of a transfer, with the SHA-256 of the whole file.
    FileComplete {
        /// Identifier of the transfer.
        transfer: TransferId,
        /// SHA-256 of the file.
        digest: [u8; DIGEST_LEN],
    },
    /// Either side ends a transfer.
    FileAbort(TransferId),
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Message")
            .field(&self.message_type())
            .finish()
    }
}

impl Message {
    /// Returns the type of this message.
    pub const fn message_type(&self) -> MessageType {
        match self {
            Self::Close => MessageType::Close,
            Self::Ping(_) => MessageType::Ping,
            Self::Pong(_) => MessageType::Pong,
            Self::ContactRequest(_) => MessageType::ContactRequest,
            Self::ContactAccept => MessageType::ContactAccept,
            Self::ChatMessage { .. } => MessageType::ChatMessage,
            Self::MessageAck(_) => MessageType::MessageAck,
            Self::Profile { .. } => MessageType::Profile,
            Self::EndpointUpdate(_) => MessageType::EndpointUpdate,
            Self::FileOffer { .. } => MessageType::FileOffer,
            Self::FileAccept(_) => MessageType::FileAccept,
            Self::FileReject(_) => MessageType::FileReject,
            Self::FileChunk(_) => MessageType::FileChunk,
            Self::FileComplete { .. } => MessageType::FileComplete,
            Self::FileAbort(_) => MessageType::FileAbort,
        }
    }

    /// Encodes the body of this message.
    ///
    /// Text fields, chunks and cards are validated by their own types. Two
    /// things are not, because the variants that hold them have public
    /// fields: a card inside a message must carry no invitation capability,
    /// and the size in a file offer must not exceed [`MAX_FILE_SIZE`]. This
    /// fails with [`ProtocolError::InvalidValue`] for a message that breaks
    /// either rule, so such a message cannot be put on the wire.
    pub fn encode_body(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut writer = Writer::new();
        match self {
            Self::Close | Self::ContactAccept => {}
            Self::Ping(nonce) | Self::Pong(nonce) => writer.raw(nonce),
            Self::ContactRequest(request) => {
                if request.card.invitation().is_some() {
                    return Err(ProtocolError::InvalidValue);
                }
                writer.raw(&request.card.encode());
                writer.presence(request.invitation.is_some());
                if let Some(capability) = &request.invitation {
                    writer.raw(capability.expose());
                }
                writer.bytes(request.display_name.as_bytes(), MAX_DISPLAY_NAME_LEN)?;
                writer.bytes(request.introduction.as_bytes(), MAX_INTRODUCTION_TEXT_LEN)?;
            }
            Self::ChatMessage { id, text } => {
                writer.raw(id.as_bytes());
                writer.bytes(text.as_bytes(), MAX_CHAT_TEXT_LEN)?;
            }
            Self::MessageAck(id) => writer.raw(id.as_bytes()),
            Self::Profile {
                display_name,
                profile_text,
            } => {
                writer.bytes(display_name.as_bytes(), MAX_DISPLAY_NAME_LEN)?;
                writer.bytes(profile_text.as_bytes(), MAX_PROFILE_TEXT_LEN)?;
            }
            Self::EndpointUpdate(card) => {
                if card.invitation().is_some() {
                    return Err(ProtocolError::InvalidValue);
                }
                writer.raw(&card.encode());
            }
            Self::FileOffer {
                transfer,
                size,
                filename,
            } => {
                if *size > MAX_FILE_SIZE {
                    return Err(ProtocolError::InvalidValue);
                }
                writer.raw(transfer.as_bytes());
                writer.u64(*size);
                writer.bytes(filename.as_bytes(), MAX_FILENAME_LEN)?;
            }
            Self::FileAccept(transfer) | Self::FileReject(transfer) | Self::FileAbort(transfer) => {
                writer.raw(transfer.as_bytes());
            }
            Self::FileChunk(chunk) => {
                writer.raw(chunk.transfer.as_bytes());
                writer.bytes(&chunk.data, MAX_FILE_CHUNK_LEN)?;
            }
            Self::FileComplete { transfer, digest } => {
                writer.raw(transfer.as_bytes());
                writer.raw(digest);
            }
        }
        Ok(writer.into_bytes())
    }

    /// Decodes the body of a message of the given type.
    ///
    /// The whole body must be consumed. A body that is shorter or longer
    /// than its fields require is rejected.
    pub fn decode(message_type: MessageType, body: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(body);
        let message = match message_type {
            MessageType::Close => Self::Close,
            MessageType::Ping => Self::Ping(reader.array()?),
            MessageType::Pong => Self::Pong(reader.array()?),
            MessageType::ContactRequest => {
                let card = decode_card_without_invitation(&mut reader)?;
                let invitation = if reader.presence()? {
                    Some(InvitationCapability::from_bytes(reader.array()?))
                } else {
                    None
                };
                let display_name = DisplayName::from_bytes(reader.bytes(0, MAX_DISPLAY_NAME_LEN)?)?;
                let introduction =
                    IntroductionText::from_bytes(reader.bytes(0, MAX_INTRODUCTION_TEXT_LEN)?)?;
                Self::ContactRequest(Box::new(ContactRequest {
                    card,
                    invitation,
                    display_name,
                    introduction,
                }))
            }
            MessageType::ContactAccept => Self::ContactAccept,
            MessageType::ChatMessage => {
                let id = MessageId::from_bytes(reader.array()?);
                let text = ChatText::from_bytes(reader.bytes(1, MAX_CHAT_TEXT_LEN)?)?;
                Self::ChatMessage { id, text }
            }
            MessageType::MessageAck => Self::MessageAck(MessageId::from_bytes(reader.array()?)),
            MessageType::Profile => {
                let display_name = DisplayName::from_bytes(reader.bytes(0, MAX_DISPLAY_NAME_LEN)?)?;
                let profile_text = ProfileText::from_bytes(reader.bytes(0, MAX_PROFILE_TEXT_LEN)?)?;
                Self::Profile {
                    display_name,
                    profile_text,
                }
            }
            MessageType::EndpointUpdate => {
                Self::EndpointUpdate(Box::new(decode_card_without_invitation(&mut reader)?))
            }
            MessageType::FileOffer => {
                let transfer = TransferId::from_bytes(reader.array()?);
                let size = reader.u64()?;
                if size > MAX_FILE_SIZE {
                    return Err(ProtocolError::InvalidValue);
                }
                let filename = Filename::from_bytes(reader.bytes(1, MAX_FILENAME_LEN)?)?;
                Self::FileOffer {
                    transfer,
                    size,
                    filename,
                }
            }
            MessageType::FileAccept => Self::FileAccept(TransferId::from_bytes(reader.array()?)),
            MessageType::FileReject => Self::FileReject(TransferId::from_bytes(reader.array()?)),
            MessageType::FileChunk => {
                let transfer = TransferId::from_bytes(reader.array()?);
                let data = reader.bytes(1, MAX_FILE_CHUNK_LEN)?.to_vec();
                Self::FileChunk(FileChunk { transfer, data })
            }
            MessageType::FileComplete => {
                let transfer = TransferId::from_bytes(reader.array()?);
                let digest = reader.array()?;
                Self::FileComplete { transfer, digest }
            }
            MessageType::FileAbort => Self::FileAbort(TransferId::from_bytes(reader.array()?)),
        };
        reader.finish()?;
        Ok(message)
    }
}

/// Reads a contact card that carries no invitation capability.
///
/// Such a card has a length that its own endpoint count determines. The
/// count is read first, without consuming anything, and checked against the
/// limit; only then is the card taken from the input and verified.
fn decode_card_without_invitation(reader: &mut Reader<'_>) -> Result<ContactCard, ProtocolError> {
    use crate::limits::{
        CONTACT_CARD_COUNT_OFFSET, CONTACT_CARD_ENDPOINT_LEN, CONTACT_CARD_FIXED_LEN,
        MAX_ACTIVE_ENDPOINTS,
    };

    let mut peek = Reader::new(reader.peek());
    peek.take(CONTACT_CARD_COUNT_OFFSET)?;
    let count = usize::from(peek.u8()?);
    if count == 0 || count > MAX_ACTIVE_ENDPOINTS {
        return Err(ProtocolError::InvalidValue);
    }
    let len = count
        .checked_mul(CONTACT_CARD_ENDPOINT_LEN)
        .and_then(|endpoints| endpoints.checked_add(CONTACT_CARD_FIXED_LEN))
        .ok_or(ProtocolError::BadMessageLength)?;
    let card = ContactCard::decode(reader.take(len)?)?;
    if card.invitation().is_some() {
        return Err(ProtocolError::InvalidValue);
    }
    Ok(card)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::EndpointSet;
    use crate::limits::MAX_UNCONFIRMED_BODY_LEN;
    use monolith_identity::{
        EndpointEpoch, IdentitySecretKey, OnionServiceKey, TransportPublicKey,
    };

    fn secret(seed: u8) -> IdentitySecretKey {
        IdentitySecretKey::from_seed(&[seed; 32])
    }

    fn card(invitation: bool) -> ContactCard {
        let endpoint = OnionServiceKey::from_bytes(secret(9).public_key().as_bytes()).unwrap();
        let mut transport = [1_u8; 32];
        transport[31] = 0x40;
        ContactCard::sign(
            &secret(1),
            TransportPublicKey::from_bytes(&transport).unwrap(),
            EndpointEpoch::new(3).unwrap(),
            EndpointSet::single(endpoint),
            invitation.then(|| InvitationCapability::from_bytes([7; 16])),
        )
        .unwrap()
    }

    fn transfer() -> TransferId {
        TransferId::from_bytes([0x33; 16])
    }

    fn id() -> MessageId {
        MessageId::from_bytes([0x44; 16])
    }

    /// One message of every type, at ordinary sizes.
    fn samples() -> Vec<Message> {
        vec![
            Message::Close,
            Message::Ping([1; 8]),
            Message::Pong([1; 8]),
            Message::ContactRequest(Box::new(ContactRequest {
                card: card(false),
                invitation: Some(InvitationCapability::from_bytes([5; 16])),
                display_name: DisplayName::new("Alice").unwrap(),
                introduction: IntroductionText::new("We met at the conference.").unwrap(),
            })),
            Message::ContactRequest(Box::new(ContactRequest {
                card: card(false),
                invitation: None,
                display_name: DisplayName::new("").unwrap(),
                introduction: IntroductionText::new("").unwrap(),
            })),
            Message::ContactAccept,
            Message::ChatMessage {
                id: id(),
                text: ChatText::new("hello\nworld").unwrap(),
            },
            Message::MessageAck(id()),
            Message::Profile {
                display_name: DisplayName::new("Alice").unwrap(),
                profile_text: ProfileText::new("away until Monday").unwrap(),
            },
            Message::EndpointUpdate(Box::new(card(false))),
            Message::FileOffer {
                transfer: transfer(),
                size: 12_345,
                filename: Filename::new("notes.txt").unwrap(),
            },
            Message::FileAccept(transfer()),
            Message::FileReject(transfer()),
            Message::FileChunk(FileChunk::new(transfer(), vec![0xAB; 100]).unwrap()),
            Message::FileComplete {
                transfer: transfer(),
                digest: [0x55; 32],
            },
            Message::FileAbort(transfer()),
        ]
    }

    #[test]
    fn samples_cover_every_message_type() {
        let covered: Vec<MessageType> = samples().iter().map(Message::message_type).collect();
        for message_type in MessageType::ALL {
            assert!(covered.contains(&message_type), "{message_type:?}");
        }
    }

    #[test]
    fn every_message_round_trips() {
        for message in samples() {
            let body = message.encode_body().unwrap();
            let decoded = Message::decode(message.message_type(), &body).unwrap();
            assert_eq!(decoded, message);
            assert_eq!(decoded.encode_body().unwrap(), body);
        }
    }

    #[test]
    fn body_sizes_match_the_specification() {
        let size = |message: &Message| message.encode_body().unwrap().len();
        let all = samples();
        assert_eq!(size(&Message::Close), 0);
        assert_eq!(size(&Message::Ping([0; 8])), 8);
        assert_eq!(size(&Message::ContactAccept), 0);
        assert_eq!(size(&Message::MessageAck(id())), 16);
        assert_eq!(size(&Message::EndpointUpdate(Box::new(card(false)))), 171);
        assert_eq!(size(&Message::FileAccept(transfer())), 16);
        assert_eq!(
            size(&Message::FileComplete {
                transfer: transfer(),
                digest: [0; 32]
            }),
            48
        );
        // Smallest and largest contact request.
        assert_eq!(size(&all[4]), 176);
        let largest = Message::ContactRequest(Box::new(ContactRequest {
            card: card(false),
            invitation: Some(InvitationCapability::from_bytes([5; 16])),
            display_name: DisplayName::new(&"a".repeat(64)).unwrap(),
            introduction: IntroductionText::new(&"b".repeat(512)).unwrap(),
        }));
        // 64 one-byte characters; the byte limit of 128 needs wider ones.
        assert_eq!(size(&largest), 832 - 64);
        let widest = Message::ContactRequest(Box::new(ContactRequest {
            card: card(false),
            invitation: Some(InvitationCapability::from_bytes([5; 16])),
            display_name: DisplayName::new(&"\u{e9}".repeat(64)).unwrap(),
            introduction: IntroductionText::new(&"b".repeat(512)).unwrap(),
        }));
        assert_eq!(size(&widest), MAX_UNCONFIRMED_BODY_LEN);
    }

    #[test]
    fn truncated_bodies_are_rejected() {
        for message in samples() {
            let body = message.encode_body().unwrap();
            if body.is_empty() {
                continue;
            }
            let short = &body[..body.len() - 1];
            assert!(
                Message::decode(message.message_type(), short).is_err(),
                "{:?}",
                message.message_type()
            );
        }
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        for message in samples() {
            let mut body = message.encode_body().unwrap();
            body.push(0);
            assert!(
                Message::decode(message.message_type(), &body).is_err(),
                "{:?}",
                message.message_type()
            );
        }
    }

    #[test]
    fn empty_messages_reject_any_body() {
        assert_eq!(
            Message::decode(MessageType::Close, &[0]),
            Err(ProtocolError::BadMessageLength)
        );
        assert_eq!(
            Message::decode(MessageType::ContactAccept, &[0]),
            Err(ProtocolError::BadMessageLength)
        );
    }

    #[test]
    fn chat_text_field_limits() {
        let mut body = vec![0x44; 16];
        body.extend_from_slice(&[0, 0]);
        assert_eq!(
            Message::decode(MessageType::ChatMessage, &body),
            Err(ProtocolError::InvalidValue)
        );

        // Declared length one over the maximum, with no data behind it. The
        // declared length alone is enough to reject.
        let mut body = vec![0x44; 16];
        body.extend_from_slice(&u16::try_from(MAX_CHAT_TEXT_LEN + 1).unwrap().to_be_bytes());
        assert_eq!(
            Message::decode(MessageType::ChatMessage, &body),
            Err(ProtocolError::FieldTooLong)
        );

        let longest = Message::ChatMessage {
            id: id(),
            text: ChatText::new(&"a".repeat(MAX_CHAT_TEXT_LEN)).unwrap(),
        };
        let body = longest.encode_body().unwrap();
        assert_eq!(body.len(), 16 + 2 + MAX_CHAT_TEXT_LEN);
        assert_eq!(
            Message::decode(MessageType::ChatMessage, &body),
            Ok(longest)
        );
    }

    #[test]
    fn invalid_text_in_a_body_is_rejected() {
        let mut body = vec![0x44; 16];
        body.extend_from_slice(&[0, 2, 0xff, 0xfe]);
        assert_eq!(
            Message::decode(MessageType::ChatMessage, &body),
            Err(ProtocolError::InvalidUtf8)
        );

        let mut body = vec![0x33; 16];
        body.extend_from_slice(&0_u64.to_be_bytes());
        body.extend_from_slice(&[0, 3]);
        body.extend_from_slice(b"a/b");
        assert_eq!(
            Message::decode(MessageType::FileOffer, &body),
            Err(ProtocolError::ForbiddenCharacter)
        );
    }

    #[test]
    fn file_offer_size_is_bounded() {
        let mut body = vec![0x33; 16];
        body.extend_from_slice(&(MAX_FILE_SIZE + 1).to_be_bytes());
        body.extend_from_slice(&[0, 1, b'a']);
        assert_eq!(
            Message::decode(MessageType::FileOffer, &body),
            Err(ProtocolError::InvalidValue)
        );

        let mut body = vec![0x33; 16];
        body.extend_from_slice(&MAX_FILE_SIZE.to_be_bytes());
        body.extend_from_slice(&[0, 1, b'a']);
        assert!(Message::decode(MessageType::FileOffer, &body).is_ok());

        let too_big = Message::FileOffer {
            transfer: transfer(),
            size: MAX_FILE_SIZE + 1,
            filename: Filename::new("a").unwrap(),
        };
        assert_eq!(too_big.encode_body(), Err(ProtocolError::InvalidValue));
    }

    #[test]
    fn file_chunk_limits() {
        assert_eq!(
            FileChunk::new(transfer(), Vec::new()),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(
            FileChunk::new(transfer(), vec![0; MAX_FILE_CHUNK_LEN + 1]),
            Err(ProtocolError::FieldTooLong)
        );
        let full =
            Message::FileChunk(FileChunk::new(transfer(), vec![9; MAX_FILE_CHUNK_LEN]).unwrap());
        let body = full.encode_body().unwrap();
        assert_eq!(body.len(), crate::limits::MAX_MESSAGE_BODY_LEN);
        assert_eq!(Message::decode(MessageType::FileChunk, &body), Ok(full));

        let mut empty = vec![0x33; 16];
        empty.extend_from_slice(&[0, 0]);
        assert_eq!(
            Message::decode(MessageType::FileChunk, &empty),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn presence_byte_must_be_zero_or_one() {
        let request = &samples()[4];
        let mut body = request.encode_body().unwrap();
        assert_eq!(body[171], 0);
        body[171] = 2;
        assert_eq!(
            Message::decode(MessageType::ContactRequest, &body),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn a_card_with_an_invitation_is_rejected_inside_messages() {
        let with_invitation = card(true);
        // The card with invitation is 16 bytes longer than the field. Its
        // first 171 bytes do not form a valid card.
        let mut body = with_invitation.encode();
        body.truncate(171);
        assert!(Message::decode(MessageType::EndpointUpdate, &body).is_err());
        assert!(Message::decode(MessageType::EndpointUpdate, &with_invitation.encode()).is_err());
        assert_eq!(
            Message::EndpointUpdate(Box::new(with_invitation)).encode_body(),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn debug_output_shows_no_content() {
        // Nothing but the type: no text, no nonce, no digest, no size, no
        // identifier.
        for message in samples() {
            assert_eq!(
                format!("{message:?}"),
                format!("Message({:?})", message.message_type())
            );
        }
        let request = samples().swap_remove(3);
        let Message::ContactRequest(request) = request else {
            panic!("sample 3 is a contact request");
        };
        let text = format!("{request:?}");
        assert!(!text.contains("Alice"));
        assert!(!text.contains("conference"));
        assert!(!text.contains("5, 5"));
        assert_eq!(
            format!("{:?}", FileChunk::new(transfer(), vec![0xAB; 4]).unwrap()),
            "FileChunk([redacted])"
        );
    }

    #[test]
    fn oracle_3_no_message_states_a_reason() {
        // T-ORACLE-3. The messages that exist before a session is confirmed
        // are Close, ContactRequest and ContactAccept. Close and
        // ContactAccept have empty bodies and accept no other, so they
        // cannot say why. A ContactRequest carries only its sender's own
        // card, name and introduction. There is no message that says
        // "blocked", "former contact" or "not in the contact list".
        use crate::SessionState;

        let unconfirmed: Vec<MessageType> = MessageType::ALL
            .into_iter()
            .filter(|message| message.may_be_received_in(SessionState::AuthenticatedUnknown))
            .collect();
        assert_eq!(
            unconfirmed,
            [
                MessageType::Close,
                MessageType::ContactRequest,
                MessageType::ContactAccept
            ]
        );

        for message in [Message::Close, Message::ContactAccept] {
            assert_eq!(message.encode_body().unwrap(), Vec::<u8>::new());
            for byte in 0..=255_u8 {
                assert!(Message::decode(message.message_type(), &[byte]).is_err());
            }
        }
    }
}
