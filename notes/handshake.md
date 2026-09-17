# Protocol 28 handshake and information

Evidence: ARM64 SDK SHA-256
`4bbef7e782ac6ef37b758cba7aadf61ca29ad957870e3c5f96e23de5efd44695`,
RONGTA 3.1.6 (1028), ODM 28. Ghidra addresses below include its `0x100000`
image base. The Android harness executes the original library against simulated
printer replies. No hardware exchange has been captured yet.

## Exchange

The Android R22 path uses Classic Bluetooth SPP UUID
`00001101-0000-1000-8000-00805f9b34fb`. It waits 500 ms after connecting before
sending `getProtocolData()` (`0x140ae4`): empty requests `01/01` and `01/06`.
All requests here use mode 0, message type 1, subtype 1. Successful responses
use type 2, subtype 1; subtype 2 indicates failure.

Once protocol and serial responses arrive, `getShakeData()` (`0x140bf4`)
produces five concatenated frames, in this order:

| Command | Request payload | Successful reply |
| --- | --- | --- |
| `01/02` | 16 ASCII proof characters | Success header is sufficient |
| `01/17` | TLV tag 1, 16 ASCII random hex characters | String TLVs below |
| `01/03` | Empty | Device info JSON |
| `01/04` | Empty | Model numeric TLVs |
| `01/0e` | Empty | Battery numeric TLVs |

`getMD5()` (`0x122064`) computes the proof:

```text
hex_lower(MD5(serial_bytes || "D932A43C3084B1D3")[4..12])
```

The SDK hex-encodes eight low bytes from `rand()` for its request nonce.
Rust uses eight OS random bytes in the same format. The randomness source is
not part of the wire protocol; fixture tests inject the SDK's captured nonce.

`isCheckMD5Success` checks the returned challenge's MD5 against:

```text
hex_lower(MD5(returned_shakeCode_bytes || proof_ascii))
```

It does **not** require the returned challenge to echo the request nonce, or
compare the challenge serial to the discovery serial. The harness confirms a
different returned challenge succeeds. Both serials are retained in CLI output.
`judgeShake` (`0x124ee0`) / `isConnect` (`0x124be8`) require all seven successful
steps; an acknowledgement alone does not establish a completed handshake.

## Reply payloads

TLV encoding is `tag:u8 | length:u16 little-endian | value:length bytes`.
`analysisDatas` (`0x122c90`) interprets numeric values as little-endian integers.
Rust accepts lengths 1–4 and rejects truncated records, duplicate tags, missing
required fields, and oversized integers. Unknown tags are skipped. These
validation choices are stricter than the SDK's unsafe C parser.

| Command | Fields |
| --- | --- |
| `01/01` | Numeric: 1 `pVer`, 2 `pAlgo`, 3 `pBuffLength` |
| `01/06` | Raw serial string, no TLV wrapper |
| `01/17` | Strings: 1 `shakeCode`, 2 `md5` (32 ASCII hex), 3 `sn`, 4 `authCode` |
| `01/03` | JSON: nonempty `DevName`, `Firmware`, `HWVer`, `SN`; optional `Manufacturer` (defaults to Rongta) |
| `01/04` | Numeric: 1 `devProdType`, 2 `supportPaperSize`, 3 `dpi`, 4 `widthMaxDot` |
| `01/0e` | Numeric: 1 voltage in hundredths V, 2 percent in tenths %, 3 battery state |

The SDK converts these payloads to JSON before Java receives them; that JSON is
not the wire layout except for `01/03`. Enum meanings for product type, paper
size, battery state, and `pAlgo` remain open. In particular, `getShakeData()`
does not branch on `pAlgo`: it emits the cleartext commands above for ODM 28.
The reported buffer length is exposed as data, not applied as a frame limit;
its exact unit/scope still needs tracing. Receive decoding retains the SDK's
2050-byte inner-body bound. Encrypted mode 2 frames are unsupported.

## Reference tests and remaining hardware checks

`rongta-rs/tests/fixtures/sdk/handshake.json` records generated requests, every
simulated reply, and native callbacks for two accepted serials and one rejected
challenge digest. Model values (203 dpi, 384 dots, product type 22, etc.) are
**invented fixture data, not measured R22 capabilities**. The Rust tests compare
request bytes and decoded information with these SDK observations, and cover
failure, reordering, duplicates, and malformed input.

Still to verify on the printer: pairing/PIN requirements, SDP service availability,
actual field values and encodings, response timing, and whether an additional
app-side initialization step matters for printing. This command only establishes
the handshake and reads information; raster generation/printing is separate.
