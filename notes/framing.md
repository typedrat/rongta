# Rongta protocol 28 framing

Evidence: original RONGTA 3.1.6 ARM64 SDK output from the Android reference
harness, compared byte-for-byte with the Rust encoder. No hardware validation.
See `rongta-rs/tests/fixtures/sdk/framing.json` for library SHA-256, environment,
JNI inputs, and original output bytes.

## Cleartext layout

| Offset | Size | Meaning |
| --- | --- | --- |
| 0 | 3 | Magic and protocol: `a3 1e 1c` |
| 3 | 1 | Mode in high nibble; reserved low nibble is zero |
| 4 | 2 | Little-endian inner-message length: payload length + 5 |
| 6 | 1 | Message type in high nibble, subtype in low nibble |
| 7 | 1 | Superclass |
| 8 | 1 | Subclass |
| 9 | 2 | Little-endian payload length |
| 11 | N | Payload |
| 11 + N | 4 | Little-endian CRC |

The CRC covers bytes starting at offset 6 through the last payload byte.
It uses the reflected IEEE CRC-32 polynomial, shift-register initial value
`0x896acade`, and final XOR `0xffffffff`. Equivalently, Python's
`zlib.crc32(inner_message, 0x76953521)` gives the wire checksum. Rust's `crc`
crate reflects `Algorithm::init` when `refin` is true, so its initial parameter
is `0x7b535691`. The check value for `123456789` is `0xf3d84598`.

Discovery emits these two frames with mode 0 and type/subtype 1/1:

```text
a3 1e 1c 00 05 00 11 01 01 00 00 31 ee 35 9b
a3 1e 1c 00 05 00 11 01 06 00 00 b4 f8 7a 9e
```

The generic `getCommandWithBytArray` API sets mode **1** but still emits the
same cleartext layout. The native packet builder only takes its AES branch
for mode 2. Rust supports modes 0 and 1 and rejects other modes; mode 1's
semantic purpose remains unverified.

## Bounds and stream behavior

`HAL_RT_Packet_Decode` (`0x001213b4`) rejects inner-message lengths above
`0x802` (2050), so that is `FrameCodec::default()`'s limit. This is the SDK's
receive bound, not an established printer receive limit or negotiated send
limit. `FrameCodec::new` accepts an explicit inner-message bound, including
the five-byte inner header. The SDK builder generates larger messages too:
the corpus includes a 4096-byte payload.

The u16 outer length mathematically allows up to 65530 payload bytes; Rust
checks this before encoding, but printer support for that size is unknown.
Lengths are computed on every encode, so changing the payload cannot leave
stale stored lengths.

The codec buffers fragmented packets, yields concatenated packets separately,
and preserves unknown command IDs and payload bytes. It skips noise preceding
magic while retaining a possible partial magic prefix. After recognizing magic,
invalid lengths, reserved bits, unsupported modes, or a bad CRC return an error
without consuming the invalid frame. Callers should treat that as stream
failure; raw transport logging belongs before this decoder.

## Native evidence

- `HAL_RT_Protocol_PacketBuid`, `0x00122454`: header, CRC, mode 2 AES branch.
- `getProtocolData`, `0x00140ae4`: discovery command pair.
- `getCommandPackageWithData`, `0x0014a490`: mode 1 generic commands.
- `HAL_RT_Packet_Decode`, `0x001213b4`: receive length bound and CRC coverage.
- `RT_crc32tab`, `0x0010d5b4`: reflected IEEE lookup table.
