# R22 print-job submission

The first successful R22 hardware test is documented in [hardware.md](hardware.md).

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
delay and submits a complete image as one buffer. Additional inter-packet pacing
is available for diagnostics but produced incomplete output on the tested R22.

The image start/tree/data/end sequence is documented in [image-data.md](image-data.md).
The printer's reported head width and buffer length configure the encoder;
fixture values are simulated; the first physical R22 reports 384 dots and a
2050-byte buffer. Rust centers
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

The hardware sends `01/12` notifications using message type 1, subtype 0
(header `10`). Its status is a four-byte tag-1 TLV; tag 2 contains a one-byte
battery-power state. The original SDK successfully decodes the captured shapes.
The earlier eight rejected header probes remain as negative fixtures.

There is no verified job identifier. Each copy must produce an observed printing
state (queried or notified), followed by an idle status query. This keeps the
connection open through the printer's delayed start; initial idle responses no
longer count as completion. A short cycle can be detected through notifications
even if queries miss its busy state. A cycle that is never observed times out;
the CLI does not retry automatically. `sent_printer_idle` confirms this state
cycle, not that the image is visually correct.

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
