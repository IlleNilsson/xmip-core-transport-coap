//! What a Location of this technology is configured with: its declaration,
//! which `Configured` reads, built with the crate's `declaration` feature
//! alone and the code behind the default (ADR-0064, amendments 2026-10-22
//! and 2026-11-26), for the library the surfaces load to carry (open
//! problem 48). The acknowledgement wait a Location takes unless told
//! otherwise is declared here, and the code takes it from here.

use std::time::Duration;

use xcore::settings::{Applies, Fixed, Kind, Presence, Setting, Settings};

/// `ACK_TIMEOUT`, RFC 7252 section 4.8.
pub(crate) const ACK_TIMEOUT: Duration = Duration::from_secs(2);

/// Every setting a Location takes beyond its address, the local socket a
/// Receive Location binds — `0.0.0.0:5683` the standard port; a Send
/// Location sends to the `coap://host/path` its route gives.
pub const SETTINGS: &Settings = &Settings {
    technology: env!("CARGO_PKG_NAME"),
    settings: &[
        Setting {
            name: "method",
            kind: Kind::Choice {
                choices: &["POST", "PUT"],
            },
            presence: Presence::Optional,
            meaning: "The request method a Stream is sent with; POST when left out.",
            applies: Applies::Send,
        },
        Setting {
            name: "confirmable",
            kind: Kind::Boolean,
            presence: Presence::Optional,
            meaning: "Whether a request awaits its acknowledgement, retransmitted until \
                      it comes; confirmable when left out.",
            applies: Applies::Send,
        },
        Setting {
            name: "ack_timeout",
            kind: Kind::Duration,
            presence: Presence::Default(Fixed::Duration(ACK_TIMEOUT)),
            meaning: "How long the first acknowledgement is waited for, doubled on each \
                      retransmission.",
            applies: Applies::Send,
        },
        transport::configured::timeout(
            Applies::Receive,
            None,
            "How long a receive waits for a request; unbounded when left out.",
        ),
    ],
};

/// The schemes a Location's address is entered with to choose this technology.
pub const SCHEMES: &[&str] = &["coap"];
