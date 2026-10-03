# xmip-core-transport-coap

CoAP transport: one request is one Stream, the resource path beside it; confirmable requests acknowledged and retransmitted per RFC 7252, over UDP. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A Send Location's request and its answer go through one socket per address family, bound on the first send and kept by the transport (`transport::sender::Sender`), whatever came late read off before the next request, so an IPv6 target is reached too; until 2026-09-27 every send bound a new IPv4 socket.

A Receive Location keeps its socket, bound on the first receive (`transport::kept::Kept`): a datagram that arrives between two receives waits in its buffer for the next, where until 2026-09-27 each receive bound a socket of its own and a datagram sent between receives was lost.

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls. Until 2026-09-28 this technology stripped its scheme by hand.

## Acknowledgement

A confirmable request is answered after the whole receive cycle, piggybacked
on its ACK. On Accepted the answer is 2.04 Changed. On Refused it is a 4.xx,
the client error RFC 7252 section 5.9.2 says is not repeated unchanged: 4.01
Unauthorized for a sender not identified, 4.03 Forbidden for one not
permitted, 4.00 Bad Request for content refused. On Failed it is 5.03 Service
Unavailable (section 5.9.3.4), which tells the device to send it again (and a send of
this transport answered 5.xx fails as retryable, 4.xx as permanent). Until the
answer goes the device retransmits as RFC 7252 section 4.2 says; a
retransmission of a request already answered is answered again from the
answers last sent and not taken twice (section 4.5), and one whose answer never
went is a request again. A non-confirmable request expects no answer, so
acceptance is at-most-once there. Each request arrives whole.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
