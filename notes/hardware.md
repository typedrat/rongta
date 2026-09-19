# First R22 hardware test — 2026-09-18

The user confirmed that the small border/block/stripe label printed correctly
using the original-SDK-equivalent image job submitted as one buffer, with the
connection retained to observe printing. Earlier attempts failed or were partial.
Local captures are under ignored `captures/first-hardware/`.

## Measured device information

| Field | Value |
| --- | --- |
| Model | R22 |
| Firmware | `V1.00 260119` |
| Hardware | `A70B1C3D1F1` |
| Protocol version / algorithm | 23 / 0 |
| Buffer length | 2050 bytes |
| Resolution / head width | 203 dpi / 384 dots |
| Product type / supported paper-size value | 2 / 3 |
| Initial battery | 3.83 V, 30%, state 0 |
| Paper loaded | Individual gap labels, approximately 2 × 1 inches (user estimate) |

BlueZ discovered the device's printer class and SPP UUID, paired without an
explicit PIN, and resolved the RFCOMM service. All handshake stages passed
without protocol changes. These are measured values, unlike the invented fields
in the original SDK handshake fixture.

## Failed and successful transfers

1. Default frame-by-frame writes followed by the first idle query: no output.
2. The same frames with 100 ms between them: a printing/idle cycle, but the user
   saw only about 1 mm of marks at the label's top edge.
3. The complete 1088-byte image job in one write, with the connection held for
   five seconds: the user reported that the label worked perfectly.

The image in all three attempts is 384 × 64 dots after centering the 128 × 64
source on the printer's full-width canvas. The original ARM64 SDK generated
**exactly the same 1088 image bytes** from the same raster and the printer's
actual discovery replies (`native-image.json` / `native-image.bin`). Thus the
failed test was not a Rust/SDK encoder mismatch for this raster.

The successful probe differs in both write grouping and connection lifetime.
It establishes a working combination, not independent proof of each firmware
failure mechanism. Android's `SPPDriver.write(List<byte[]>)` submits each complete
image as one buffer. The normal Rust path now does the same; the kernel may still
split writes into RFCOMM packets. Explicit packet pacing remains a diagnostic
option and is not the recommended setting for this device.

The successful trace contains idle replies before the printer starts. Closing
on the first idle reply is therefore unsafe. The CLI now requires a printing
state followed by an idle reply for each copy, and processes unsolicited status
while writing/waiting. It times out if that cycle is not observed; there is no
automatic resend. Physical image quality still requires looking at the label.

The repaired normal `print` CLI was then run with default settings. It stayed
connected past the initial idle reply, observed notification/query status 128,
and completed only after status returned to zero. That run is preserved in
`print-fixed-cli.json` / `print-fixed-cli.log` (1198 submitted bytes including
setup and status queries).

## Status notifications

The observed header is `10`: message type 1, subtype 0, command `01/12`.
Payloads contain tag 1 with a four-byte little-endian status, followed by tag 2
with a one-byte battery-power state. Printing is bit 7 (`128`); idle is zero.
Both idle and printing notification shapes were fed to the original SDK and
successfully decoded; see `hardware-notify-*` in the native print fixture.
The earlier eight header probes did not include `10` and remain negative tests.

The capture also contains a `07/01` notification; its semantics are not yet
established. It is retained in the raw log and ignored by the print session.

## Evidence and remaining checks

`info.json` / `info.log` record the initial measured handshake. `print*.log`
record the transfer attempts. `settings-probe.log` records read-only queries of
speed, paper type, busy state, density, and feed settings. The print-mode query
received no reply; an acknowledgment of that optional setup is not required.

Regression tests cover one-buffer submission, delayed start, a short cycle
reported through notifications, per-copy state tracking, and fault notifications
interrupting blocked writes. Broader media sizes, longer images, explicit speed
or feed changes, and repeated-copy physical alignment remain to be tested.

## Spoolman label trial

The user's PNG was 399 × 199 pixels. At threshold 208 its ink bounds were
`(16,16)` through `(303,184)`. A temporary PBM print raster retained columns
0–383, discarding only white right padding. It did not resize the QR code or
text. The original PNG was not modified.

The normal CLI submitted this 384 × 199 job with default pacing and density.
It observed printing and then idle, with no faults, and reported 6102 submitted
bytes including controls/status queries. `spool-label-8.pbm`, `.json`, and `.log`
are in the local capture directory. Visual placement and QR readability require
user confirmation; protocol completion alone does not establish either.
