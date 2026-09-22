# Stock app name filtering

`ScanDevice::is_printer_candidate` uses the name-filtering rules from the normal
RONGTA 3.1.6 app channel. Both CLI scan output and the settings picker call it.
The discovery transport, BlueZ cache behavior, and `scan --all` bypass are unchanged.

The decompiled sources in the ignored `analysis/jadx/sources/` directory are:

- `com/rongta/pn81/ui/SearchBluetoothDeviceActivity.java`, `filterDevice`: reject
  blank names and case-insensitive `BLE` endings, then match any `PrintModel`.
- `com/rongta/pn81/models/PrintModel.java`: the 23 model names.
- `qh/j0.java`, method `d`: uppercase using `Locale.ROOT` and match
  `^MODEL([-_ ].*)?$`, or `^MODEL[A-Z0-9]{0,3}([-_ ].*)?$` for variant names.
  Selection of the variant pattern checks case-sensitive prefixes on the
  **original** name: R5, R22, R26, TM95, TD210, TH110, RP425, RP420, RP415, RP310D.

The Rust matcher uses string operations instead of adding a regex dependency.
It preserves the original case-sensitive variant selection and Java regex dot's
exclusion of LF, CR, NEL, line separator, and paragraph separator. The name
filter does not use Bluetooth class, SPP services, or our supported print models.

This is a name-filter port. Android's BLE device-type exclusion (with its X21
exception) and the tattoo/X21-test app-channel restrictions are outside its scope.
BlueZ's Device1 API has no equivalent of Android's Classic/BLE/dual type, and
public addresses can belong to either Classic or LE devices; see the
[BlueZ device API](https://github.com/bluez/bluez/blob/master/doc/org.bluez.Device.rst).
Consequently, an LE-only device with an accepted name can still be listed.

## Reference verification

`tools/StockDeviceFilterReference.java` extracts the normal-channel name rules
and original Java regex matcher. It runs on a JDK without Android dependencies:

```sh
java tools/StockDeviceFilterReference.java R22 R22Pro r22Pro R22_BLE
# true, true, false, false (one result per line)
```

`rongta-rs/tests/fixtures/stock-device-filter.json` contains reference outputs
for all models, suffix boundaries, case differences, generic names, BLE endings,
Unicode, and control characters. Verify the named cases from the repository root:

```sh
python3 - <<'PY'
import json
import subprocess
from pathlib import Path

cases = json.loads(Path('rongta-rs/tests/fixtures/stock-device-filter.json').read_text())
named = [case for case in cases if case['name'] is not None]
actual = subprocess.check_output(
    ['java', 'tools/StockDeviceFilterReference.java', *[case['name'] for case in named]],
    text=True,
).splitlines()
assert actual == [str(case['expected']).lower() for case in named]
assert all(not case['expected'] for case in cases if case['name'] is None)
print(f'Confirmed {len(cases)} reference cases')
PY
```

The Rust test applies every fixture across combinations of missing and present
printer/non-printer classes and SPP UUID results, ensuring neither property can
bypass name filtering. Output tests cover filtered JSON/table output and `--all`.
These checks do not validate live Bluetooth discovery or printer compatibility.
