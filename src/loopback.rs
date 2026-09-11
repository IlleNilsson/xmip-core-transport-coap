//! Both ends of one exchange on this machine (ADR-0051): a server on an
//! ephemeral local port, and a Stream sent to it as confirmable POSTs
//! of at most [`MAX_PAYLOAD`] bytes in turn, each acknowledged before the
//! next goes, then an empty POST to close. The server takes them in order, a
//! retransmitted one only once. What block-wise transfer (RFC 7959) will do
//! above this, done here so the round stays one thing.
//!
//! One message in flight is block-wise transfer with a window of one, and it
//! is the flow control that keeps a burst from overrunning the far end's
//! socket: sent non-confirmable, a mebibyte lost its tail on loopback
//! (2026-09-09), which was UDP being honest about a sender without one.

use std::net::UdpSocket;
use std::time::Duration;

use transport::Arrived;
use transport::Transport;
use transport::error::Result;
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};

use crate::{CoapTransport, MAX_PAYLOAD, message};

/// Loopback acknowledges within a millisecond; a hundred, doubled on every
/// retransmission, keeps a far end that went away inside the bound a round
/// is judged by.
const LOOPBACK_ACK: Duration = Duration::from_millis(100);

impl CoapTransport {
    /// Both ends on this machine: an ephemeral local port, the loopback
    /// timeout on the receive, a short wait for each acknowledgement.
    #[must_use]
    pub fn loopback() -> Self {
        Self::new("127.0.0.1:0")
            .acknowledged_within(LOOPBACK_ACK)
            .timing_out_after(LOOPBACK_TIMEOUT)
    }
}

/// A bound server waiting for one Stream's POSTs. Bound before the sender
/// fires, or the first one is gone.
struct Bound {
    /// Taking and answering read nothing of the instance; a loopback one
    /// stands in for the one that bound the socket.
    transport: CoapTransport,
    socket: UdpSocket,
    address: String,
}

impl FarEnd for Bound {
    fn address(&self) -> &str {
        &self.address
    }

    /// The POSTs in order, each answered 2.04 Changed, a retransmitted one
    /// taken once, until the empty one closes the Stream.
    fn take_one(self: Box<Self>) -> Result<Arrived> {
        let mut bytes = Vec::new();
        let mut last_seen: Option<(String, u16)> = None;
        loop {
            let request = self.transport.receive_one(&self.socket)?;
            self.transport
                .respond(&self.socket, &request, message::CHANGED, &[])?;
            let seen = (request.peer.clone(), request.message.id);
            if last_seen.as_ref() == Some(&seen) {
                continue;
            }
            last_seen = Some(seen);
            if request.message.payload.is_empty() {
                return Ok(Arrived::new(request.arrived().origin_uri, bytes));
            }
            bytes.extend_from_slice(&request.message.payload);
        }
    }
}

impl Loopback for CoapTransport {
    fn far_end(&self) -> Result<Box<dyn FarEnd>> {
        let (socket, address) = self.bind()?;
        Ok(Box::new(Bound {
            transport: Self::loopback(),
            socket,
            address,
        }))
    }

    fn send_to(&self, address: &str, payload: &[u8]) -> Result<()> {
        let near_end = Self::new("127.0.0.1:0").acknowledged_within(self.ack_timeout);
        let target = format!("coap://{address}/pingpong");
        for block in payload.chunks(MAX_PAYLOAD) {
            near_end.send(&target, block)?;
        }
        near_end.send(&target, &[])
    }

    fn unblock(&self, _address: &str) {
        // The receive has its own timeout; there is no listener to poke.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shapes a transport is most likely to change: nothing, one byte,
    /// every byte value, a run of NULs, high bytes, and line endings alone.
    fn edge_payloads() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("empty", Vec::new()),
            ("one byte", vec![0x2a]),
            ("every byte", (0..=255).collect()),
            ("nul run", vec![0; 512]),
            ("high bytes", vec![0xff; 512]),
            ("crlf storm", b"\r\n".repeat(400)),
        ]
    }

    #[test]
    fn the_loopback_carries_a_stream_as_posts_in_turn() {
        let server = CoapTransport::loopback();
        let arrived = server.round(b"post").expect("round");
        assert_eq!(arrived.bytes, b"post");
        assert!(arrived.origin_uri.starts_with("coap://127.0.0.1:"));
        assert!(arrived.origin_uri.contains("/pingpong?code=0.02&id="));
        let long = vec![0x2a; 5000];
        assert_eq!(server.round(&long).expect("five posts").bytes, long);
        assert!(server.round(b"").expect("empty").bytes.is_empty());
    }

    #[test]
    fn the_loopback_returns_the_edge_payloads_whole() {
        let server = CoapTransport::loopback();
        assert!(server.ceiling().is_none());
        for (name, bytes) in edge_payloads() {
            assert!(server.refuses(&bytes).is_none(), "{name}");
            assert_eq!(server.round(&bytes).expect(name).bytes, bytes, "{name}");
        }
    }
}
