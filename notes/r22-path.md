# R22 app-to-native path

Static analysis of RONGTA 3.1.6 (1028), using the ARM64 SDK identified in
[native-survey.md](native-survey.md). These are app-side findings, not hardware
test results. Addresses are from the current Ghidra import.

## Selection and transport

1. `com/rongta/pn81/models/PrintModelKt.java` maps the selected R22 to
   `DeviceModel.R22`.
2. `com/rongta/base/data/model/DeviceModel.java:89-110` places R22 in
   `MiniModels`, alongside R5 and R26. It is not in `LabelModels` or
   `DualColorModels`.
3. `qh/j0.java:701-707` therefore calls `NativeApis.setODMPrinterType(28)`,
   sets the Java command preference to `CmdType.ZPL`, and connects with
   `ConnectOptions.Builder().asSpp(mac)`.
4. The JNI setter at `0x0014e9bc` writes both the native ODM selection and
   `current_g_protocol_type_value`. The native enum names 28
   `RTProtocalRongTa`. `isNormalProtocol()` tests for zero, so it is false.
5. `com/rt/driver/bluetooth/SPPDriver.java:37-79` connects an RFCOMM socket
   using UUID `00001101-0000-1000-8000-00805F9B34FB`.

The Android R22 path is Bluetooth Classic SPP. BLE GATT services elsewhere
in the APK are not used by this connection path. The Java ZPL preference
does not make the traced raster output raw ZPL: it proceeds through the
proprietary native packet builder.

## Connection sequence

`SPPDriver` waits 500 ms after opening the socket, then requests native
`getProtocolData()`. A configured timeout runs while the SDK waits for the
protocol exchange. `BaseDriver.processData()` feeds received bytes through
`NativeApis.decodePackages()` and dispatches the resulting SDK events.

Command pairs below are hexadecimal **wire superclass/subclass**, not the
synthetic connection-event IDs returned to Java.

| Phase | Outgoing command pair | Evidence |
| --- | --- | --- |
| Discovery | `01/01`, followed by `01/06` | `getProtocolData`, `0x00140ae4`: concatenates two frames for Rongta. |
| Handshake | `01/02`, 16-byte proof | `getShakeData`, `0x00140bf4`: calls `getMD5` and sends its result. |
| Handshake | `01/17`, 19-byte payload | Same function: builds a random challenge payload. Exact encoding needs a closer pass. |
| Device queries | `01/03`, `01/04`, `01/0e` | Same function: appends three zero-payload query frames. |

`judgeShake` (`0x00124ee0`) parses discovery response JSON containing
`pVer`, `pAlgo`, and `pBuffLength`, and stores `pBuffLength` as the native
packet-buffer limit. It also copies the serial-number response's `sn` string
into `encryptionKeyStr`.

`getMD5` (`0x00122064`) hashes that serial-derived string followed by a
16-byte constant, then produces a 16-character hexadecimal result from
digest bytes 4 through 11. This is a device-data-dependent handshake; we
can reconstruct the algorithm without possessing this printer's serial.
The constant and challenge verification have not yet been fully documented.

For Rongta, `isFirstStepDone` waits for protocol and serial success, provided
later handshake stages have not failed. The decoder synthesizes
`RT_CONNECT_PROTOCOL` (Java event `0/0`), causing `BaseDriver` to send
`getShakeData()`.

`isConnect` (`0x00124be8`) requires successful protocol, serial, algorithm
setup, challenge, device info, model, and battery states on this branch.
The final synthetic `RT_CONNECT_HANDSHAKE` event (Java `0/1`) advances the
Java driver to connected. A socket opening alone is not SDK-level success.

## Packet framing and encryption scope

`HAL_RT_Protocol_PacketBuid` (`0x00122454`) is the common packet builder.
The selected branch prefixes frames with bytes `a3 1e`, followed by the
ODM/protocol byte `1c`. The cleartext layout has an 11-byte header, payload,
and a 4-byte CRC. Length fields, message type/subtype, superclass/subclass,
and the CRC calculation are visible in this function; a standalone encoder
still needs reference-vector validation.

`getEncryptType` (`0x00140ac0`) returns 2 only for `RTProtocalXiaoMi`, and
zero otherwise. Rongta's traced discovery, handshake, and raster commands
therefore do not enter the packet builder's AES payload-encryption branch.
This does not remove the MD5/challenge handshake, and says nothing about
Bluetooth link-layer encryption or untraced SDK operations.

## R22 image-print path

`ei/j.java:f()` identifies R22. In the R22/R26 branch of
`com/rt/rlabelnew/ui/entry/PrintEntryActivity.java:284-317`, the app crops and
scales the label at 8 pixels/mm, places it on a white canvas using the
native-reported printer width, and applies the configured offsets. It then
calls `RTSDK.printBitmap()`.

That screen's `initViews` sets print mode 0, selects
`RTImagePrintEffectTypeBlackAndWhite` (algorithm 0), and sets threshold 208.
Heat density comes from the user's saved configuration. This describes the
image-entry screen; other editing/printing screens may choose other settings.

`ApisImp.bitmapToPrintData()` passes row-major A,R,G,B bytes into:

```text
getImagePackageData(pixels, width, height, 0, algorithm, 0, 0, 0)
  -> getPrintImageData                       0x00147a4c
  -> getAllCompressImagePart                 0x0014695c
       -> getTreePack
       -> getCompressSendImageData           0x001465ac
            -> RTGetHfmTree / RTHFMDGencoding
```

For protocol 28, `getPrintImageData` selects compressed output; the
alternative `getAllImageData` branch is selected for Xiaomi/RuiDa.
The compression functions expose Huffman-tree structures and an encoder;
their exact bitstream format has not yet been reconstructed.

| Job stage | Command pair |
| --- | --- |
| Start | `05/0b` |
| Compression tree for a section | `05/0e` |
| Compressed data packets | `05/0d` |
| End | `05/0c` |

The compressor reduces sections to fit the packet-buffer limit and aligns
them to raster rows. `getCompressSendImageData` includes a packet index,
lengths, and byte width with each encoded packet.

`SppPrinter.print()` adds configured copies, optional metadata, speed, and
paper-feed commands via `BasePrinter.addPrintTimesAndFeedPaper()`, then sends
the list through `SppBluetoothPrinterCore` and `SPPDriver.write()`.
The latter writes and flushes a normal byte stream; list elements have a
configured inter-command delay. There is no BLE MTU/chunking step in this path.

## Focus for tomorrow

- Capture a connection starting before RFCOMM opens. Check `a3 1e 1c`
  framing and the two discovery commands first.
- Record the actual `sn`, `pVer`, `pAlgo`, and `pBuffLength` replies, then
  compare the five-command handshake and its acknowledgments.
- Print one small monochrome label from the image-entry screen. Compare
  setup commands and `05/0b -> (05/0e, 05/0d...) -> 05/0c` structure.
- Confirm the reported width, printed alignment, status messages, and pacing.

Useful work before arrival: reproduce the cleartext frame encoder and CRC,
generate reference packets by calling the original SDK offline, and map
the challenge and Huffman encoding. Actual printer replies are needed to
validate device behavior, not to identify which code implements it.
