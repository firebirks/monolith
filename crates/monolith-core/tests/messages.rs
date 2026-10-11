//! The messages of a local identity, step by step over links
//! (`docs/adr/0007-delivery-semantics.md`, `docs/adr/0008-message-store.md`):
//! what is queued, taken in, acknowledged and delivered, bound to which
//! conversation, and what a crash at each boundary leaves.
//!
//! The steps a runtime takes are taken here one at a time, so that a crash,
//! a held write or a concurrent duplicate lands exactly where a test puts
//! it.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::{
    Node, Released, befriend, both, confirm_both, fixed_endpoint, held, keys, node, node_in, run,
};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, InstallationError, LocalIdentity, MessageError};
use monolith_core::link::Link;
use monolith_core::messages::{Receipt, Refusal};
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::limits::{MAX_QUEUED_MESSAGES_PER_CONTACT, MAX_VAULT_FILE_LEN};
use monolith_protocol::text::ChatText;
use monolith_storage::StorageError;
use monolith_storage::dir::{CrashOutcome, MESSAGES, MemoryDir, VAULT, VaultDir};
use monolith_storage::message_records::EntryKind;
use monolith_storage::record::{
    ContactId, Contents, PayloadVersion, StoredContact, StoredIdentity,
};
use monolith_storage::vault::{KdfParams, Passphrase, Vault};
use monolith_tor::{KeySource, MockNetwork, OnionServiceSecret, TorBackend};
use tokio::io::DuplexStream;
use zeroize::Zeroizing;

fn passphrase() -> Passphrase {
    Passphrase::new("correct horse battery staple").unwrap()
}

fn text(text: &str) -> ChatText {
    ChatText::new(text).unwrap()
}

/// The record of `peer` at `node`.
fn record(node: &Node, peer: &Node) -> ContactId {
    node.identity
        .contact_id(peer.identity.identity())
        .expect("a contact record")
}

/// The kinds of the messages of a conversation, oldest first.
fn kinds(identity: &LocalIdentity, contact: &ContactId) -> Vec<EntryKind> {
    identity
        .conversation(contact, None, 1000)
        .into_iter()
        .map(|entry| entry.kind)
        .collect()
}

/// A node of the identity of `seed` in a persistent installation on
/// `dir`, published at its fixed endpoint.
async fn persistent(network: &MockNetwork, dir: &MemoryDir, seed: u8) -> Node {
    let installation =
        Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
    common::node_fixed(installation, network, seed, seed, 1, fixed_endpoint(seed)).await
}

/// The node of the identity of `seed`, from the installation opened again
/// on what a crash of `dir` left, published again.
async fn reopened(network: &MockNetwork, dir: &MemoryDir, seed: u8, outcome: CrashOutcome) -> Node {
    let (installation, _) =
        Installation::open(Box::new(dir.restart(outcome)), &passphrase()).unwrap();
    let identity = installation.identities().into_iter().next().unwrap();
    let tor = network.backend();
    let service = tor
        .publish_onion(KeySource::Existing {
            secret: OnionServiceSecret::from_bytes(&[seed; 64]),
            expected: identity.endpoint(),
        })
        .await
        .unwrap();
    Node {
        installation,
        identity,
        tor,
        service,
        budgets: Budgets::new(),
    }
}

/// Sends the head of the queue of `contact` at `from` on `link`. Returns
/// its identifier.
async fn send_head(from: &Node, contact: &ContactId, link: &mut Link<DuplexStream>) -> MessageId {
    let (_, id, text) = from
        .identity
        .next_to_send(contact)
        .await
        .unwrap()
        .expect("a queued message");
    link.send(&Message::ChatMessage { id, text }).await.unwrap();
    id
}

/// Receives a chat message on `link` and takes it in at `node`.
async fn take_in(
    node: &Node,
    link: &mut Link<DuplexStream>,
) -> Result<(MessageId, Receipt), MessageError> {
    let arrived = link.receive().await.unwrap();
    node.identity
        .take_in(&arrived)
        .await
        .map(|taken| (taken.id, taken.receipt))
}

/// Receives a MessageAck on `link` and applies it at `node`.
async fn acknowledged(node: &Node, link: &mut Link<DuplexStream>) -> Option<(ContactId, u64)> {
    let arrived = link.receive().await.unwrap();
    node.identity.acknowledged(&arrived).await.unwrap()
}

#[test]
fn a_message_is_stored_once_and_delivered_by_its_acknowledgement() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let (at_a, at_b) = (record(&a, &b), record(&b, &a));
        let queued = a.identity.queue_message(at_a, text("hello")).await.unwrap();
        assert_eq!(kinds(&a.identity, &at_a), vec![EntryKind::Queued]);
        let (mut a_link, mut b_link) = confirm_both(&a, &mut b).await;
        let id = send_head(&a, &at_a, &mut a_link).await;
        assert_eq!(id, queued.id);
        // A written frame is not delivery.
        assert_eq!(kinds(&a.identity, &at_a), vec![EntryKind::Queued]);
        let (taken, receipt) = take_in(&b, &mut b_link).await.unwrap();
        assert_eq!(taken, id);
        assert!(matches!(receipt, Receipt::Stored(_)));
        assert_eq!(kinds(&b.identity, &at_b), vec![EntryKind::Received]);
        // The same message again: held already, acknowledged again.
        send_head(&a, &at_a, &mut a_link).await;
        assert_eq!(
            take_in(&b, &mut b_link).await.unwrap().1,
            Receipt::Duplicate
        );
        assert_eq!(kinds(&b.identity, &at_b), vec![EntryKind::Received]);
        b_link.send(&Message::MessageAck(id)).await.unwrap();
        assert_eq!(
            acknowledged(&a, &mut a_link).await,
            Some((at_a, queued.sequence))
        );
        assert_eq!(kinds(&a.identity, &at_a), vec![EntryKind::Delivered]);
        assert!(a.identity.next_queued(&at_a).is_none());
        // A second acknowledgement changes nothing.
        b_link.send(&Message::MessageAck(id)).await.unwrap();
        assert_eq!(acknowledged(&a, &mut a_link).await, None);
        // An identifier never queued changes nothing.
        b_link
            .send(&Message::MessageAck(MessageId::from_bytes([9; 16])))
            .await
            .unwrap();
        assert_eq!(acknowledged(&a, &mut a_link).await, None);
    });
}

#[test]
fn the_queue_is_first_in_first_out() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        let first = a.identity.queue_message(at_a, text("one")).await.unwrap();
        let second = a.identity.queue_message(at_a, text("two")).await.unwrap();
        assert!(first.sequence < second.sequence);
        assert_eq!(a.identity.next_queued(&at_a).unwrap().1, first.id);
        let (mut a_link, mut b_link) = confirm_both(&a, &mut b).await;
        // An acknowledgement of the second first: it is delivered, and the
        // first is still the head.
        b_link.send(&Message::MessageAck(second.id)).await.unwrap();
        acknowledged(&a, &mut a_link).await;
        assert_eq!(a.identity.next_queued(&at_a).unwrap().1, first.id);
        assert_eq!(
            kinds(&a.identity, &at_a),
            vec![EntryKind::Queued, EntryKind::Delivered]
        );
    });
}

#[test]
fn a_message_is_queued_only_for_a_current_contact_record() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        let c = node(&network, 3).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        // A record of another identity is not one of this identity.
        let other = record(&b, &a);
        assert_eq!(
            a.identity.queue_message(other, text("x")).await.err(),
            Some(MessageError::NotAContact)
        );
        // A stranger has no record.
        assert!(a.identity.contact_id(c.identity.identity()).is_none());
        // A deleted record takes nothing more.
        a.identity.delete(b.identity.identity()).await.unwrap();
        assert_eq!(
            a.identity.queue_message(at_a, text("x")).await.err(),
            Some(MessageError::NotAContact)
        );
    });
}

#[test]
fn an_acknowledgement_reaches_only_the_conversation_of_its_session() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        let mut c = node(&network, 3).await;
        befriend(&a, &mut b).await;
        befriend(&a, &mut c).await;
        let to_b = record(&a, &b);
        let queued = a.identity.queue_message(to_b, text("for b")).await.unwrap();
        // C learned the identifier somehow and acknowledges it on its own
        // session: nothing of B's conversation changes.
        let (mut a_with_c, mut c_link) = confirm_both(&a, &mut c).await;
        c_link.send(&Message::MessageAck(queued.id)).await.unwrap();
        assert_eq!(acknowledged(&a, &mut a_with_c).await, None);
        assert_eq!(kinds(&a.identity, &to_b), vec![EntryKind::Queued]);
        // A chat message of C goes to C's conversation and nowhere else.
        c_link
            .send(&Message::ChatMessage {
                id: queued.id,
                text: text("from c"),
            })
            .await
            .unwrap();
        let (_, receipt) = take_in(&a, &mut a_with_c).await.unwrap();
        assert!(matches!(receipt, Receipt::Stored(_)));
        assert_eq!(kinds(&a.identity, &to_b), vec![EntryKind::Queued]);
        assert_eq!(
            kinds(&a.identity, &record(&a, &c)),
            vec![EntryKind::Received]
        );
    });
}

#[test]
fn the_identities_of_one_installation_keep_their_messages_apart() {
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let mut x = node_in(installation.clone(), &network, 1, 1, 1).await;
        let y = node_in(installation.clone(), &network, 4, 4, 1).await;
        let mut p = node(&network, 2).await;
        befriend(&x, &mut p).await;
        befriend(&y, &mut p).await;
        let x_to_p = record(&x, &p);
        let y_to_p = record(&y, &p);
        assert_ne!(x_to_p, y_to_p);
        let of_y = y.identity.queue_message(y_to_p, text("y")).await.unwrap();
        // P acknowledges Y's message on its session with X: nothing.
        let (mut p_link, mut x_link) = confirm_both(&p, &mut x).await;
        p_link.send(&Message::MessageAck(of_y.id)).await.unwrap();
        assert_eq!(acknowledged(&x, &mut x_link).await, None);
        assert_eq!(kinds(&y.identity, &y_to_p), vec![EntryKind::Queued]);
        // A message P sends on its session with X is X's alone.
        p_link
            .send(&Message::ChatMessage {
                id: MessageId::from_bytes([5; 16]),
                text: text("to x"),
            })
            .await
            .unwrap();
        take_in(&x, &mut x_link).await.unwrap();
        assert_eq!(kinds(&x.identity, &x_to_p), vec![EntryKind::Received]);
        assert_eq!(kinds(&y.identity, &y_to_p), vec![EntryKind::Queued]);
        assert!(
            y.identity
                .conversations()
                .iter()
                .all(|view| view.contact == y_to_p)
        );
    });
}

#[test]
fn a_contact_made_again_does_not_inherit_the_queue_of_the_old_record() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let old = record(&a, &b);
        a.identity
            .queue_message(old, text("for the old record"))
            .await
            .unwrap();
        a.identity.delete(b.identity.identity()).await.unwrap();
        // The old conversation is ended: its queue is kept and never sent.
        assert!(a.identity.current_record(&old).is_none());
        assert_eq!(kinds(&a.identity, &old), vec![EntryKind::Queued]);
        // The same identity made a contact again is another record.
        befriend(&a, &mut b).await;
        let new = record(&a, &b);
        assert_ne!(new, old);
        assert!(a.identity.next_queued(&new).is_none());
        assert!(a.identity.conversation(&new, None, 10).is_empty());
        // A session of the new record carries nothing of the old one.
        let (_a_link, _b_link) = confirm_both(&a, &mut b).await;
        assert!(a.identity.next_queued(&new).is_none());
        assert_eq!(kinds(&a.identity, &old), vec![EntryKind::Queued]);
    });
}

#[test]
fn blocking_ends_the_record_and_keeps_its_queue_without_sending_it() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        let (a_link, _b_link) = confirm_both(&a, &mut b).await;
        a.identity
            .queue_message(at_a, text("queued"))
            .await
            .unwrap();
        a.identity.block(b.identity.identity()).await.unwrap();
        // The session was withdrawn: nothing more goes out on it.
        assert!(a_link.is_withdrawn());
        assert!(a.identity.current_record(&at_a).is_none());
        assert_eq!(
            a.identity.queue_message(at_a, text("more")).await.err(),
            Some(MessageError::NotAContact)
        );
        assert_eq!(kinds(&a.identity, &at_a), vec![EntryKind::Queued]);
        // Unblocked and made a contact again: another record, no queue.
        a.identity.unblock(b.identity.identity()).await.unwrap();
        befriend(&a, &mut b).await;
        assert!(a.identity.next_queued(&record(&a, &b)).is_none());
    });
}

#[test]
fn a_new_card_of_the_same_contact_keeps_its_record_and_its_queue() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        let queued = a.identity.queue_message(at_a, text("kept")).await.unwrap();
        // B's newer card of the same key, imported by the user: the
        // credentials change, the record is the same one.
        let newer = common::node_with(&network, 2, 2, 2).await;
        a.identity.import(&newer.card()).await.unwrap();
        assert_eq!(record(&a, &b), at_a);
        assert_eq!(a.identity.next_queued(&at_a).unwrap().1, queued.id);
    });
}

#[test]
fn concurrent_copies_of_one_message_are_stored_once() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        a.identity.queue_message(at_a, text("once")).await.unwrap();
        // Two sessions at once, the same message on each.
        let (mut a_one, mut b_one) = confirm_both(&a, &mut b).await;
        let (mut a_two, mut b_two) = confirm_both(&a, &mut b).await;
        send_head(&a, &at_a, &mut a_one).await;
        send_head(&a, &at_a, &mut a_two).await;
        let (one, two) = both(take_in(&b, &mut b_one), take_in(&b, &mut b_two)).await;
        let receipts = [one.unwrap().1, two.unwrap().1];
        assert_eq!(
            receipts
                .iter()
                .filter(|receipt| matches!(receipt, Receipt::Stored(_)))
                .count(),
            1,
            "{receipts:?}"
        );
        assert_eq!(
            kinds(&b.identity, &record(&b, &a)),
            vec![EntryKind::Received]
        );
    });
}

#[test]
fn a_queue_holds_its_limit_and_refuses_one_more() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        for n in 0..MAX_QUEUED_MESSAGES_PER_CONTACT {
            a.identity
                .queue_message(at_a, text(&format!("m{n}")))
                .await
                .unwrap();
        }
        assert_eq!(
            a.identity.queue_message(at_a, text("one more")).await.err(),
            Some(MessageError::Refused(Refusal::ContactQueueFull))
        );
        assert_eq!(
            a.identity.conversation(&at_a, None, 1000).len(),
            MAX_QUEUED_MESSAGES_PER_CONTACT
        );
    });
}

#[test]
fn a_queued_message_is_reported_only_once_it_is_durable() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let a = persistent(&network, &dir, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        dir.hold_writes();
        let _released = Released(&dir);
        let crashed = async {
            held(&dir).await;
            dir.restart(CrashOutcome::ALL[0])
        };
        let after = match race(crashed, a.identity.queue_message(at_a, text("held"))).await {
            Ok(after) => after,
            Err(queued) => panic!("reported before its write: {queued:?}"),
        };
        drop(a);
        let (installation, _) = Installation::open(Box::new(after), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        assert!(identity.conversation(&at_a, None, 10).is_empty());
    });
}

#[test]
fn a_queued_message_is_offered_for_sending_only_once_it_is_durable() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let a = persistent(&network, &dir, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        dir.hold_writes();
        let released = Released(&dir);
        let identity = a.identity.clone();
        let queueing =
            tokio::spawn(async move { identity.queue_message(at_a, text("held")).await });
        held(&dir).await;
        // In memory, and its write held: there is a message, and none to
        // send yet.
        assert!(a.identity.next_queued(&at_a).is_some());
        let waited = race(
            tokio::time::sleep(core::time::Duration::from_millis(200)),
            a.identity.next_to_send(&at_a),
        )
        .await;
        assert!(waited.is_ok(), "offered before its write: {waited:?}");
        // A crash now loses it, and nothing was offered to send.
        let after = dir.restart(CrashOutcome::ALL[0]);
        drop(released);
        let _ = queueing.await;
        drop(a);
        let (installation, _) = Installation::open(Box::new(after), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        assert!(identity.conversation(&at_a, None, 10).is_empty());
    });
}

#[test]
fn what_arrived_on_a_link_that_ended_counts_for_nothing() {
    run(async {
        let network = MockNetwork::new();
        let a = node(&network, 1).await;
        let mut b = node(&network, 2).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        let queued = a.identity.queue_message(at_a, text("hello")).await.unwrap();
        let (mut a_link, mut b_link) = confirm_both(&a, &mut b).await;
        b_link.send(&Message::MessageAck(queued.id)).await.unwrap();
        b_link
            .send(&Message::ChatMessage {
                id: MessageId::from_bytes([7; 16]),
                text: text("late"),
            })
            .await
            .unwrap();
        let acknowledgement = a_link.receive().await.unwrap();
        let message = a_link.receive().await.unwrap();
        // The link ends before what it brought is applied.
        drop(a_link);
        assert_eq!(
            a.identity.acknowledged(&acknowledgement).await.err(),
            Some(MessageError::NotStanding)
        );
        assert_eq!(
            a.identity.take_in(&message).await.err(),
            Some(MessageError::NotStanding)
        );
        assert_eq!(kinds(&a.identity, &at_a), vec![EntryKind::Queued]);
    });
}

// --- Crash boundaries ---------------------------------------------------------

/// A persistent A and an ephemeral B, contacts of each other, and a message
/// queued at A.
async fn queued_pair(network: &MockNetwork, dir: &MemoryDir) -> (Node, Node, MessageId) {
    let a = persistent(network, dir, 1).await;
    let mut b = node(network, 2).await;
    befriend(&a, &mut b).await;
    let queued = a
        .identity
        .queue_message(record(&a, &b), text("hello"))
        .await
        .unwrap();
    (a, b, queued.id)
}

#[test]
fn a_sender_that_crashes_before_sending_finds_the_message_queued() {
    run(async {
        for outcome in CrashOutcome::ALL {
            let network = MockNetwork::new();
            let dir = MemoryDir::new();
            let (a, b, id) = queued_pair(&network, &dir).await;
            drop(a);
            let a = reopened(&network, &dir, 1, outcome).await;
            let at_a = record(&a, &b);
            assert_eq!(a.identity.next_queued(&at_a).unwrap().1, id, "{outcome:?}");
        }
    });
}

#[test]
fn a_sender_that_wrote_and_got_no_acknowledgement_sends_again_after_a_restart() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let (a, mut b, id) = queued_pair(&network, &dir).await;
        let at_b = record(&b, &a);
        {
            let (mut a_link, mut b_link) = confirm_both(&a, &mut b).await;
            send_head(&a, &record(&a, &b), &mut a_link).await;
            // B stores it; its acknowledgement is lost with A.
            assert!(matches!(
                take_in(&b, &mut b_link).await.unwrap().1,
                Receipt::Stored(_)
            ));
        }
        drop(a);
        let mut a = reopened(&network, &dir, 1, CrashOutcome::ALL[1]).await;
        let at_a = record(&a, &b);
        assert_eq!(kinds(&a.identity, &at_a), vec![EntryKind::Queued]);
        let (mut b_link, mut a_link) = confirm_both(&b, &mut a).await;
        assert_eq!(send_head(&a, &at_a, &mut a_link).await, id);
        assert_eq!(
            take_in(&b, &mut b_link).await.unwrap().1,
            Receipt::Duplicate
        );
        b_link.send(&Message::MessageAck(id)).await.unwrap();
        assert!(acknowledged(&a, &mut a_link).await.is_some());
        assert_eq!(kinds(&a.identity, &at_a), vec![EntryKind::Delivered]);
        assert_eq!(kinds(&b.identity, &at_b), vec![EntryKind::Received]);
    });
}

#[test]
fn a_receiver_that_crashes_before_the_message_is_durable_stores_nothing_and_acknowledges_nothing() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let mut b = persistent(&network, &dir, 2).await;
        let a = node(&network, 1).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        let queued = a.identity.queue_message(at_a, text("hello")).await.unwrap();
        let after = {
            let (mut a_link, mut b_link) = confirm_both(&a, &mut b).await;
            send_head(&a, &at_a, &mut a_link).await;
            let arrived = b_link.receive().await.unwrap();
            // The write of the message is held: B crashes before it is
            // durable. Nothing was acknowledged.
            dir.hold_writes();
            let _released = Released(&dir);
            let crashed = async {
                held(&dir).await;
                dir.restart(CrashOutcome::ALL[0])
            };
            match race(crashed, b.identity.take_in(&arrived)).await {
                Ok(after) => after,
                Err(taken) => panic!("taken in before its write: {taken:?}"),
            }
        };
        drop(b);
        let (installation, _) = Installation::open(Box::new(after), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        let at_b = identity.contact_id(a.identity.identity()).unwrap();
        assert!(identity.conversation(&at_b, None, 10).is_empty());
        assert_eq!(a.identity.next_queued(&at_a).unwrap().1, queued.id);
    });
}

#[test]
fn a_receiver_that_crashes_after_storing_and_before_acknowledging_drops_the_retry() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let mut b = persistent(&network, &dir, 2).await;
        let a = node(&network, 1).await;
        befriend(&a, &mut b).await;
        let at_a = record(&a, &b);
        let queued = a.identity.queue_message(at_a, text("hello")).await.unwrap();
        {
            let (mut a_link, mut b_link) = confirm_both(&a, &mut b).await;
            send_head(&a, &at_a, &mut a_link).await;
            assert!(matches!(
                take_in(&b, &mut b_link).await.unwrap().1,
                Receipt::Stored(_)
            ));
        }
        drop(b);
        for outcome in CrashOutcome::ALL {
            let mut b = reopened(&network, &dir, 2, outcome).await;
            let at_b = record(&b, &a);
            assert_eq!(
                kinds(&b.identity, &at_b),
                vec![EntryKind::Received],
                "{outcome:?}"
            );
            let (mut a_link, mut b_link) = confirm_both(&a, &mut b).await;
            send_head(&a, &at_a, &mut a_link).await;
            assert_eq!(
                take_in(&b, &mut b_link).await.unwrap().1,
                Receipt::Duplicate
            );
            assert_eq!(kinds(&b.identity, &at_b), vec![EntryKind::Received]);
            drop((a_link, b_link));
            let _ = queued;
        }
    });
}

#[test]
fn a_sender_that_crashes_before_the_delivery_is_durable_sends_again() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let (a, mut b, id) = queued_pair(&network, &dir).await;
        let at_a = record(&a, &b);
        let after = {
            let (mut a_link, mut b_link) = confirm_both(&a, &mut b).await;
            send_head(&a, &at_a, &mut a_link).await;
            take_in(&b, &mut b_link).await.unwrap();
            b_link.send(&Message::MessageAck(id)).await.unwrap();
            let arrived = a_link.receive().await.unwrap();
            // The acknowledgement is applied; its write is held, and A
            // crashes before it is durable.
            dir.hold_writes();
            let _released = Released(&dir);
            let crashed = async {
                held(&dir).await;
                dir.restart(CrashOutcome::ALL[0])
            };
            match race(crashed, a.identity.acknowledged(&arrived)).await {
                Ok(after) => after,
                Err(applied) => panic!("durable before its write: {applied:?}"),
            }
        };
        drop(a);
        let (installation, _) = Installation::open(Box::new(after), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        let at_a = identity.contact_id(b.identity.identity()).unwrap();
        assert_eq!(identity.next_queued(&at_a).unwrap().1, id);
    });
}

#[test]
fn a_delivered_message_stays_delivered_after_a_restart() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let (a, mut b, id) = queued_pair(&network, &dir).await;
        let at_a = record(&a, &b);
        let second = a.identity.queue_message(at_a, text("next")).await.unwrap();
        {
            let (mut a_link, mut b_link) = confirm_both(&a, &mut b).await;
            send_head(&a, &at_a, &mut a_link).await;
            take_in(&b, &mut b_link).await.unwrap();
            b_link.send(&Message::MessageAck(id)).await.unwrap();
            assert!(acknowledged(&a, &mut a_link).await.is_some());
        }
        drop(a);
        for outcome in CrashOutcome::ALL {
            let a = reopened(&network, &dir, 1, outcome).await;
            let at_a = record(&a, &b);
            assert_eq!(
                kinds(&a.identity, &at_a),
                vec![EntryKind::Delivered, EntryKind::Queued],
                "{outcome:?}"
            );
            assert_eq!(a.identity.next_queued(&at_a).unwrap().1, second.id);
        }
    });
}

#[test]
fn a_crash_at_any_step_of_queueing_leaves_the_message_or_nothing() {
    run(async {
        let network = MockNetwork::new();
        let base = MemoryDir::new();
        let (a, b, _) = queued_pair(&network, &base).await;
        let peer = *b.identity.identity();
        drop(a);
        let mut step = 0;
        loop {
            let dir = base.restart(CrashOutcome::ALL[0]);
            let (installation, _) =
                Installation::open(Box::new(dir.clone()), &passphrase()).unwrap();
            let identity = installation.identities().into_iter().next().unwrap();
            let at_a = identity.contact_id(&peer).unwrap();
            dir.crash_at(dir.steps() + step);
            let queued = identity.queue_message(at_a, text("second")).await;
            let crashed = dir.crashed();
            drop((identity, installation));
            for outcome in CrashOutcome::ALL {
                let (installation, _) =
                    Installation::open(Box::new(dir.restart(outcome)), &passphrase()).unwrap();
                let identity = installation.identities().into_iter().next().unwrap();
                let held = identity.conversation(&at_a, None, 10).len();
                assert!(held == 1 || held == 2, "{step} {outcome:?} {held}");
                if queued.is_ok() && !crashed {
                    assert_eq!(held, 2, "{step} {outcome:?}");
                }
            }
            if queued.is_ok() && !crashed {
                break;
            }
            step += 1;
        }
    });
}

// --- Opening ------------------------------------------------------------------

#[test]
fn a_message_store_that_does_not_authenticate_fails_the_installation() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let (a, _b, _) = queued_pair(&network, &dir).await;
        drop(a);
        let file = dir.read(MESSAGES, MAX_VAULT_FILE_LEN).unwrap().unwrap();
        let mut changed = file.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        dir.overwrite(MESSAGES, &changed).unwrap();
        assert_eq!(
            Installation::open(Box::new(dir.clone()), &passphrase()).err(),
            Some(InstallationError::Storage(
                StorageError::AuthenticationFailed
            ))
        );
        // Nothing was repaired: the file is as it was left.
        assert_eq!(
            dir.read(MESSAGES, MAX_VAULT_FILE_LEN).unwrap().unwrap(),
            changed
        );
    });
}

/// Writes in `dir` a vault of Phase 4, payload version 1, with one
/// identity of seed 1 that holds `b` as an accepted contact, and no message
/// store.
fn phase_4_vault(dir: &MemoryDir, b: &Node) {
    let stored = StoredIdentity {
        id: None,
        identity_seed: Zeroizing::new([1; 32]),
        transport_secret: Zeroizing::new([1 ^ 0xA5; 32]),
        onion_secret: Some(Zeroizing::new([1; 64])),
        epoch: monolith_identity::EndpointEpoch::FIRST,
        endpoint: fixed_endpoint(1),
        rotation: None,
        request_mode: monolith_protocol::contact::RequestMode::Invitation,
        label: None,
        invitations: Vec::new(),
        contacts: vec![StoredContact {
            id: None,
            kind: monolith_protocol::contact::RecordKind::Accepted,
            credentials: monolith_protocol::credential::Credentials::new(b.card()),
            dial: b.card(),
            verified: false,
            alias: None,
            successor_announced: false,
            successor_promoted: false,
        }],
        blocked: Vec::new(),
        declined: Vec::new(),
    };
    let contents = Contents {
        version: PayloadVersion::V1,
        identities: vec![stored],
    };
    Vault::create(
        dir.clone(),
        &passphrase(),
        KdfParams::FLOOR,
        &contents.encode().unwrap(),
    )
    .unwrap();
}

#[test]
fn a_vault_of_phase_4_is_opened_with_identifiers_and_an_empty_message_store() {
    run(async {
        let network = MockNetwork::new();
        let b = node(&network, 2).await;
        let dir = MemoryDir::new();
        phase_4_vault(&dir, &b);
        assert!(dir.read(MESSAGES, MAX_VAULT_FILE_LEN).unwrap().is_none());
        let (installation, _) = Installation::open(Box::new(dir.clone()), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        let first = (
            identity.id(),
            identity.contact_id(b.identity.identity()).unwrap(),
        );
        assert!(dir.read(MESSAGES, MAX_VAULT_FILE_LEN).unwrap().is_some());
        drop((identity, installation));
        // The identifiers were written: the same ones the next time.
        let (installation, _) = Installation::open(Box::new(dir.clone()), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        assert_eq!(
            (
                identity.id(),
                identity.contact_id(b.identity.identity()).unwrap()
            ),
            first
        );
        let (_, plaintext, _) = Vault::open(dir.clone(), &passphrase()).unwrap();
        assert_eq!(
            Contents::decode(&plaintext).unwrap().version,
            PayloadVersion::V2
        );
    });
}

#[test]
fn a_vault_with_identities_and_no_message_store_opens_and_says_so() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let (a, b, _) = queued_pair(&network, &dir).await;
        let at_a = record(&a, &b);
        // A store that is in place says nothing.
        assert!(!a.installation.message_store_missing());
        drop(a);
        dir.remove(MESSAGES).unwrap();
        // The vault alone is a backup of its identities (T-MI-9): it opens,
        // with an empty store, and says that the messages are not there.
        let (installation, _) = Installation::open(Box::new(dir.clone()), &passphrase()).unwrap();
        assert!(installation.message_store_missing());
        let identity = installation.identities().into_iter().next().unwrap();
        assert!(identity.conversation(&at_a, None, 10).is_empty());
        drop((identity, installation));
        // The store made then is the store from then on.
        let (installation, _) = Installation::open(Box::new(dir.clone()), &passphrase()).unwrap();
        assert!(!installation.message_store_missing());
    });
}

#[test]
fn a_vault_of_phase_4_beside_a_store_with_messages_is_refused() {
    run(async {
        let network = MockNetwork::new();
        let b = node(&network, 2).await;
        let dir = MemoryDir::new();
        phase_4_vault(&dir, &b);
        let old = dir.read(VAULT, MAX_VAULT_FILE_LEN).unwrap().unwrap();
        let (installation, _) = Installation::open(Box::new(dir.clone()), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        let at_a = identity.contact_id(b.identity.identity()).unwrap();
        identity.queue_message(at_a, text("kept")).await.unwrap();
        drop((identity, installation));
        // The vault of before the upgrade is put back beside the store.
        dir.overwrite(VAULT, &old).unwrap();
        let store = dir.read(MESSAGES, MAX_VAULT_FILE_LEN).unwrap().unwrap();
        assert_eq!(
            Installation::open(Box::new(dir.clone()), &passphrase()).err(),
            Some(InstallationError::MessageStore)
        );
        // Neither file was changed.
        assert_eq!(dir.read(VAULT, MAX_VAULT_FILE_LEN).unwrap().unwrap(), old);
        assert_eq!(
            dir.read(MESSAGES, MAX_VAULT_FILE_LEN).unwrap().unwrap(),
            store
        );
    });
}

#[test]
fn a_crash_while_the_message_store_is_made_leaves_an_installation_that_opens() {
    run(async {
        // A new installation: the vault is made, then the store.
        let made = MemoryDir::new();
        Vault::create(
            made.clone(),
            &passphrase(),
            KdfParams::FLOOR,
            &Contents::default().encode().unwrap(),
        )
        .unwrap();
        let vault_steps = made.steps();
        let mut step = vault_steps + 1;
        loop {
            let dir = MemoryDir::new();
            dir.crash_at(step);
            let created =
                Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR);
            let crashed = dir.crashed();
            drop(created);
            for outcome in CrashOutcome::ALL {
                let opened = Installation::open(Box::new(dir.restart(outcome)), &passphrase());
                assert!(opened.is_ok(), "{step} {outcome:?} {:?}", opened.err());
            }
            if !crashed {
                break;
            }
            step += 1;
        }
        // A vault of Phase 4: the store is made, then the vault written as
        // version 2.
        let network = MockNetwork::new();
        let b = node(&network, 2).await;
        let mut step = 1;
        loop {
            let dir = MemoryDir::new();
            phase_4_vault(&dir, &b);
            dir.crash_at(dir.steps() + step);
            let opened = Installation::open(Box::new(dir.clone()), &passphrase());
            let crashed = dir.crashed();
            drop(opened);
            for outcome in CrashOutcome::ALL {
                let (installation, _) =
                    Installation::open(Box::new(dir.restart(outcome)), &passphrase())
                        .unwrap_or_else(|error| panic!("{step} {outcome:?} {error:?}"));
                let identity = installation.identities().into_iter().next().unwrap();
                assert!(identity.contact_id(b.identity.identity()).is_some());
            }
            if !crashed {
                break;
            }
            step += 1;
        }
    });
}

#[test]
fn a_message_deleted_while_it_waits_to_be_sent_is_not_offered() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let (a, b, _) = queued_pair(&network, &dir).await;
        let at_a = record(&a, &b);
        dir.hold_writes();
        let released = Released(&dir);
        // Another change waits for its write, and the head with it.
        let identity = a.identity.clone();
        let queueing =
            tokio::spawn(async move { identity.queue_message(at_a, text("second")).await });
        held(&dir).await;
        let identity = a.identity.clone();
        let offering = tokio::spawn(async move { identity.next_to_send(&at_a).await });
        tokio::time::sleep(core::time::Duration::from_millis(100)).await;
        assert!(!offering.is_finished());
        // The user deletes the conversation meanwhile.
        let identity = a.identity.clone();
        let deleting = tokio::spawn(async move { identity.delete_conversation(&at_a).await });
        while a.identity.next_queued(&at_a).is_some() {
            tokio::time::sleep(core::time::Duration::from_millis(1)).await;
        }
        drop(released);
        assert!(offering.await.unwrap().unwrap().is_none());
        let _ = queueing.await.unwrap();
        assert!(deleting.await.unwrap().unwrap());
    });
}

#[test]
fn a_deleted_identity_restored_from_its_keys_has_no_messages() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let (a, b, _) = queued_pair(&network, &dir).await;
        let peer = *b.identity.identity();
        let old = a.identity.id();
        let installation = a.installation.clone();
        let identity = a.identity.clone();
        drop(a);
        installation.delete_identity(&identity).await.unwrap();
        let restored = installation
            .restore_identity(keys(1, 1, 1, fixed_endpoint(1)), None)
            .await
            .unwrap();
        assert_ne!(restored.id(), old);
        assert!(restored.conversations().is_empty());
        restored.import(&b.card()).await.unwrap();
        let again = restored.contact_id(&peer).unwrap();
        assert!(restored.next_queued(&again).is_none());
        drop((restored, identity, installation));
        let (installation, _) = Installation::open(Box::new(dir.clone()), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        assert!(identity.conversations().is_empty());
    });
}

/// Runs `first` and `second` together and returns what the one that ends
/// first gives; the other is dropped.
async fn race<A: core::future::Future, B: core::future::Future>(
    first: A,
    second: B,
) -> Result<A::Output, B::Output> {
    let mut first = core::pin::pin!(first);
    let mut second = core::pin::pin!(second);
    core::future::poll_fn(|cx| {
        if let core::task::Poll::Ready(value) = first.as_mut().poll(cx) {
            return core::task::Poll::Ready(Ok(value));
        }
        if let core::task::Poll::Ready(value) = second.as_mut().poll(cx) {
            return core::task::Poll::Ready(Err(value));
        }
        core::task::Poll::Pending
    })
    .await
}
