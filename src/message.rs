//! RFC 7252 section 3: the four-byte header, the token, the options with
//! their delta encoding, the payload marker, and the payload.

use transport::error::{Result, protocol_error};

/// The largest datagram a CoAP endpoint must take, RFC 7252 section 4.6.
pub const MAX_DATAGRAM: usize = 1152;
/// The most payload one message carries without block-wise transfer.
pub const MAX_PAYLOAD: usize = 1024;

pub const URI_PATH: u16 = 11;
pub const CONTENT_FORMAT: u16 = 12;
pub const URI_QUERY: u16 = 15;

pub const GET: u8 = 0x01;
pub const POST: u8 = 0x02;
pub const PUT: u8 = 0x03;
pub const DELETE: u8 = 0x04;
/// 2.01 Created.
pub const CREATED: u8 = 0x41;
/// 2.04 Changed.
pub const CHANGED: u8 = 0x44;
/// 2.05 Content.
pub const CONTENT: u8 = 0x45;
/// 4.00 Bad Request.
pub const BAD_REQUEST: u8 = 0x80;
/// 4.13 Request Entity Too Large.
pub const TOO_LARGE: u8 = 0x8d;

/// The message type, two bits of the first byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Confirmable,
    NonConfirmable,
    Acknowledgement,
    Reset,
}

/// One CoAP message, split.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub kind: Kind,
    /// `c.dd` packed as three bits of class and five of detail.
    pub code: u8,
    pub id: u16,
    pub token: Vec<u8>,
    /// Option number and value, in the order they are sent.
    pub options: Vec<(u16, Vec<u8>)>,
    pub payload: Vec<u8>,
}

impl Message {
    /// A request for `path` carrying `payload`.
    #[must_use]
    pub fn request(kind: Kind, code: u8, id: u16, path: &str, payload: &[u8]) -> Self {
        let options = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(|segment| (URI_PATH, segment.as_bytes().to_vec()))
            .collect();
        Self {
            kind,
            code,
            id,
            token: id.to_be_bytes().to_vec(),
            options,
            payload: payload.to_vec(),
        }
    }

    /// The answer to `self`: piggybacked on the ACK when confirmable, a
    /// non-confirmable message otherwise, under the same token.
    #[must_use]
    pub fn response(&self, code: u8, payload: &[u8]) -> Self {
        Self {
            kind: match self.kind {
                Kind::Confirmable => Kind::Acknowledgement,
                _ => Kind::NonConfirmable,
            },
            code,
            id: self.id,
            token: self.token.clone(),
            options: Vec::new(),
            payload: payload.to_vec(),
        }
    }

    /// The Uri-Path options joined: `sensors/1`.
    #[must_use]
    pub fn uri_path(&self) -> String {
        self.options
            .iter()
            .filter(|(number, _)| *number == URI_PATH)
            .map(|(_, value)| String::from_utf8_lossy(value).into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// The code as it is written: `2.04`.
    #[must_use]
    pub fn code_text(&self) -> String {
        format!("{}.{:02}", self.code >> 5, self.code & 0x1f)
    }

    /// Whether the code is a request, class 0.
    #[must_use]
    pub const fn is_request(&self) -> bool {
        self.code >> 5 == 0 && self.code != 0
    }
}

/// Encode `message` as a datagram.
///
/// # Errors
/// A token over eight bytes, or a message over [`MAX_DATAGRAM`].
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    if message.token.len() > 8 {
        return Err(protocol_error("a token over eight bytes"));
    }
    let kind = match message.kind {
        Kind::Confirmable => 0,
        Kind::NonConfirmable => 1,
        Kind::Acknowledgement => 2,
        Kind::Reset => 3,
    };
    let mut out = vec![
        0x40 | (kind << 4) | u8::try_from(message.token.len()).unwrap_or(0),
        message.code,
    ];
    out.extend_from_slice(&message.id.to_be_bytes());
    out.extend_from_slice(&message.token);
    let mut options: Vec<&(u16, Vec<u8>)> = message.options.iter().collect();
    options.sort_by_key(|(number, _)| *number);
    let mut last = 0u16;
    for (number, value) in options {
        let delta = number - last;
        last = *number;
        let (delta_nibble, delta_extra) = extend(delta);
        let length = u16::try_from(value.len())
            .map_err(|_| protocol_error("an option value over what CoAP can frame"))?;
        let (length_nibble, length_extra) = extend(length);
        out.push((delta_nibble << 4) | length_nibble);
        out.extend_from_slice(&delta_extra);
        out.extend_from_slice(&length_extra);
        out.extend_from_slice(value);
    }
    if !message.payload.is_empty() {
        out.push(0xff);
        out.extend_from_slice(&message.payload);
    }
    if out.len() > MAX_DATAGRAM {
        return Err(protocol_error(
            "a message over what a CoAP datagram carries",
        ));
    }
    Ok(out)
}

/// The nibble and the extension bytes for one delta or length.
fn extend(value: u16) -> (u8, Vec<u8>) {
    match value {
        0..=12 => (u8::try_from(value).unwrap_or(0), Vec::new()),
        13..=268 => (13, vec![u8::try_from(value - 13).unwrap_or(0)]),
        _ => (14, (value - 269).to_be_bytes().to_vec()),
    }
}

/// Decode one datagram.
///
/// # Errors
/// Not CoAP version 1, a token longer than the datagram, an option that
/// runs past the end, or a reserved nibble.
pub fn decode(datagram: &[u8]) -> Result<Message> {
    if datagram.len() < 4 || datagram[0] >> 6 != 1 {
        return Err(protocol_error("a datagram that is not CoAP version 1"));
    }
    let kind = match (datagram[0] >> 4) & 0x03 {
        0 => Kind::Confirmable,
        1 => Kind::NonConfirmable,
        2 => Kind::Acknowledgement,
        _ => Kind::Reset,
    };
    let token_length = usize::from(datagram[0] & 0x0f);
    if token_length > 8 {
        return Err(protocol_error("a token length over eight"));
    }
    let code = datagram[1];
    let id = u16::from_be_bytes([datagram[2], datagram[3]]);
    let token = datagram
        .get(4..4 + token_length)
        .ok_or_else(|| protocol_error("a token longer than the datagram"))?
        .to_vec();
    let mut at = 4 + token_length;
    let mut options = Vec::new();
    let mut last = 0u16;
    while at < datagram.len() && datagram[at] != 0xff {
        let byte = datagram[at];
        at += 1;
        let delta = extended(datagram, &mut at, byte >> 4)?;
        let length = usize::from(extended(datagram, &mut at, byte & 0x0f)?);
        last = last
            .checked_add(delta)
            .ok_or_else(|| protocol_error("an option number over what CoAP has"))?;
        let value = datagram
            .get(at..at + length)
            .ok_or_else(|| protocol_error("an option that runs past the datagram"))?;
        options.push((last, value.to_vec()));
        at += length;
    }
    let payload = if at < datagram.len() {
        if datagram.len() == at + 1 {
            return Err(protocol_error("a payload marker with no payload"));
        }
        datagram[at + 1..].to_vec()
    } else {
        Vec::new()
    };
    Ok(Message {
        kind,
        code,
        id,
        token,
        options,
        payload,
    })
}

fn extended(datagram: &[u8], at: &mut usize, nibble: u8) -> Result<u16> {
    match nibble {
        0..=12 => Ok(u16::from(nibble)),
        13 => {
            let byte = *datagram
                .get(*at)
                .ok_or_else(|| protocol_error("an option extension past the datagram"))?;
            *at += 1;
            Ok(u16::from(byte) + 13)
        }
        14 => {
            let bytes = datagram
                .get(*at..*at + 2)
                .ok_or_else(|| protocol_error("an option extension past the datagram"))?;
            *at += 2;
            u16::from_be_bytes([bytes[0], bytes[1]])
                .checked_add(269)
                .ok_or_else(|| protocol_error("an option extension over what CoAP has"))
        }
        _ => Err(protocol_error("the reserved option nibble 15")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_round_trips_with_its_options_in_order() {
        let mut request = Message::request(Kind::Confirmable, POST, 0x1234, "/sensors/1/", b"21.5");
        request.options.push((CONTENT_FORMAT, vec![0]));
        request.options.push((URI_QUERY, b"unit=c".to_vec()));
        request.options.push((2000, vec![0x2a; 300]));
        let bytes = encode(&request).expect("encode");
        assert_eq!(&bytes[..4], &[0x42, POST, 0x12, 0x34]);
        let back = decode(&bytes).expect("decode");
        assert_eq!(back.uri_path(), "sensors/1");
        assert_eq!(back.payload, b"21.5");
        assert_eq!(back.token, [0x12, 0x34]);
        assert_eq!(back.options.len(), 5);
        assert_eq!(back.options[4].0, 2000);
        assert_eq!(back.options[4].1.len(), 300);
        assert!(back.is_request());
        let response = back.response(CHANGED, b"");
        assert_eq!(response.kind, Kind::Acknowledgement);
        assert_eq!(response.code_text(), "2.04");
        assert!(!response.is_request());
        let bytes = encode(&response).expect("encode");
        assert_eq!(decode(&bytes).expect("decode"), response);
    }

    #[test]
    fn what_is_not_coap_is_refused() {
        assert!(decode(&[0x00, 0x01, 0, 0]).is_err(), "version 0");
        assert!(decode(&[0x49, 0x01, 0, 0]).is_err(), "token length 9");
        assert!(decode(&[0x42, 0x01, 0, 0, 1]).is_err(), "short token");
        assert!(decode(&[0x40, 0x01, 0, 0, 0xf0]).is_err(), "nibble 15");
        assert!(decode(&[0x40, 0x01, 0, 0, 0xd0]).is_err(), "no extension");
        assert!(decode(&[0x40, 0x01, 0, 0, 0x12, b'a']).is_err(), "past");
        assert!(decode(&[0x40, 0x01, 0, 0, 0xff]).is_err(), "empty payload");
        let big = Message::request(Kind::Confirmable, POST, 1, "p", &[0; 1200]);
        assert!(encode(&big).is_err(), "over the datagram");
    }
}
