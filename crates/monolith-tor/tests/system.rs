//! `SystemTorBackend` against scripted SOCKS and control servers on
//! loopback, including hostile ones. No Tor and no network are used.

// Test code builds its own inputs, and its scripted servers listen on
// loopback.
#![allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use core::future::Future;
use core::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use monolith_identity::{IdentitySecretKey, OnionServiceKey, ServiceId};
use monolith_tor::{
    Bootstrap, ControlAuth, ControlStatus, Endpoint, KeySource, OnionService, OnionServiceSecret,
    Readiness, SOCKS_PROOF_LIFETIME, SocksProof, SocksStatus, SystemTorBackend, SystemTorConfig,
    TorBackend, TorError,
};
use sha2::Sha256;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn key(seed: u8) -> OnionServiceKey {
    let public = IdentitySecretKey::from_seed(&[seed; 32]).public_key();
    OnionServiceKey::from_bytes(public.as_bytes()).unwrap()
}

const COOKIE: [u8; 32] = [0x42; 32];
const SERVER_NONCE: [u8; 32] = [0x24; 32];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().to_vec()
}

/// How the scripted control server behaves.
#[derive(Clone)]
struct Script {
    version: &'static str,
    methods: &'static str,
    /// Send a wrong server hash.
    wrong_server_hash: bool,
    circuit: &'static str,
    bootstrap: &'static str,
    /// The reply to ADD_ONION, with `{id}` and `{key}` replaced.
    add_onion: String,
    /// Close the connection right after answering ADD_ONION.
    close_after_add: bool,
    del_onion: &'static str,
    /// Raw bytes sent instead of the PROTOCOLINFO reply.
    protocolinfo_override: Option<Vec<u8>>,
    /// The reply to GETINFO net/listeners/socks.
    listeners: String,
}

impl Script {
    fn good() -> Self {
        Self {
            version: "0.4.9.13",
            methods: "COOKIE,SAFECOOKIE",
            wrong_server_hash: false,
            circuit: "250-status/circuit-established=1\r\n250 OK\r\n",
            bootstrap: "250-status/bootstrap-phase=NOTICE BOOTSTRAP PROGRESS=100 TAG=done SUMMARY=\"Done\"\r\n250 OK\r\n",
            add_onion: "250-ServiceID={id}\r\n250-PrivateKey=ED25519-V3:{key}\r\n250 OK\r\n".into(),
            close_after_add: false,
            del_onion: "250 OK\r\n",
            protocolinfo_override: None,
            listeners: "552 Unrecognized key \"net/listeners/socks\"\r\n".into(),
        }
    }

    /// A good Tor whose SOCKS listeners are `addresses`.
    fn listing(addresses: &[SocketAddr]) -> Self {
        let quoted: Vec<String> = addresses
            .iter()
            .map(|address| format!("\"{address}\""))
            .collect();
        Self {
            listeners: format!("250-net/listeners/socks={}\r\n250 OK\r\n", quoted.join(" ")),
            ..Self::good()
        }
    }
}

/// A control server that runs the script for every connection and
/// records every line it receives.
struct FakeControl {
    address: SocketAddr,
    cookie_dir: PathBuf,
    lines: Arc<Mutex<Vec<String>>>,
}

impl FakeControl {
    async fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let cookie_dir = std::env::temp_dir().join(format!(
            "monolith-fake-control-{}-{}",
            std::process::id(),
            address.port()
        ));
        std::fs::create_dir_all(&cookie_dir).unwrap();
        let cookie_path = cookie_dir.join("control.authcookie");
        std::fs::write(&cookie_path, COOKIE).unwrap();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let recorded = lines.clone();
        let publications = Arc::new(AtomicU8::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let script = script.clone();
                let recorded = recorded.clone();
                let cookie_path = cookie_path.clone();
                let publications = publications.clone();
                tokio::spawn(serve(stream, script, recorded, cookie_path, publications));
            }
        });
        Self {
            address,
            cookie_dir,
            lines,
        }
    }

    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    /// How many control connections were opened: each starts with
    /// PROTOCOLINFO.
    fn sessions(&self) -> usize {
        self.lines()
            .iter()
            .filter(|line| line.as_str() == "PROTOCOLINFO 1\r\n")
            .count()
    }
}

impl Drop for FakeControl {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.cookie_dir);
    }
}

async fn serve(
    stream: TcpStream,
    script: Script,
    recorded: Arc<Mutex<Vec<String>>>,
    cookie: PathBuf,
    publications: Arc<AtomicU8>,
) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut client_nonce = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        recorded.lock().unwrap().push(line.clone());
        let command = line.trim_end_matches("\r\n");
        let reply: Vec<u8> = if command == "PROTOCOLINFO 1" {
            match &script.protocolinfo_override {
                Some(raw) => raw.clone(),
                None => format!(
                    "250-PROTOCOLINFO 1\r\n250-AUTH METHODS={} COOKIEFILE=\"{}\"\r\n250-VERSION Tor=\"{}\"\r\n250 OK\r\n",
                    script.methods,
                    // A QuotedString, as Tor writes it: a Windows path has
                    // backslashes, which it escapes.
                    cookie
                        .display()
                        .to_string()
                        .replace('\\', "\\\\")
                        .replace('"', "\\\""),
                    script.version
                )
                .into_bytes(),
            }
        } else if let Some(nonce) = command.strip_prefix("AUTHCHALLENGE SAFECOOKIE ") {
            client_nonce = unhex(nonce);
            let mut hash = hmac(
                b"Tor safe cookie authentication server-to-controller hash",
                &[&COOKIE, &client_nonce, &SERVER_NONCE],
            );
            if script.wrong_server_hash {
                hash[0] ^= 1;
            }
            format!(
                "250 AUTHCHALLENGE SERVERHASH={} SERVERNONCE={}\r\n",
                hex(&hash),
                hex(&SERVER_NONCE)
            )
            .into_bytes()
        } else if let Some(hash) = command.strip_prefix("AUTHENTICATE ") {
            let expected = hmac(
                b"Tor safe cookie authentication controller-to-server hash",
                &[&COOKIE, &client_nonce, &SERVER_NONCE],
            );
            if unhex(hash) == expected {
                b"250 OK\r\n".to_vec()
            } else {
                b"515 Authentication failed\r\n".to_vec()
            }
        } else if command == "AUTHENTICATE" {
            b"250 OK\r\n".to_vec()
        } else if command == "GETINFO status/circuit-established" {
            script.circuit.as_bytes().to_vec()
        } else if command == "GETINFO status/bootstrap-phase" {
            script.bootstrap.as_bytes().to_vec()
        } else if command == "GETINFO net/listeners/socks" {
            script.listeners.as_bytes().to_vec()
        } else if command.starts_with("ADD_ONION ") {
            // The first service of a server is key(77), the next key(78),
            // and so on, as a Tor names every service it holds differently.
            let id = ServiceId::from_key(&key(77 + publications.fetch_add(1, Ordering::SeqCst)));
            let reply = script
                .add_onion
                .replace("{id}", id.as_str())
                .replace("{key}", &format!("{}A==", "B".repeat(85)));
            let _ = write.write_all(reply.as_bytes()).await;
            if script.close_after_add {
                return;
            }
            continue;
        } else if command.starts_with("DEL_ONION ") {
            script.del_onion.as_bytes().to_vec()
        } else {
            b"510 Unrecognized command\r\n".to_vec()
        };
        if write.write_all(&reply).await.is_err() {
            return;
        }
    }
}

/// A SOCKS server that accepts the exchange and records the CONNECT
/// request, then answers with `connect_reply` and echoes.
async fn fake_socks(connect_reply: Vec<u8>) -> (SocketAddr, Arc<Mutex<Vec<Vec<u8>>>>) {
    let (address, requests, _) = counted_socks(connect_reply).await;
    (address, requests)
}

/// [`fake_socks`], with the number of connections it accepted.
async fn counted_socks(
    connect_reply: Vec<u8>,
) -> (SocketAddr, Arc<Mutex<Vec<Vec<u8>>>>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let connections = Arc::new(AtomicUsize::new(0));
    let accepted = connections.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            accepted.fetch_add(1, Ordering::SeqCst);
            let recorded = recorded.clone();
            let reply = connect_reply.clone();
            tokio::spawn(async move {
                let mut greeting = [0_u8; 3];
                if stream.read_exact(&mut greeting).await.is_err() {
                    return;
                }
                let _ = stream.write_all(&[5, 2]).await;
                let mut head = [0_u8; 2];
                if stream.read_exact(&mut head).await.is_err() {
                    return;
                }
                let mut user = vec![0_u8; usize::from(head[1])];
                stream.read_exact(&mut user).await.unwrap();
                let mut len = [0_u8; 1];
                stream.read_exact(&mut len).await.unwrap();
                let mut password = vec![0_u8; usize::from(len[0])];
                stream.read_exact(&mut password).await.unwrap();
                let _ = stream.write_all(&[1, 0]).await;
                let mut request = vec![0_u8; 5];
                stream.read_exact(&mut request).await.unwrap();
                let mut rest = vec![0_u8; usize::from(request[4]) + 2];
                stream.read_exact(&mut rest).await.unwrap();
                request.extend_from_slice(&rest);
                recorded.lock().unwrap().push(request);
                let _ = stream.write_all(&reply).await;
                let mut buffer = [0_u8; 64];
                while let Ok(n) = stream.read(&mut buffer).await {
                    if n == 0 || stream.write_all(&buffer[..n]).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (address, requests, connections)
}

/// SAFECOOKIE with the cookie file of a scripted control server.
fn safecookie(control: &FakeControl) -> ControlAuth {
    ControlAuth::SafeCookie {
        cookie_file: control.cookie_dir.join("control.authcookie"),
    }
}

/// SAFECOOKIE with the cookie file of a Debian system Tor, for backends
/// whose control endpoint is not used.
fn any_cookie() -> ControlAuth {
    ControlAuth::SafeCookie {
        cookie_file: PathBuf::from("/run/tor/control.authcookie"),
    }
}

fn system(socks: SocketAddr, control: SocketAddr, auth: ControlAuth) -> SystemTorBackend {
    SystemTorBackend::new(SystemTorConfig {
        socks: Endpoint::tcp(socks).unwrap(),
        control: Endpoint::tcp(control).unwrap(),
        auth,
        socks_proof: SocksProof::Off,
    })
}

/// [`system`] with the listener check on.
fn proved(socks: SocketAddr, control: SocketAddr, auth: ControlAuth) -> SystemTorBackend {
    SystemTorBackend::new(SystemTorConfig {
        socks: Endpoint::tcp(socks).unwrap(),
        control: Endpoint::tcp(control).unwrap(),
        auth,
        socks_proof: SocksProof::ControlListeners,
    })
}

/// The answer of a SOCKS proxy to a CONNECT that succeeded.
fn connected() -> Vec<u8> {
    vec![5, 0, 0, 1, 0, 0, 0, 0, 0, 0]
}

/// An address on which nothing listens.
async fn closed_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

#[test]
fn an_onion_connection_carries_the_literal_hostname_and_then_the_peer_bytes() {
    run(async {
        let (socks, requests) = fake_socks(vec![5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        let backend = system(socks, closed_port().await, any_cookie());
        let target = key(9);
        let group = backend.isolation_group().unwrap();
        let mut stream = backend.connect_onion(&target, &group).await.unwrap();
        stream.write_all(b"hello").await.unwrap();
        let mut echo = [0_u8; 5];
        stream.read_exact(&mut echo).await.unwrap();
        assert_eq!(&echo, b"hello");

        // The request is a domain name, the .onion name of the key, port
        // 29170. A local resolution would have produced an IP address type
        // and could not have produced this name.
        let request = requests.lock().unwrap()[0].clone();
        let hostname = ServiceId::from_key(&target).hostname();
        assert_eq!(&request[..4], &[5, 1, 0, 3]);
        assert_eq!(usize::from(request[4]), hostname.len());
        assert_eq!(&request[5..5 + hostname.len()], hostname.as_bytes());
        assert_eq!(&request[5 + hostname.len()..], &29170_u16.to_be_bytes());
    });
}

#[test]
fn without_a_socks_endpoint_there_is_no_connection_at_all() {
    run(async {
        let backend = system(closed_port().await, closed_port().await, any_cookie());
        let group = backend.isolation_group().unwrap();
        assert_eq!(
            backend.connect_onion(&key(9), &group).await.err(),
            Some(TorError::SocksUnavailable)
        );
        // A failed onion connection is an error, not a retry elsewhere.
        let (socks, _) = fake_socks(vec![5, 0xF2, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        let backend = backend_with(socks);
        assert_eq!(
            backend.connect_onion(&key(9), &group).await.err(),
            Some(TorError::OnionUnreachable)
        );
    });
}

fn backend_with(socks: SocketAddr) -> SystemTorBackend {
    SystemTorBackend::new(SystemTorConfig {
        socks: Endpoint::tcp(socks).unwrap(),
        control: Endpoint::tcp(socks).unwrap(),
        auth: any_cookie(),
        socks_proof: SocksProof::Off,
    })
}

#[test]
fn status_reports_a_ready_tor_and_sends_only_the_allowed_commands() {
    run(async {
        let control = FakeControl::start(Script::good()).await;
        let (socks, _) = fake_socks(vec![]).await;
        let backend = system(socks, control.address, safecookie(&control));
        let status = backend.status().await;
        assert_eq!(status.readiness(), Readiness::Ready);
        assert!(matches!(
            status.control,
            ControlStatus::Reachable {
                circuit_established: true,
                bootstrap: Bootstrap::Done,
                ..
            }
        ));
        assert_eq!(status.socks, SocksStatus::Reachable);
        let lines = control.lines();
        assert_eq!(lines[0], "PROTOCOLINFO 1\r\n");
        assert!(lines[1].starts_with("AUTHCHALLENGE SAFECOOKIE "));
        assert!(lines[2].starts_with("AUTHENTICATE "));
        assert_eq!(lines[3], "GETINFO status/circuit-established\r\n");
        assert_eq!(lines[4], "GETINFO status/bootstrap-phase\r\n");
        assert_eq!(lines.len(), 5);
    });
}

#[test]
fn a_server_that_does_not_know_the_cookie_never_gets_the_client_hash() {
    run(async {
        let control = FakeControl::start(Script {
            wrong_server_hash: true,
            ..Script::good()
        })
        .await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        let status = backend.status().await;
        assert_eq!(status.control, ControlStatus::AuthenticationFailed);
        assert!(
            control
                .lines()
                .iter()
                .all(|line| !line.starts_with("AUTHENTICATE"))
        );
    });
}

#[test]
fn authentication_follows_the_configuration_not_the_server() {
    run(async {
        // The server offers only NULL; SAFECOOKIE is configured.
        let control = FakeControl::start(Script {
            methods: "NULL",
            ..Script::good()
        })
        .await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        assert_eq!(
            backend.status().await.control,
            ControlStatus::AuthenticationUnavailable
        );
        assert!(
            control
                .lines()
                .iter()
                .all(|line| !line.starts_with("AUTHENTICATE"))
        );

        // Only COOKIE offered: not used.
        let control = FakeControl::start(Script {
            methods: "COOKIE",
            ..Script::good()
        })
        .await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        assert_eq!(
            backend.status().await.control,
            ControlStatus::AuthenticationUnavailable
        );

        // A trusted filter, configured as such.
        let control = FakeControl::start(Script {
            methods: "NULL",
            ..Script::good()
        })
        .await;
        let backend = system(
            closed_port().await,
            control.address,
            ControlAuth::TrustedFilter,
        );
        assert!(matches!(
            backend.status().await.control,
            ControlStatus::Reachable { .. }
        ));
        assert_eq!(control.lines()[1], "AUTHENTICATE\r\n");
    });
}

#[test]
fn an_old_tor_is_refused() {
    run(async {
        let control = FakeControl::start(Script {
            version: "0.4.8.25",
            ..Script::good()
        })
        .await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        assert!(matches!(
            backend.status().await.control,
            ControlStatus::UnsupportedVersion(_)
        ));
        assert_eq!(
            backend.publish_onion(KeySource::Generate).await.err(),
            Some(TorError::UnsupportedTorVersion)
        );
        assert!(
            control
                .lines()
                .iter()
                .all(|line| !line.starts_with("ADD_ONION"))
        );
    });
}

#[test]
fn hostile_control_replies_fail_cleanly() {
    run(async {
        let oversized = {
            let mut raw = b"250-PROTOCOLINFO 1\r\n250-".to_vec();
            raw.extend(std::iter::repeat_n(b'x', 5000));
            raw.extend_from_slice(b"\r\n");
            raw
        };
        let many_lines = {
            let mut raw = b"250-PROTOCOLINFO 1\r\n".to_vec();
            for _ in 0..40 {
                raw.extend_from_slice(b"250-X\r\n");
            }
            raw.extend_from_slice(b"250 OK\r\n");
            raw
        };
        for raw in [
            oversized,
            many_lines,
            b"250-PROTOCOLINFO 1\n250 OK\n".to_vec(),
            b"25O OK\r\n".to_vec(),
            b"650 STATUS_GENERAL NOTICE\r\n".to_vec(),
            b"250+PROTOCOLINFO\r\n.\r\n250 OK\r\n".to_vec(),
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=SAFECOOKIE COOKIEFILE=\"/nonexistent/cookie\"\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n".to_vec(),
        ] {
            let control = FakeControl::start(Script { protocolinfo_override: Some(raw), ..Script::good() }).await;
            let backend = system(closed_port().await, control.address, safecookie(&control));
            let status = tokio::time::timeout(Duration::from_secs(20), backend.status()).await.unwrap();
            assert!(
                matches!(
                    status.control,
                    ControlStatus::InvalidResponse
                        | ControlStatus::AuthenticationFailed
                        | ControlStatus::Unavailable
                ),
                "{status:?}"
            );
        }
    });
}

#[test]
fn a_published_service_is_owned_by_its_control_connection() {
    run(async {
        let control = FakeControl::start(Script::good()).await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
        assert_eq!(service.service_key(), &key(77));
        assert!(service.is_published());
        let secret = service.take_generated_secret().unwrap();
        assert_eq!(format!("{secret:?}"), "OnionServiceSecret([redacted])");
        assert!(service.take_generated_secret().is_none());

        // The exact ADD_ONION, with the listener on loopback.
        let lines = control.lines();
        let add = lines
            .iter()
            .find(|line| line.starts_with("ADD_ONION"))
            .unwrap();
        assert!(add.starts_with(
            "ADD_ONION NEW:ED25519-V3 Flags=MaxStreamsCloseCircuit MaxStreams=8 PoWDefensesEnabled=1 Port=29170,127.0.0.1:"
        ));
        let port: u16 = add.trim_end().rsplit(':').next().unwrap().parse().unwrap();
        assert_ne!(port, 0);

        // A stream to the listener arrives through accept.
        let connector = tokio::spawn(TcpStream::connect(("127.0.0.1", port)));
        let accepted = service.accept().await;
        assert!(accepted.is_ok());
        assert!(connector.await.unwrap().is_ok());

        // An orderly close sends DEL_ONION with the service's id.
        service.close().await.unwrap();
        let lines = control.lines();
        assert_eq!(
            lines.last().unwrap(),
            &format!("DEL_ONION {}\r\n", ServiceId::from_key(&key(77)).as_str())
        );
        // Every line sent is one of the allowed commands.
        for line in lines {
            assert!(
                [
                    "PROTOCOLINFO 1",
                    "AUTHCHALLENGE SAFECOOKIE ",
                    "AUTHENTICATE ",
                    "ADD_ONION ",
                    "DEL_ONION "
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix)),
                "{line}"
            );
        }
    });
}

#[test]
fn two_published_services_coexist_and_each_owns_its_listener() {
    // One backend holds several publications at once, as several local
    // identities need: each has its own control connection, listener and
    // key, a stream reaches only the service whose listener it was sent
    // to, and ending one publication leaves the other.
    run(async {
        let control = FakeControl::start(Script::good()).await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        let mut first = backend.publish_onion(KeySource::Generate).await.unwrap();
        let mut second = backend.publish_onion(KeySource::Generate).await.unwrap();
        assert_eq!(first.service_key(), &key(77));
        assert_eq!(second.service_key(), &key(78));
        assert!(first.is_published() && second.is_published());

        let ports: Vec<u16> = control
            .lines()
            .iter()
            .filter(|line| line.starts_with("ADD_ONION"))
            .map(|line| line.trim_end().rsplit(':').next().unwrap().parse().unwrap())
            .collect();
        assert_eq!(ports.len(), 2);
        assert_ne!(ports[0], ports[1]);

        let quiet = Duration::from_millis(200);
        let _to_second = TcpStream::connect(("127.0.0.1", ports[1])).await.unwrap();
        assert!(tokio::time::timeout(quiet, first.accept()).await.is_err());
        assert!(second.accept().await.is_ok());
        let _to_first = TcpStream::connect(("127.0.0.1", ports[0])).await.unwrap();
        assert!(tokio::time::timeout(quiet, second.accept()).await.is_err());
        assert!(first.accept().await.is_ok());

        first.close().await.unwrap();
        assert!(second.is_published());
        let _again = TcpStream::connect(("127.0.0.1", ports[1])).await.unwrap();
        assert!(second.accept().await.is_ok());
        let removed: Vec<String> = control
            .lines()
            .into_iter()
            .filter(|line| line.starts_with("DEL_ONION"))
            .collect();
        assert_eq!(
            removed,
            vec![format!(
                "DEL_ONION {}\r\n",
                ServiceId::from_key(&key(77)).as_str()
            )]
        );
        second.close().await.unwrap();
    });
}

#[test]
fn losing_the_control_connection_unpublishes_the_service() {
    run(async {
        let control = FakeControl::start(Script {
            close_after_add: true,
            ..Script::good()
        })
        .await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
        // The server closed right after the reply. Without any accept, the
        // handle already reports the service as gone.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!service.is_published());
        let accepted = tokio::time::timeout(Duration::from_secs(5), service.accept())
            .await
            .unwrap();
        assert_eq!(accepted.err(), Some(TorError::ControlLost));
        assert!(!service.is_published());
        assert_eq!(service.accept().await.err(), Some(TorError::ControlLost));
        // Closing a lost service sends nothing and succeeds.
        service.close().await.unwrap();
    });
}

#[test]
fn malformed_add_onion_replies_publish_nothing() {
    run(async {
        let valid_id = ServiceId::from_key(&key(77));
        let mut wrong = valid_id.as_str().as_bytes().to_vec();
        wrong[3] = if wrong[3] == b'a' { b'b' } else { b'a' };
        let wrong = String::from_utf8(wrong).unwrap();
        let cases = [
            (
                format!("250-ServiceID={wrong}\r\n250-PrivateKey=ED25519-V3:{{key}}\r\n250 OK\r\n"),
                TorError::InvalidTorResponse,
            ),
            (
                "250-ServiceID={id}\r\n250-PrivateKey=ED25519-V3:short==\r\n250 OK\r\n".to_string(),
                TorError::InvalidTorResponse,
            ),
            (
                "250-ServiceID={id}\r\n250 OK\r\n".to_string(),
                TorError::InvalidTorResponse,
            ),
            (
                "512 Tor is in non-anonymous hidden service mode\r\n".to_string(),
                TorError::NonAnonymousTorMode,
            ),
            (
                "551 Failed to add Onion Service\r\n".to_string(),
                TorError::OnionPublicationFailed,
            ),
            (
                "510 Command filtered\r\n".to_string(),
                TorError::CommandRefused,
            ),
        ];
        for (reply, expected) in cases {
            let control = FakeControl::start(Script {
                add_onion: reply.clone(),
                ..Script::good()
            })
            .await;
            let backend = system(closed_port().await, control.address, safecookie(&control));
            assert_eq!(
                backend.publish_onion(KeySource::Generate).await.err(),
                Some(expected),
                "{reply}"
            );
        }

        // A key for an existing service: no key in the reply, and the
        // ServiceID must be the expected one.
        let existing = "250-ServiceID={id}\r\n250 OK\r\n".to_string();
        let control = FakeControl::start(Script {
            add_onion: existing.clone(),
            ..Script::good()
        })
        .await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        let service = backend
            .publish_onion(KeySource::Existing {
                secret: OnionServiceSecret::from_bytes(&[1; 64]),
                expected: key(77),
            })
            .await
            .unwrap();
        drop(service);
        let add = control
            .lines()
            .into_iter()
            .find(|line| line.starts_with("ADD_ONION"))
            .unwrap();
        assert!(add.starts_with("ADD_ONION ED25519-V3:AQEB"));
        let control = FakeControl::start(Script {
            add_onion: existing,
            ..Script::good()
        })
        .await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        assert_eq!(
            backend
                .publish_onion(KeySource::Existing {
                    secret: OnionServiceSecret::from_bytes(&[1; 64]),
                    expected: key(78),
                })
                .await
                .err(),
            Some(TorError::OnionKeyMismatch)
        );
    });
}

#[test]
fn a_failed_del_onion_is_reported_and_the_service_still_ends() {
    run(async {
        let control = FakeControl::start(Script {
            del_onion: "552 Unknown Onion Service id\r\n",
            ..Script::good()
        })
        .await;
        let backend = system(closed_port().await, control.address, safecookie(&control));
        let service = backend.publish_onion(KeySource::Generate).await.unwrap();
        assert_eq!(
            service.close().await.err(),
            Some(TorError::OnionRemovalFailed)
        );
    });
}

#[test]
fn a_bootstrapping_tor_is_not_ready_and_a_filter_hides_progress() {
    run(async {
        let control = FakeControl::start(Script {
            circuit: "250-status/circuit-established=0\r\n250 OK\r\n",
            bootstrap: "250-status/bootstrap-phase=WARN BOOTSTRAP PROGRESS=25 TAG=x SUMMARY=\"y\" HOSTADDR=\"192.0.2.1:443\"\r\n250 OK\r\n",
            ..Script::good()
        })
        .await;
        let (socks, _) = fake_socks(vec![]).await;
        let backend = system(socks, control.address, safecookie(&control));
        assert_eq!(
            backend.status().await.readiness(),
            Readiness::NotReady(Bootstrap::InProgress(25))
        );

        let control = FakeControl::start(Script {
            circuit: "250-status/circuit-established=0\r\n250 OK\r\n",
            bootstrap: "510 Command filtered\r\n",
            ..Script::good()
        })
        .await;
        let backend = system(socks, control.address, safecookie(&control));
        assert_eq!(
            backend.status().await.readiness(),
            Readiness::NotReady(Bootstrap::Unknown)
        );
    });
}

#[test]
fn a_control_endpoint_that_names_another_cookie_file_is_refused() {
    run(async {
        // A server that wrote a cookie of its own and names it: it would
        // pass SAFECOOKIE for that file. The configured file is another.
        let control = FakeControl::start(Script::good()).await;
        let backend = system(closed_port().await, control.address, any_cookie());
        assert_eq!(
            backend.status().await.control,
            ControlStatus::AuthenticationFailed
        );
        assert_eq!(control.lines(), vec!["PROTOCOLINFO 1\r\n".to_owned()]);
        assert_eq!(
            backend.publish_onion(KeySource::Generate).await.err(),
            Some(TorError::ControlAuthentication)
        );
    });
}

// The listener check of the SOCKS endpoint (SocksProof::ControlListeners).

/// Dials once with `backend` and checks that the dial failed without a
/// connection to the SOCKS endpoint whose connections `socks` counts.
async fn refused_before_socks(backend: &SystemTorBackend, socks: &AtomicUsize, why: &str) {
    let group = backend.isolation_group().unwrap();
    assert_eq!(
        backend.connect_onion(&key(9), &group).await.err(),
        Some(TorError::SocksUnavailable),
        "{why}"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(socks.load(Ordering::SeqCst), 0, "{why}");
}

/// Starts `count` dials with `backend`, each before any of them ends, and
/// waits for all of them.
async fn at_once(backend: &Arc<SystemTorBackend>, count: u8) -> Vec<Result<(), TorError>> {
    let dials: Vec<_> = (0..count)
        .map(|index| {
            let backend = Arc::clone(backend);
            tokio::spawn(async move {
                let group = backend.isolation_group().unwrap();
                backend
                    .connect_onion(&key(20 + index), &group)
                    .await
                    .map(drop)
            })
        })
        .collect();
    let mut results = Vec::new();
    for dial in dials {
        results.push(dial.await.unwrap());
    }
    results
}

/// Moves the clock of the runtime forward by `duration` at once.
async fn skip(duration: Duration) {
    tokio::time::pause();
    tokio::time::advance(duration).await;
    tokio::time::resume();
}

/// A listener that closes every connection at once.
async fn closing_listener() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    address
}

#[test]
fn with_the_check_off_a_dial_asks_the_control_endpoint_nothing() {
    run(async {
        let (socks, _, connections) = counted_socks(connected()).await;
        let control = FakeControl::start(Script::listing(&[socks])).await;
        let backend = system(socks, control.address, safecookie(&control));
        let group = backend.isolation_group().unwrap();
        backend.connect_onion(&key(9), &group).await.unwrap();
        assert_eq!(connections.load(Ordering::SeqCst), 1);
        assert!(control.lines().is_empty());
    });
}

#[test]
fn with_the_check_on_a_dial_waits_until_tor_lists_the_socks_endpoint() {
    run(async {
        let (socks, requests, connections) = counted_socks(connected()).await;
        let control = FakeControl::start(Script::listing(&[socks])).await;
        let backend = proved(socks, control.address, safecookie(&control));
        let group = backend.isolation_group().unwrap();
        let mut stream = backend.connect_onion(&key(9), &group).await.unwrap();
        stream.write_all(b"hello").await.unwrap();
        let mut echo = [0_u8; 5];
        stream.read_exact(&mut echo).await.unwrap();
        assert_eq!(&echo, b"hello");
        assert_eq!(connections.load(Ordering::SeqCst), 1);
        assert_eq!(requests.lock().unwrap().len(), 1);
        // Authentication, then the one question.
        let lines = control.lines();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "PROTOCOLINFO 1\r\n");
        assert!(lines[1].starts_with("AUTHCHALLENGE SAFECOOKIE "));
        assert!(lines[2].starts_with("AUTHENTICATE "));
        assert_eq!(lines[3], "GETINFO net/listeners/socks\r\n");

        // Tor lists other listeners as well.
        let other = closed_port().await;
        let control = FakeControl::start(Script::listing(&[other, socks])).await;
        let backend = proved(socks, control.address, safecookie(&control));
        assert!(backend.connect_onion(&key(9), &group).await.is_ok());
        assert_eq!(connections.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn without_a_proof_nothing_reaches_the_socks_endpoint() {
    run(async {
        let (socks, _, connections) = counted_socks(connected()).await;

        let backend = proved(socks, closed_port().await, any_cookie());
        refused_before_socks(&backend, &connections, "no control endpoint").await;

        let control = FakeControl::start(Script {
            wrong_server_hash: true,
            ..Script::listing(&[socks])
        })
        .await;
        let backend = proved(socks, control.address, safecookie(&control));
        refused_before_socks(&backend, &connections, "a server without the cookie").await;
        assert!(
            control
                .lines()
                .iter()
                .all(|line| !line.starts_with("GETINFO"))
        );

        let control = FakeControl::start(Script::listing(&[socks])).await;
        let backend = proved(socks, control.address, any_cookie());
        refused_before_socks(&backend, &connections, "another cookie file").await;
        assert_eq!(control.lines(), vec!["PROTOCOLINFO 1\r\n".to_owned()]);

        // A filter answers authentication itself, so its answer proves
        // nothing; it is not asked.
        let control = FakeControl::start(Script {
            methods: "NULL",
            ..Script::listing(&[socks])
        })
        .await;
        let backend = proved(socks, control.address, ControlAuth::TrustedFilter);
        refused_before_socks(&backend, &connections, "a trusted filter").await;
        assert!(control.lines().is_empty());

        let unquoted = format!("250-net/listeners/socks={socks}\r\n250 OK\r\n");
        for (script, why) in [
            (
                Script::listing(&[closed_port().await]),
                "another endpoint listed",
            ),
            (Script::listing(&[]), "no endpoint listed"),
            (Script::good(), "the key refused"),
            (
                Script {
                    listeners: unquoted,
                    ..Script::good()
                },
                "an unquoted list",
            ),
        ] {
            let control = FakeControl::start(script).await;
            let backend = proved(socks, control.address, safecookie(&control));
            refused_before_socks(&backend, &connections, why).await;
            assert_eq!(control.sessions(), 1, "{why}");
        }
    });
}

#[test]
fn a_proof_lets_dials_through_for_its_lifetime_only() {
    run(async {
        let (socks, _, connections) = counted_socks(connected()).await;
        let control = FakeControl::start(Script::listing(&[socks])).await;
        let backend = proved(socks, control.address, safecookie(&control));
        let group = backend.isolation_group().unwrap();
        for target in [9, 10] {
            backend.connect_onion(&key(target), &group).await.unwrap();
        }
        assert_eq!(control.sessions(), 1);
        // Shortly before its end the proof still counts; from its end on it
        // does not.
        skip(SOCKS_PROOF_LIFETIME - Duration::from_secs(1)).await;
        backend.connect_onion(&key(11), &group).await.unwrap();
        assert_eq!(control.sessions(), 1);
        skip(Duration::from_secs(1)).await;
        backend.connect_onion(&key(12), &group).await.unwrap();
        assert_eq!(control.sessions(), 2);
        assert_eq!(connections.load(Ordering::SeqCst), 4);
    });
}

#[test]
fn dials_at_the_same_time_share_one_proof() {
    run(async {
        let (socks, _, connections) = counted_socks(connected()).await;
        let control = FakeControl::start(Script::listing(&[socks])).await;
        let backend = Arc::new(proved(socks, control.address, safecookie(&control)));
        let results = at_once(&backend, 4).await;
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        assert_eq!(control.sessions(), 1);
        assert_eq!(connections.load(Ordering::SeqCst), 4);

        // A proof that fails fails the dials that waited for it, without a
        // control connection for each.
        let control = FakeControl::start(Script {
            wrong_server_hash: true,
            ..Script::listing(&[socks])
        })
        .await;
        let backend = Arc::new(proved(socks, control.address, safecookie(&control)));
        let results = at_once(&backend, 4).await;
        assert!(
            results
                .iter()
                .all(|result| *result == Err(TorError::SocksUnavailable)),
            "{results:?}"
        );
        assert_eq!(control.sessions(), 1);
        assert_eq!(connections.load(Ordering::SeqCst), 4);
    });
}

#[test]
fn a_failure_of_tor_ends_the_proof() {
    run(async {
        // A dial that fails.
        let (socks, _, _) = counted_socks(vec![5, 0xF2, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        let control = FakeControl::start(Script::listing(&[socks])).await;
        let backend = proved(socks, control.address, safecookie(&control));
        let group = backend.isolation_group().unwrap();
        for sessions in 1..=2 {
            assert_eq!(
                backend.connect_onion(&key(9), &group).await.err(),
                Some(TorError::OnionUnreachable)
            );
            assert_eq!(control.sessions(), sessions);
        }

        // A status query that finds the control endpoint failing: its own
        // control connection, then a new proof.
        let (socks, _, _) = counted_socks(connected()).await;
        let control = FakeControl::start(Script {
            circuit: "250-status/circuit-established=2\r\n250 OK\r\n",
            ..Script::listing(&[socks])
        })
        .await;
        let backend = proved(socks, control.address, safecookie(&control));
        backend.connect_onion(&key(9), &group).await.unwrap();
        assert_eq!(control.sessions(), 1);
        let status = backend.status().await;
        assert_eq!(status.control, ControlStatus::InvalidResponse);
        assert_eq!(status.socks, SocksStatus::Reachable);
        assert_eq!(control.sessions(), 3);

        // A status query whose SOCKS greeting fails.
        let closing = closing_listener().await;
        let control = FakeControl::start(Script::listing(&[closing])).await;
        let backend = proved(closing, control.address, safecookie(&control));
        assert_ne!(backend.status().await.socks, SocksStatus::Reachable);
        assert_eq!(control.sessions(), 2);
        backend.status().await;
        assert_eq!(control.sessions(), 4);
    });
}

#[test]
fn losing_a_published_service_ends_the_proof() {
    run(async {
        let (socks, _, _) = counted_socks(connected()).await;
        let control = FakeControl::start(Script {
            close_after_add: true,
            ..Script::listing(&[socks])
        })
        .await;
        let backend = proved(socks, control.address, safecookie(&control));
        let group = backend.isolation_group().unwrap();
        backend.connect_onion(&key(9), &group).await.unwrap();
        let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
        backend.connect_onion(&key(9), &group).await.unwrap();
        // The proof, then the publication.
        assert_eq!(control.sessions(), 2);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!service.is_published());
        backend.connect_onion(&key(9), &group).await.unwrap();
        assert_eq!(control.sessions(), 3);
    });
}

#[test]
fn with_the_check_on_status_reports_a_socks_endpoint_tor_does_not_list() {
    run(async {
        let (socks, _, connections) = counted_socks(vec![]).await;
        let control = FakeControl::start(Script::listing(&[closed_port().await])).await;
        let backend = proved(socks, control.address, safecookie(&control));
        let status = backend.status().await;
        assert!(matches!(status.control, ControlStatus::Reachable { .. }));
        assert_eq!(status.socks, SocksStatus::Unavailable);
        assert_eq!(status.readiness(), Readiness::SocksUnavailable);
        // Not even greeted.
        assert_eq!(connections.load(Ordering::SeqCst), 0);

        // Tor not running: nothing is greeted either.
        let backend = proved(socks, closed_port().await, any_cookie());
        assert_eq!(backend.status().await.readiness(), Readiness::Unavailable);
        assert_eq!(connections.load(Ordering::SeqCst), 0);

        // Listed: the greeting, and only allowed commands.
        let control = FakeControl::start(Script::listing(&[socks])).await;
        let backend = proved(socks, control.address, safecookie(&control));
        assert_eq!(backend.status().await.readiness(), Readiness::Ready);
        assert_eq!(connections.load(Ordering::SeqCst), 1);
        for line in control.lines() {
            assert!(
                [
                    "PROTOCOLINFO 1\r\n",
                    "AUTHCHALLENGE SAFECOOKIE ",
                    "AUTHENTICATE ",
                    "GETINFO status/circuit-established\r\n",
                    "GETINFO status/bootstrap-phase\r\n",
                    "GETINFO net/listeners/socks\r\n",
                ]
                .iter()
                .any(|allowed| line.starts_with(allowed)),
                "{line}"
            );
        }
    });
}
