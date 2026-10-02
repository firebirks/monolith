//! SOCKS5 CONNECT replies: a reply is complete exactly when the bytes its
//! address type calls for have arrived, it is never longer than the bound,
//! shorter prefixes are incomplete rather than wrong, and an error stays
//! an error whatever follows.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::limits::MAX_SOCKS_REPLY_LEN;
use monolith_tor::fuzzing::{ConnectReply, decode_connect_reply};

fuzz_target!(|data: &[u8]| {
    match decode_connect_reply(data) {
        Ok(Some((reply, len))) => {
            assert!(len <= data.len() && len <= MAX_SOCKS_REPLY_LEN);
            assert_eq!(data[0], 5);
            assert_eq!(data[2], 0);
            assert_eq!(reply == ConnectReply::Succeeded, data[1] == 0);
            let expected = match data[3] {
                1 => 10,
                4 => 22,
                3 => 7 + usize::from(data[4]),
                other => panic!("address type {other} accepted"),
            };
            assert_eq!(len, expected);
            for cut in 0..len {
                assert_eq!(decode_connect_reply(&data[..cut]), Ok(None), "{cut}");
            }
            assert_eq!(decode_connect_reply(&data[..len]), Ok(Some((reply, len))));
        }
        Ok(None) => assert!(data.len() < MAX_SOCKS_REPLY_LEN),
        Err(_) => {
            let mut more = data.to_vec();
            more.extend_from_slice(&[0; MAX_SOCKS_REPLY_LEN]);
            assert!(decode_connect_reply(&more).is_err());
        }
    }
});
