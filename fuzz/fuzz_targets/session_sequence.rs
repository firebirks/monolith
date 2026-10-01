//! Session logic: an arbitrary sequence of message types against a session
//! of arbitrary standing never produces application data before the session
//! is confirmed, never moves a session backwards, and treats every identity
//! that is not a contact alike.
//!
//! The first input byte selects how far the session got before the messages
//! arrive, the second the standing of the peer. Every further byte is one
//! message type.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::session::{Action, Session, Standing};
use monolith_protocol::{MessageType, SessionState};

const STANDINGS: [Standing; 5] = [
    Standing::None,
    Standing::Declined,
    Standing::Blocked,
    Standing::Requested,
    Standing::Accepted,
];

fn start(steps: u8, standing: Standing) -> Session {
    let mut session = Session::new();
    if steps >= 1 {
        let _ = session.stream_established();
    }
    if steps >= 2 {
        let _ = session.handshake_completed();
    }
    if steps >= 3 {
        let _ = session.identities_proven(standing);
    }
    session
}

/// What the peer can observe: the visible actions per message and where the
/// session ends up.
fn observe(
    steps: u8,
    standing: Standing,
    messages: &[MessageType],
) -> (Vec<Vec<Action>>, bool, SessionState) {
    let mut session = start(steps, standing);
    let mut seen = Vec::new();
    let mut failed = false;
    for message in messages {
        match session.receive(*message) {
            Ok(actions) => seen.push(
                actions
                    .into_iter()
                    .filter(|action| action.is_visible_to_peer())
                    .collect(),
            ),
            Err(_) => {
                failed = true;
                break;
            }
        }
    }
    (seen, failed, session.state())
}

fuzz_target!(|data: &[u8]| {
    let [steps, standing_selector, rest @ ..] = data else {
        return;
    };
    let steps = steps % 4;
    let standing = STANDINGS[usize::from(*standing_selector) % STANDINGS.len()];
    let types = MessageType::ALL;
    let messages: Vec<MessageType> = rest
        .iter()
        .map(|byte| types[usize::from(*byte) % types.len()])
        .collect();

    let mut session = start(steps, standing);
    for message in &messages {
        let before = session.state();
        let result = session.receive(*message);
        let after = session.state();
        assert!(before == after || before.can_transition_to(after));
        match result {
            Ok(actions) => {
                if actions.contains(&Action::Deliver) {
                    assert_eq!(before, SessionState::AuthenticatedContact);
                }
                if actions.contains(&Action::MarkAccepted) {
                    assert_eq!(standing, Standing::Requested);
                }
                if actions.contains(&Action::ConsiderRequest) {
                    assert_eq!(standing, Standing::None);
                    assert_eq!(before, SessionState::AuthenticatedUnknown);
                }
            }
            Err(_) => assert_eq!(after, SessionState::Closed),
        }
    }

    // Identities that are not contacts are indistinguishable to the peer.
    let reference = observe(steps, Standing::None, &messages);
    assert_eq!(observe(steps, Standing::Declined, &messages), reference);
    assert_eq!(observe(steps, Standing::Blocked, &messages), reference);
});
