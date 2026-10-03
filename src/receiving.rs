//! A request as a Receive Location hands it on, and its answer once the
//! receive cycle has ended.
//!
//! A confirmable request is answered after the whole cycle, piggybacked on
//! its ACK: 2.04 Changed on [`Verdict::Accepted`]; on [`Verdict::Refused`]
//! a 4.xx, the client error RFC 7252 section 5.9.2 says is not to be
//! repeated unchanged — 4.01 Unauthorized for a sender not identified,
//! 4.03 Forbidden for one not permitted, 4.00 Bad Request for content
//! refused; 5.03 Service Unavailable (section 5.9.3.4) on
//! [`Verdict::Failed`], which tells the device to send it again. Until then
//! the device retransmits as RFC 7252 section 4.2 says, and nothing is
//! taken twice: a retransmission of a request already
//! answered is answered again from [`Answered`] (the deduplication of
//! section 4.5), and one whose answer never went is a request again. A
//! non-confirmable request expects no answer: acceptance is at-most-once
//! there ([`AT_MOST_ONCE`]).

use std::collections::VecDeque;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex, PoisonError};

use transport::answer::Datagram;
use transport::error::{Result, classify};
use transport::{Acknowledgement, Arrived, Refusal, Verdict};

use crate::message::{self, Kind};
use crate::{CoapTransport, Request};

/// Why a non-confirmable request cannot be acknowledged after the receive
/// cycle.
pub const AT_MOST_ONCE: &str = "a non-confirmable CoAP request expects no answer, so the device \
                                is never told how the receive cycle ended";

/// How many answers are kept to answer a retransmission again: more than
/// one device retransmits within one exchange lifetime on any Location
/// this side serves.
const REMEMBERED: usize = 64;

/// The answers last sent, by peer and message id.
#[derive(Default)]
pub struct Answered(Mutex<VecDeque<(String, u16, Vec<u8>)>>);

impl Answered {
    /// Answer `request` again where it was answered already, and say so.
    ///
    /// # Errors
    /// Where the answer could not be sent.
    fn repeated(&self, socket: &UdpSocket, request: &Request) -> Result<bool> {
        let answered = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((_, _, answer)) = answered
            .iter()
            .find(|(peer, id, _)| *peer == request.peer && *id == request.message.id)
        else {
            return Ok(false);
        };
        socket
            .send_to(answer, &request.peer)
            .map_err(|e| classify("answering a retransmission", &e))?;
        Ok(true)
    }

    fn remember(&self, request: &Request, answer: Vec<u8>) {
        let mut answered = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if answered.len() == REMEMBERED {
            answered.pop_front();
        }
        answered.push_back((request.peer.clone(), request.message.id, answer));
    }
}

impl CoapTransport {
    /// The next request on `socket` not answered already, as a Stream whose
    /// answer waits for the verdict.
    ///
    /// # Errors
    /// Where nothing arrived in time, what arrived is not CoAP, or the
    /// socket could not be shared with the answer.
    pub(crate) fn arrival(&self, socket: &UdpSocket) -> Result<Arrived> {
        let request = loop {
            let request = self.receive_one(socket)?;
            if !self.answered.repeated(socket, &request)? {
                break request;
            }
        };
        let origin = request.origin();
        let payload = request.message.payload.clone();
        if request.message.kind != Kind::Confirmable {
            let acknowledgement = Acknowledgement::at_most_once(AT_MOST_ONCE);
            return Ok(Arrived::whole(origin, payload, acknowledgement));
        }
        let answering = Datagram::to(socket, request.peer.clone())?;
        let answered = Arc::clone(&self.answered);
        let acknowledgement = Acknowledgement::deferred(move |verdict| {
            let code = match verdict {
                Verdict::Accepted => message::CHANGED,
                Verdict::Refused(Refusal::Unidentified) => message::UNAUTHORIZED,
                Verdict::Refused(Refusal::Forbidden) => message::FORBIDDEN,
                Verdict::Refused(Refusal::Unacceptable) => message::BAD_REQUEST,
                Verdict::Failed => message::SERVICE_UNAVAILABLE,
            };
            let answer = message::encode(&request.message.response(code, &[]))?;
            answering.send(&answer)?;
            answered.remember(&request, answer);
            Ok(())
        });
        Ok(Arrived::whole(origin, payload, acknowledgement))
    }
}
