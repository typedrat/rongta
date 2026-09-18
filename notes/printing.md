# R22 print-job submission

The implementation uses one BlueR SPP connection and one persistent frame decoder
for handshake, setup, image writes, and status queries. Buffered bytes survive
phase changes. During writes it drains replies concurrently, so a received fault
or rejection stops a blocked transfer. Partial writes are never restarted.

## App and native evidence

The original ARM64 SDK's generated command bytes are preserved in
`rongta-rs/tests/fixtures/sdk/print.json`, regenerated with the native harness's
`--suite print`. Command payloads below exclude the outer frame and CRC. All
requests use mode 0, message type 1, subtype 1.

| Command | Class/subclass | Payload |
| --- | --- | --- |
| Query print status | `01/13` | empty |
| App connected | `01/1e` | `01` |
| Print mode | `05/29` | `00` |
| Heat density | `05/04` | `03 08 N`, N=0–15 |
| Optional speed | `05/07` | raw u8 |
| Optional forward feed | `05/08` | `01` then raw u16 LE length |

`PrintEntryActivity` selects mode 0, raster algorithm 0, threshold 208, and
heat density. The app's density enum maps light/normal/dark to strings `8,0`,
`8,8`, `8,15`; the two-argument native encoder produces the payload above.
`BasePrinter.addPrintTimesAndFeedPaper` repeats the image, optionally sets speed,
and optionally appends forward feed. Defaults are one copy, no automatic speed
selection, and zero extra feed. Physical speed/feed units remain unverified.
`SPPDriver.write(List)` waits `RTSDKConfig.cmdIntervalTime` (100 ms) before each
list item and writes the image byte array as one batch. Rust keeps that command
delay and permits additional inter-packet pacing for experiments.

The image start/tree/data/end sequence is documented in [image-data.md](image-data.md).
The printer's reported head width and buffer length configure the encoder;
fixture values are simulated and are not measured R22 capabilities. Rust centers
a narrower, byte-padded raster on a full-width white canvas, refuses cropping,
and caps the expanded raster at 64 MiB.

## Status and completion

The native decoder accepts simulated `01/13` replies with message type 2,
subtype 1 and TLV tag 1 containing a little-endian integer. Java's
`PrinterStatusPareseUtils.parseStatus` supplies these bit meanings:

| Bit | Meaning |
| --- | --- |
| 0–6 | cover open, out of paper, paper jam, overheating, missing head, high voltage, low voltage |
| 7 | printing |
| 8–11 | cutter, font memory, PSRAM, paper learning errors |
| 12–15 | about to shut down, shutting down, black-mark paper, NFC errors |

All defined bits other than printing, and any unknown upper bits, stop the job.
A busy preflight status also prevents submission. Negative acknowledgments for
print controls fail immediately. Rust polls after each copy until idle; the
whole submission/wait sequence has a fixed deadline. It does not assume every
image packet receives an acknowledgment.

The app also handles `01/12` notifications, but the native decoder rejected all
eight tested message-type/subtype combinations with our simulated TLV payload.
Their framing is unresolved; Rust ignores them rather than claiming validated
notification support. Consequently fault detection currently depends on valid
`01/13` replies, not a guaranteed asynchronous printer notification.

There is no verified job identifier or paper-completion event. The report
`sent_printer_idle` means local writes completed and a subsequent status reply
was idle. An initial idle reply could precede the printer starting, and a short
job may finish between polls. `observed_printing` exposes that distinction;
hardware captures must establish stronger completion semantics if needed.

## Verification and first hardware test

Tests compare controls and status decoding to the original SDK. A simulated
32-byte duplex connection exercises fragmented handshake replies, full image
bytes, two copies, optional speed/feed, and busy-to-idle replies. Other cases
cover faults buffered with the final handshake reply, faults/rejections while
image writes block, busy preflight, invalid geometry/buffer limits, silent peers,
and absent final status without retransmission. These do not exercise BlueZ,
radio transport, printer firmware, or the mechanism.

Start with `examples/first-label.png`, one copy and defaults. Save `--verbose`
logs and the JSON report. Check the border, solid block at upper left, stripe
orientation, margins, and actual paper movement. Compare with a stock-app HCI
capture if setup, pacing, or completion differs. Then try two copies and only
change speed/feed after the baseline works. Automatic reconnect/retry and
cancellation commands are deliberately absent until their semantics are known.
