# xmip-core-transport-coap

CoAP transport: one request is one Stream, the resource path beside it; confirmable requests acknowledged and retransmitted per RFC 7252, over UDP. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A Send Location's request and its answer go through one socket per address family, bound on the first send and kept by the transport (`transport::sender::Sender`), whatever came late read off before the next request, so an IPv6 target is reached too; until 2026-09-27 every send bound a new IPv4 socket.

A Receive Location keeps its socket, bound as its transport is opened (`Configured::listening`; on its first receive until 2026-11-16) (`transport::kept::Kept`): a datagram that arrives between two receives waits in its buffer for the next, where until 2026-09-27 each receive bound a socket of its own and a datagram sent between receives was lost.

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls. Until 2026-09-28 this technology stripped its scheme by hand.

A Send Location's target is its whole address, `coap://host:port/path`, which a send reads the resource from (`Configured::own_target`); until 2026-11-25 it was the address without its scheme, and every send of a node was refused. The loopback's server takes POSTs until one shorter than a message carries closes the Stream, as a last block does, so it takes a node's one POST whole, and the near end sends an empty POST only after a full one; until 2026-11-25 only an empty POST closed it, and a node's POST was never taken.

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

## The declaration

What a Location of this technology is configured with is two `const`s in its `declaration` module, `src/declaration.rs`, beside the code, which `Configured` reads (ADR-0064, amendment 2026-11-26): `declaration::SETTINGS`, `method`, `confirmable`, `ack_timeout` and `timeout`, and `SCHEMES`, `coap`. The acknowledgement wait a Location takes unless told otherwise, RFC 7252's `ACK_TIMEOUT`, is declared there too, and the code takes it from there. Built with its `declaration` feature alone (`--no-default-features --features declaration`), the crate is those and nothing more: its code and every crate the code is built on are behind the default feature. So the library the surfaces load carries it without the code, through the gathering crate xmip-core-declaration (open problem 48), and the language server and the desktop suggest and check a CoAP Location's settings though no node in their process loaded the technology. `test/Declaration.Test.ps1` in the estate holds that the declaration builds nothing the transport capability does not.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
