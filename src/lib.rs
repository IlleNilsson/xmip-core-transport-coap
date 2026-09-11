#![forbid(unsafe_code)]

//! Streams that arrive as CoAP requests. One request is one Stream, the
//! resource path kept beside it.
//!
//! CoAP is HTTP for things that run on a coin cell: the same verbs and
//! status classes, in a four-byte header over UDP on port 5683. A Receive
//! Location binds and takes what constrained devices POST or PUT to it,
//! answering each confirmable request so the device stops retransmitting; a
//! Send Location POSTs a Stream to a resource and waits for the
//! acknowledgement, retransmitting the way RFC 7252 section 4.2 says.
//!
//! One message carries at most [`MAX_PAYLOAD`] bytes. Block-wise transfer,
//! RFC 7959, is the layer that splits a larger Stream, and it is not here yet:
//! a payload over the limit is refused and says so. Observe (RFC 7641) and
//! DTLS are likewise later.
//!
//! The origin URI carries what the header knew:
//! `coap://peer/sensors/1?code=0.02&id=4660`.
//!
//! ```text
//! message.rs   the four-byte header, the token, the options, the payload
//! loopback.rs  both ends on this machine: a Stream as POSTs in turn
//! ```

mod loopback;
pub mod message;

use std::net::UdpSocket;
use std::time::Duration;

pub use message::{Kind, MAX_DATAGRAM, MAX_PAYLOAD, Message};
use transport::error::{Result, TransportError, classify, protocol_error};
use transport::socket;
use transport::{Arrived, Directions, Transport};

/// `ACK_TIMEOUT`, RFC 7252 section 4.8.
pub const ACK_TIMEOUT: Duration = Duration::from_secs(2);
/// `MAX_RETRANSMIT`, RFC 7252 section 4.8.
pub const MAX_RETRANSMIT: u32 = 4;

pub struct CoapTransport {
    bind: String,
    code: u8,
    confirmable: bool,
    ack_timeout: Duration,
    receive_timeout: Option<Duration>,
    next_id: std::sync::atomic::AtomicU16,
    /// The sending socket, bound once and connected per request: a Stream
    /// that travels as thousands of messages should not bind thousands of
    /// sockets.
    sender: std::sync::Mutex<Option<UdpSocket>>,
}

impl CoapTransport {
    /// Bind at `bind`; `0.0.0.0:5683` is the standard port. Sends POST,
    /// confirmable.
    #[must_use]
    pub fn new(bind: impl Into<String>) -> Self {
        Self {
            bind: bind.into(),
            code: message::POST,
            confirmable: true,
            ack_timeout: ACK_TIMEOUT,
            receive_timeout: None,
            next_id: std::sync::atomic::AtomicU16::new(1),
            sender: std::sync::Mutex::new(None),
        }
    }

    /// Send with `code` — PUT rather than POST, say.
    #[must_use]
    pub const fn with_code(mut self, code: u8) -> Self {
        self.code = code;
        self
    }

    /// Send non-confirmable: fire and forget, no acknowledgement awaited.
    #[must_use]
    pub const fn non_confirmable(mut self) -> Self {
        self.confirmable = false;
        self
    }

    /// Wait this long for an acknowledgement before retransmitting.
    #[must_use]
    pub const fn acknowledged_within(mut self, timeout: Duration) -> Self {
        self.ack_timeout = timeout;
        self
    }

    /// Give up waiting for a request after `timeout`.
    #[must_use]
    pub const fn timing_out_after(mut self, timeout: Duration) -> Self {
        self.receive_timeout = Some(timeout);
        self
    }

    /// Bind and report the address actually assigned.
    ///
    /// # Errors
    /// Where the address is taken, malformed, or not permitted.
    pub fn bind(&self) -> Result<(UdpSocket, String)> {
        socket::bind_udp(&self.bind, self.receive_timeout)
    }

    /// Take one request from an already-bound socket, and where it came
    /// from. A message that is not a request is answered with Reset and
    /// skipped.
    ///
    /// # Errors
    /// Where nothing arrived in time, or what arrived is not CoAP.
    pub fn receive_one(&self, socket: &UdpSocket) -> Result<Request> {
        let mut buffer = vec![0u8; MAX_DATAGRAM];
        loop {
            let (read, peer) = socket
                .recv_from(&mut buffer)
                .map_err(|e| classify("receiving a datagram", &e))?;
            let message = message::decode(&buffer[..read])?;
            if message.is_request() {
                return Ok(Request {
                    peer: peer.to_string(),
                    message,
                });
            }
            let reset = Message {
                kind: Kind::Reset,
                code: 0,
                id: message.id,
                token: Vec::new(),
                options: Vec::new(),
                payload: Vec::new(),
            };
            socket
                .send_to(&message::encode(&reset)?, peer)
                .map_err(|e| classify("sending Reset", &e))?;
        }
    }

    /// Answer `request` with `code` and `payload`.
    ///
    /// # Errors
    /// Where the answer could not be sent or does not fit a datagram.
    pub fn respond(
        &self,
        socket: &UdpSocket,
        request: &Request,
        code: u8,
        payload: &[u8],
    ) -> Result<()> {
        let response = request.message.response(code, payload);
        socket
            .send_to(&message::encode(&response)?, &request.peer)
            .map_err(|e| classify("sending the response", &e))?;
        Ok(())
    }

    /// One request to `target`, `coap://host:5683/path`, and the response.
    /// Confirmable requests are retransmitted up to [`MAX_RETRANSMIT`] times.
    ///
    /// # Errors
    /// A payload over [`MAX_PAYLOAD`], a target that is not `coap://`, a peer
    /// that never acknowledged, or a response with an error class.
    pub fn exchange(&self, target: &str, payload: &[u8]) -> Result<Message> {
        if payload.len() > MAX_PAYLOAD {
            return Err(protocol_error(
                "a payload over what one CoAP message carries; block-wise transfer is not here",
            ));
        }
        let rest = target
            .strip_prefix("coap://")
            .ok_or_else(|| protocol_error(format!("{target:?} is not coap://host/path")))?;
        let (address, path) = rest.split_once('/').unwrap_or((rest, ""));
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let kind = if self.confirmable {
            Kind::Confirmable
        } else {
            Kind::NonConfirmable
        };
        let request = message::encode(&Message::request(kind, self.code, id, path, payload))?;
        let mut guard = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_none() {
            let bound = UdpSocket::bind("0.0.0.0:0")
                .map_err(|e| classify("binding the sending socket", &e))?;
            *guard = Some(bound);
        }
        let socket = guard
            .as_ref()
            .ok_or_else(|| protocol_error("no sending socket"))?;
        socket
            .connect(address)
            .map_err(|e| classify("resolving the peer", &e))?;
        let mut timeout = self.ack_timeout;
        let mut buffer = vec![0u8; MAX_DATAGRAM];
        for attempt in 0..=MAX_RETRANSMIT {
            socket
                .send(&request)
                .map_err(|e| classify("sending the request", &e))?;
            if !self.confirmable {
                return Ok(Message::request(Kind::NonConfirmable, 0, id, "", &[]));
            }
            socket
                .set_read_timeout(Some(timeout))
                .map_err(|e| classify("setting the acknowledgement timeout", &e))?;
            match socket.recv(&mut buffer) {
                Ok(read) => {
                    let response = message::decode(&buffer[..read])?;
                    if response.id != id {
                        continue;
                    }
                    if response.code >> 5 >= 4 {
                        return Err(protocol_error(format!(
                            "the peer answered {}",
                            response.code_text()
                        )));
                    }
                    return Ok(response);
                }
                Err(error) if attempt < MAX_RETRANSMIT && is_timeout(&error) => timeout *= 2,
                Err(error) => return Err(classify("awaiting the acknowledgement", &error)),
            }
        }
        Err(TransportError::retryable(
            "the peer did not acknowledge after every retransmission",
        ))
    }
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// One request as it arrived, and who sent it.
#[derive(Clone, Debug)]
pub struct Request {
    pub peer: String,
    pub message: Message,
}

impl Request {
    /// The Stream this request carries, its origin from the header.
    #[must_use]
    pub fn arrived(&self) -> Arrived {
        Arrived::new(
            format!(
                "coap://{}/{}?code={}&id={}",
                self.peer,
                self.message.uri_path(),
                self.message.code_text(),
                self.message.id
            ),
            self.message.payload.clone(),
        )
    }
}

impl Transport for CoapTransport {
    fn name(&self) -> &'static str {
        "coap"
    }

    fn directions(&self) -> Directions {
        Directions::BOTH
    }

    /// One request, acknowledged 2.04 Changed.
    fn receive(&self) -> Result<Vec<Arrived>> {
        let (socket, _) = self.bind()?;
        let request = self.receive_one(&socket)?;
        self.respond(&socket, &request, message::CHANGED, &[])?;
        Ok(vec![request.arrived()])
    }

    fn send(&self, target: &str, bytes: &[u8]) -> Result<()> {
        self.exchange(target, bytes).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_post_is_taken_and_acknowledged() {
        let far_end = CoapTransport::new("127.0.0.1:0").timing_out_after(Duration::from_secs(2));
        let (socket, address) = far_end.bind().expect("binding");
        let sender = std::thread::spawn(move || {
            CoapTransport::new("127.0.0.1:0")
                .acknowledged_within(Duration::from_millis(500))
                .exchange(&format!("coap://{address}/sensors/1"), b"21.5")
        });
        let request = far_end.receive_one(&socket).expect("receiving");
        assert_eq!(request.message.payload, b"21.5");
        assert_eq!(request.message.uri_path(), "sensors/1");
        far_end
            .respond(&socket, &request, message::CREATED, b"ok")
            .expect("responding");
        let response = sender.join().expect("thread").expect("exchange");
        assert_eq!(response.code_text(), "2.01");
        assert_eq!(response.payload, b"ok");
        let arrived = request.arrived();
        assert!(arrived.origin_uri.contains("/sensors/1?code=0.02&id="));
    }

    #[test]
    fn an_unanswered_request_is_retransmitted_then_retryable() {
        let far_end = CoapTransport::new("127.0.0.1:0").timing_out_after(Duration::from_secs(2));
        let (socket, address) = far_end.bind().expect("binding");
        let sender = std::thread::spawn(move || {
            CoapTransport::new("127.0.0.1:0")
                .acknowledged_within(Duration::from_millis(20))
                .exchange(&format!("coap://{address}/quiet"), b"x")
        });
        let mut ids = Vec::new();
        for _ in 0..=MAX_RETRANSMIT {
            ids.push(far_end.receive_one(&socket).expect("attempt").message.id);
        }
        assert!(ids.iter().all(|id| *id == ids[0]), "the same message id");
        let error = sender.join().expect("thread").expect_err("unacknowledged");
        assert!(error.retryable);
    }

    #[test]
    fn an_error_class_and_an_oversize_payload_are_permanent() {
        let far_end = CoapTransport::new("127.0.0.1:0").timing_out_after(Duration::from_secs(2));
        let (socket, address) = far_end.bind().expect("binding");
        let sender = std::thread::spawn(move || {
            CoapTransport::new("127.0.0.1:0").exchange(&format!("coap://{address}/x"), b"bad")
        });
        let request = far_end.receive_one(&socket).expect("receiving");
        far_end
            .respond(&socket, &request, message::BAD_REQUEST, &[])
            .expect("responding");
        let error = sender.join().expect("thread").expect_err("4.00");
        assert!(!error.retryable);
        assert!(error.message.contains("4.00"));
        let too_big = CoapTransport::new("127.0.0.1:0")
            .send("coap://127.0.0.1:1/x", &[0; MAX_PAYLOAD + 1])
            .expect_err("too big");
        assert!(!too_big.retryable);
        assert!(far_end.claims().is_none());
    }

    #[test]
    fn a_non_confirmable_send_does_not_wait() {
        let far_end = CoapTransport::new("127.0.0.1:0").timing_out_after(Duration::from_secs(2));
        let (socket, address) = far_end.bind().expect("binding");
        CoapTransport::new("127.0.0.1:0")
            .non_confirmable()
            .send(&format!("coap://{address}/fire"), b"forget")
            .expect("sending");
        let request = far_end.receive_one(&socket).expect("receiving");
        assert_eq!(request.message.kind, Kind::NonConfirmable);
        assert_eq!(request.message.payload, b"forget");
    }
}
