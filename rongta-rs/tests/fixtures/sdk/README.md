# Original SDK reference vectors

`framing.json` was generated with `tools/native-reference/generate.py` from the
ARM64 native library in RONGTA 3.1.6 (1028). The JSON records the exact library
SHA-256 and Android image fingerprint, plus every call's input and output.

Evidence level: **SDK-generated, no printer connected**. In particular,
`sequence-4096` establishes the builder's byte output; it does not establish
the printer's receive limit. The native SDK's receive decoder independently
limits inner-message length to 0x802 bytes.

`handshake.json` records simulated printer replies, the SDK's resulting
handshake requests, and its success/failure callbacks. It is not a hardware
capture.

`image.json` records seven ARGB conversion cases and 12 packed-raster cases.
Image buffer limits are configured through simulated protocol replies. The
one-byte raster is explicitly marked unsupported because the SDK emits a
malformed empty packet; the other 18 jobs are byte-for-byte Rust references.
The fixture includes every input, native output, library hash, and Android
fingerprint. See `notes/image-data.md` and the harness README for interpretation
and regeneration instructions.
