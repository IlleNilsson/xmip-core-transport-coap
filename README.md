# xmip-core-transport-coap

CoAP transport: one request is one Stream, the resource path beside it; confirmable requests acknowledged and retransmitted per RFC 7252, over UDP. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
