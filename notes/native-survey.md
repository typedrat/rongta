# Native SDK: JNI and debug metadata survey

## Inputs and identity

- RONGTA 3.1.6, version code 1028, ARM64 APK variant.
- Binary: `analysis/native/arm64-v8a/librt_printer_sdk.so`.
- Binary MD5: `b812f49537eccf651d056d828e2b6229`, matching the open Ghidra program.
- Ghidra program: `/librt_printer_sdk.so`, `AARCH64:LE:64:v8A`.
- Addresses below are Ghidra addresses; this import adds `0x100000` to ELF symbol values.
- The base APK's `classes.dex` and `classes2.dex` are byte-identical between
  the downloaded ARM64 and original ARMv7 variants. The existing JADX output
  in `analysis/jadx/sources/` therefore applies to both.

## Metadata is already available

The library retains `.debug_info`, `.debug_abbrev`, `.debug_str`, `.debug_line`,
`.debug_loc`, `.debug_ranges`, and `.symtab`. Ghidra has imported DWARF types,
parameter names, structures, and enums, including `JNIEnv` and its function
table. Calls already resolve to `GetByteArrayElements`, `NewByteArray`,
`SetByteArrayRegion`, `FindClass`, and `GetMethodID`.

All 36 native declarations in `com/rt/sdk/NativeApis.java` have matching
`Java_com_rt_sdk_NativeApis_*` exports. A signature audit checked all 36 for
the `JNIEnv *` argument, argument count, Java argument types, and return type.
The complete local inventory is `analysis/jni-inventory.json`.

There is one additional exported wrapper, `setPrinterInfoButtonEnableData`,
absent from this Java class. Do not assume the app calls it.

All declarations in this Java class are static. In 21 wrappers the native
debug information names the second argument `jobject thiz`; the others use
`jclass clazz`. For static JNI calls this argument represents the class.
These reference types have compatible pointer representations here; preserve
the original debug information rather than treating `thiz` as an instance.
See the [JNI design specification](https://docs.oracle.com/en/java/javase/17/docs/specs/jni/design.html).

No generic JNI header import or guessed signature replacement was needed.
Optimized locals and some aggregate values remain imperfect in decompilation;
debug metadata is not a guarantee that every reconstructed expression is correct.

## Entry points for the next pass

| Area | Java native entry point | Ghidra address | Observed native path |
| --- | --- | --- | --- |
| Initialization | `initProtocol` | `0x0014ea54` | Selects `RTProtocalRongTa`; initializes protocol state to `0x1c`. |
| Discovery | `getProtocolData` | `0x0014dec4` | Copies the result of native `getProtocolData` at `0x00140ae4` into a Java byte array. |
| Handshake | `getShakeData` | `0x0014df38` | Copies native `getShakeData` at `0x00140bf4`; implementation has vendor-dependent branches. |
| Raster output | `getImagePackageData` | `0x0014dfac` | Builds an image and dispatches to `getPrintImageData` or `getInkjetPrintImageData`. |
| Responses | `decodePackages` | `0x0014d1e4` | Builds Java `ArrayList`/response objects and feeds native `getJsonString`. |

These five entry points have Ghidra Note bookmarks in category
`Rongta JNI survey`. Existing function names and signatures were preserved.

`getImagePackageData` retains the native parameter names `rgb_pixel_bytes`,
`width`, `height`, `reversed`, `model`, `copies`, `isHaveNext`, and `isNewAlgo`.
The parameter named `model` participates in image-format/algorithm selection;
its name alone does not identify the selected printer model.

The packet container is already typed from `src/main/c/type.h`:

```c
struct RTProtocolPack {
    U8 *packData;  // offset 0, 8 bytes
    U64 packLength; // offset 8, 8 bytes
};                 // 16 bytes total, alignment 8
```

## Scope and next evidence

This is an entry-point map, not an R22 protocol specification. Initialization
state and bundled vendor branches do not establish which path an R22 uses.
Next, correlate a real R22 connection and tiny-label capture with discovery,
handshake, raster output, and response decoding. The retained SDK metadata
substantially reduces the work needed to understand those paths.
