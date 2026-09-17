# SDK framing vectors

`framing.json` was generated with `tools/native-reference/generate.py` from the
ARM64 native library in RONGTA 3.1.6 (1028). The JSON records the exact library
SHA-256 and Android image fingerprint, plus every call's input and output.

Evidence level: **SDK-generated, no printer connected**. In particular,
`sequence-4096` establishes the builder's byte output; it does not establish
the printer's receive limit. The native SDK's receive decoder independently
limits inner-message length to 0x802 bytes.
