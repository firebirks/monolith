//! The values of the runtime as the UI model names them, and back. Every
//! error of the runtime becomes one [`Failure`]; no `Debug` or `Display`
//! of an error goes further.

use core::time::Duration;

use monolith_runtime::OpenError;
use monolith_runtime::api::{
    self, CommandError, ContactId, DeferReason, Event, IdentityId, MessageRef, MessageStatus,
    SendError, ServiceState,
};
use monolith_ui_model::{
    Deferral, Failure, MessageView, RequestMode, RuntimeEvent, Service, Status, TorCheck,
    UiContactId, UiIdentityId, UiMessageId,
};

/// The handle of an identity.
pub(crate) fn identity(id: &IdentityId) -> UiIdentityId {
    UiIdentityId::from_bytes(*id.as_bytes())
}

/// The handle of a contact record of `owner`.
pub(crate) fn contact(owner: &IdentityId, record: &ContactId) -> UiContactId {
    UiContactId::new(identity(owner), *record.as_bytes())
}

/// The handle of a message.
pub(crate) fn message(reference: &MessageRef) -> UiMessageId {
    UiMessageId::new(
        contact(&reference.identity, &reference.contact),
        reference.sequence,
    )
}

/// The identifier of the identity a handle names. A handle the bridge
/// made always converts; one that does not names no identity.
pub(crate) fn identity_id(handle: UiIdentityId) -> Result<IdentityId, Failure> {
    IdentityId::from_bytes(handle.bytes()).map_err(|_| Failure::NoSuchIdentity)
}

/// The identity and the record a contact handle names.
pub(crate) fn contact_ids(handle: UiContactId) -> Result<(IdentityId, ContactId), Failure> {
    let owner = identity_id(handle.identity())?;
    let record = ContactId::from_bytes(handle.record()).map_err(|_| Failure::NoSuchContact)?;
    Ok((owner, record))
}

/// A message for the model.
pub(crate) fn message_view(view: api::MessageView) -> MessageView {
    MessageView {
        id: message(&view.reference),
        status: match view.status {
            MessageStatus::Queued => Status::Queued,
            MessageStatus::Delivered => Status::Delivered,
            MessageStatus::Received => Status::Received,
        },
        time: view.time,
        text: view.text,
    }
}

pub(crate) fn service(state: ServiceState) -> Service {
    match state {
        ServiceState::Publishing => Service::Publishing,
        ServiceState::Available => Service::Available,
        ServiceState::Unavailable => Service::Unavailable,
        ServiceState::NoKey => Service::NoKey,
        ServiceState::Stopped => Service::Stopped,
    }
}

pub(crate) fn request_mode(mode: api::RequestMode) -> RequestMode {
    match mode {
        api::RequestMode::Invitation => RequestMode::Invitation,
        api::RequestMode::Open => RequestMode::Open,
        api::RequestMode::Closed => RequestMode::Closed,
    }
}

pub(crate) fn runtime_request_mode(mode: RequestMode) -> api::RequestMode {
    match mode {
        RequestMode::Invitation => api::RequestMode::Invitation,
        RequestMode::Open => api::RequestMode::Open,
        RequestMode::Closed => api::RequestMode::Closed,
    }
}

pub(crate) fn tor_check(check: api::TorCheck) -> TorCheck {
    match check {
        api::TorCheck::Ready { version, outdated } => TorCheck::Ready { version, outdated },
        api::TorCheck::Connecting { progress } => TorCheck::Connecting { progress },
        api::TorCheck::NotRunning => TorCheck::NotRunning,
        api::TorCheck::NoControl => TorCheck::NoControl,
        api::TorCheck::SocksUnavailable => TorCheck::SocksUnavailable,
        api::TorCheck::ControlUnavailable => TorCheck::ControlUnavailable,
        api::TorCheck::AuthenticationFailed => TorCheck::AuthenticationFailed,
        api::TorCheck::TooOld { version } => TorCheck::TooOld { version },
    }
}

fn deferral(reason: DeferReason) -> Deferral {
    match reason {
        DeferReason::Tor => Deferral::Tor,
        DeferReason::NoEndpoint => Deferral::NoAddress,
        DeferReason::Budget => Deferral::Busy,
        DeferReason::AckTimeout => Deferral::NoAcknowledgement,
        // The handshake failed, the session ended with messages left, or a
        // reason of a later version: the contact was not reached.
        _ => Deferral::Unreachable,
    }
}

/// What an event says, for the model. Sessions coming and going are not
/// shown (no presence, `docs/DESIGN_QUESTIONS.md` section 13.2), so they
/// say nothing.
pub(crate) fn event(event: Event) -> Option<RuntimeEvent> {
    Some(match event {
        Event::MessageQueued(reference) => RuntimeEvent::MessageQueued(message(&reference)),
        Event::MessageReceived(reference) => RuntimeEvent::MessageReceived(message(&reference)),
        Event::MessageDelivered(reference) => RuntimeEvent::MessageDelivered(message(&reference)),
        Event::MessageRefused {
            identity: owner,
            contact: record,
            ..
        } => RuntimeEvent::MessageRefused(contact(&owner, &record)),
        Event::DeliveryDeferred {
            identity: owner,
            contact: record,
            reason,
            retry_in,
        } => RuntimeEvent::DeliveryDeferred {
            contact: contact(&owner, &record),
            reason: deferral(reason),
            retry_in: seconds(retry_in),
        },
        Event::SessionAvailable { .. } | Event::SessionUnavailable { .. } => return None,
        Event::Service {
            identity: owner,
            state,
        } => RuntimeEvent::Service {
            identity: identity(&owner),
            state: service(state),
        },
        Event::ContactAccepted {
            identity: owner,
            contact: record,
        } => RuntimeEvent::ContactAccepted(contact(&owner, &record)),
        Event::RequestReceived {
            identity: owner,
            queued,
        } => RuntimeEvent::RequestReceived {
            identity: identity(&owner),
            queued,
        },
        Event::IdentityClosed { identity: owner } => RuntimeEvent::IdentityClosed(identity(&owner)),
        Event::StorageFailed => RuntimeEvent::StorageFailed,
        // An event of a later version: read everything again.
        _ => RuntimeEvent::Missed,
    })
}

fn seconds(duration: Duration) -> u64 {
    // Rounded up, so that "retry in 0 s" is never said of a delay.
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
}

pub(crate) fn open_error(error: OpenError) -> Failure {
    match error {
        OpenError::AuthenticationFailed => Failure::WrongPassphrase,
        OpenError::BadPassphrase => Failure::UnusablePassphrase,
        OpenError::Locked => Failure::InUse,
        OpenError::Permissions => Failure::UnsafePermissions,
        OpenError::RecoveryNeeded => Failure::RecoveryNeeded,
        OpenError::Invalid => Failure::Damaged,
        OpenError::MessageStore => Failure::MessageStoreMismatch,
        OpenError::Io => Failure::Disk,
        // Not found where one was seen, or found where none was: the
        // directory changed under the app.
        _ => Failure::Unexpected,
    }
}

pub(crate) fn command_error(error: CommandError) -> Failure {
    match error {
        CommandError::UnknownIdentity => Failure::NoSuchIdentity,
        CommandError::UnknownContact => Failure::NoSuchContact,
        CommandError::InvalidCard => Failure::InvalidCard,
        CommandError::InvalidText => Failure::InvalidText,
        CommandError::Full => Failure::Full,
        CommandError::Blocked => Failure::Blocked,
        CommandError::OwnIdentity => Failure::OwnCard,
        CommandError::Tor => Failure::Tor,
        CommandError::Storage => Failure::Storage,
        CommandError::ShuttingDown => Failure::ShuttingDown,
        CommandError::UnknownRequest => Failure::NoSuchRequest,
        CommandError::UnknownInvitation => Failure::NoSuchInvitation,
        CommandError::NotBlocked => Failure::NotBlocked,
        _ => Failure::Unexpected,
    }
}

pub(crate) fn send_error(error: SendError) -> Failure {
    match error {
        SendError::InvalidText => Failure::InvalidText,
        SendError::TooLong => Failure::TooLong,
        SendError::UnknownIdentity => Failure::NoSuchIdentity,
        SendError::NotAContact => Failure::NotAContact,
        SendError::QueueFull => Failure::QueueFull,
        SendError::StoreFull => Failure::StoreFull,
        SendError::Storage => Failure::Storage,
        SendError::ShuttingDown => Failure::ShuttingDown,
        // Said where it was met, unlike a failed store, which the window
        // says once from its event.
        _ => Failure::Unexpected,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn handles_round_trip_to_the_identifiers_they_were_made_from() {
        let owner = IdentityId::from_bytes([7; 16]).unwrap();
        let record = ContactId::from_bytes([9; 16]).unwrap();
        let handle = contact(&owner, &record);
        assert_eq!(handle.identity(), identity(&owner));
        assert_eq!(contact_ids(handle).unwrap(), (owner, record));
        assert_eq!(identity_id(identity(&owner)).unwrap(), owner);
    }

    #[test]
    fn a_delay_below_a_second_is_never_said_to_be_none() {
        assert_eq!(seconds(Duration::from_millis(1)), 1);
        assert_eq!(seconds(Duration::from_secs(30)), 30);
        assert_eq!(seconds(Duration::from_millis(30_500)), 31);
    }

    #[test]
    fn a_failed_send_is_said_where_it_was_met_unless_the_store_failed() {
        assert_eq!(send_error(SendError::ShuttingDown), Failure::ShuttingDown);
        assert_eq!(send_error(SendError::Randomness), Failure::Unexpected);
        assert_eq!(send_error(SendError::Storage), Failure::Storage);
        for error in [SendError::ShuttingDown, SendError::Randomness] {
            assert!(send_error(error).said_in_place(), "{error:?}");
        }
    }

    #[test]
    fn presence_is_not_forwarded() {
        let owner = IdentityId::from_bytes([1; 16]).unwrap();
        let record = ContactId::from_bytes([2; 16]).unwrap();
        assert_eq!(
            event(Event::SessionAvailable {
                identity: owner,
                contact: record
            }),
            None
        );
        assert_eq!(
            event(Event::SessionUnavailable {
                identity: owner,
                contact: record
            }),
            None
        );
    }
}
