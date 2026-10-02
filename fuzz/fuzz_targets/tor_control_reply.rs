//! Tor control replies: whatever a control endpoint sends, the parser keeps
//! within its limits, an error is final, and every interpretation of a
//! reply is consistent; and no key bytes can add a line to `ADD_ONION`.
//!
//! Input: the first byte selects the size of the pieces the bytes arrive
//! in; the rest is the stream from the control endpoint. The first 64
//! bytes of the rest also serve as an onion service key, and the next two
//! as a listener port, for the `ADD_ONION` encoder.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_identity::ServiceId;
use monolith_protocol::limits::{
    MAX_CONTROL_LINE_LEN, MAX_CONTROL_REPLY_LEN, MAX_CONTROL_REPLY_LINES,
};
use monolith_tor::Bootstrap;
use monolith_tor::fuzzing::{Reply, ReplyParser, add_onion_line, interpret};

fn check(reply: &Reply) {
    let lines = reply.lines();
    assert!(!lines.is_empty() && lines.len() <= MAX_CONTROL_REPLY_LINES);
    let mut total = 0_usize;
    for line in lines {
        // One status code for the whole reply, never an event.
        assert_eq!(line.code(), reply.code());
        assert!((100..600).contains(&line.code()));
        assert!(line.text().len() <= MAX_CONTROL_LINE_LEN);
        assert!(!line.text().contains(&b'\r') && !line.text().contains(&b'\n'));
        total += line.text().len() + 6;
    }
    assert!(total <= MAX_CONTROL_REPLY_LEN);

    let read = interpret(reply);
    if let Ok((key, with_key)) = &read.add_onion_new {
        // A new key comes with the service, in the second line.
        assert!(*with_key);
        assert_eq!(lines.len(), 3);
        let id = lines[0].text().strip_prefix(b"ServiceID=").unwrap();
        assert_eq!(ServiceId::from_key(key).as_str().as_bytes(), id);
    }
    if let Ok((key, with_key)) = &read.add_onion_existing {
        assert!(!*with_key);
        assert_eq!(lines.len(), 2);
        let id = lines[0].text().strip_prefix(b"ServiceID=").unwrap();
        assert_eq!(ServiceId::from_key(key).as_str().as_bytes(), id);
    }
    if read.protocolinfo.is_ok()
        || read.authchallenge.is_ok()
        || read.circuit_established.is_ok()
        || read.add_onion_new.is_ok()
        || read.add_onion_existing.is_ok()
    {
        assert_eq!(reply.code(), 250);
    }
    if let Ok(Bootstrap::InProgress(percent)) = read.bootstrap {
        assert!(percent <= 100);
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((piece, rest)) = data.split_first() else {
        return;
    };
    let piece = usize::from(*piece % 64) + 1;

    let mut parser = ReplyParser::new();
    'stream: for chunk in rest.chunks(piece) {
        let mut input = chunk;
        while !input.is_empty() {
            match parser.feed(input) {
                Ok((used, reply)) => {
                    assert!(used >= 1 && used <= input.len());
                    input = &input[used..];
                    if let Some(reply) = reply {
                        check(&reply);
                    }
                }
                Err(_) => {
                    assert!(parser.feed(b"250 OK\r\n").is_err());
                    break 'stream;
                }
            }
        }
    }

    // No key bytes can break the command line.
    if let (Some(key), Some(port)) = (rest.first_chunk::<64>(), rest.get(64..66)) {
        let port = u16::from_be_bytes([port[0], port[1]]);
        match add_onion_line(key, port) {
            Some(line) => {
                assert_ne!(port, 0);
                assert!(line.starts_with(b"ADD_ONION ED25519-V3:"));
                let end = format!(" PoWDefensesEnabled=1 Port=29170,127.0.0.1:{port}\r\n");
                assert!(line.ends_with(end.as_bytes()));
                let body = &line[..line.len() - 2];
                assert!(!body.contains(&b'\r') && !body.contains(&b'\n'));
                assert_eq!(line.iter().filter(|byte| **byte == b' ').count(), 5);
            }
            None => assert_eq!(port, 0),
        }
    }
});
