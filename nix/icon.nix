# Every raster of the app icon, rendered from the one SVG kept in the repo.
{
  runCommand,
  resvg,
  icoutils,
  libicns,
}:
runCommand "rongta-icon"
  {
    nativeBuildInputs = [
      resvg
      icoutils
      libicns
    ];
  }
  ''
    mkdir -p $out/png
    for size in 16 24 32 48 64 128 256 512 1024; do
      resvg -w $size -h $size ${../rongta-rs/gui/assets/icon.svg} $out/png/$size.png
    done
    # Windows picks the closest size per context: 16 and 32 for lists and
    # title bars, 48 for Explorer's medium icons, 256 for large ones.
    icotool -c -o $out/rongta-labels.ico \
      $out/png/{16,24,32,48,64}.png -r $out/png/256.png
    png2icns $out/rongta-labels.icns \
      $out/png/{16,32,128,256,512,1024}.png
  ''
