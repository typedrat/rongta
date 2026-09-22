import java.util.Locale;
import java.util.regex.Pattern;

// Name-filter reference extracted from RONGTA 3.1.6 (normal app channel):
// SearchBluetoothDeviceActivity.filterDevice and qh.j0.d in analysis/jadx/sources.
// Android device-type checks are outside this name-only reference.
// Run: java tools/StockDeviceFilterReference.java NAME ...
class StockDeviceFilterReference {
    static final String[] MODELS = {
        "PN81", "F81", "F82", "F81A", "F81S", "F82A", "F11", "F12",
        "R5", "R22", "TM95", "TD210", "TH110", "RP425", "RP420", "RP421A",
        "RP415", "R26", "X21", "RP310D", "T11", "T12", "X11"
    };

    static boolean matches(String name, String model) {
        String upperName = name.toUpperCase(Locale.ROOT);
        String upperModel = model.toUpperCase(Locale.ROOT);
        // The original tests these prefixes BEFORE uppercasing.
        if (name.startsWith("R5") || name.startsWith("R22") || name.startsWith("R26")
            || name.startsWith("TM95") || name.startsWith("TD210") || name.startsWith("TH110")
            || name.startsWith("RP425") || name.startsWith("RP420") || name.startsWith("RP415")
            || name.startsWith("RP310D")) {
            return Pattern.compile("^" + upperModel + "[A-Z0-9]{0,3}([-_ ].*)?$")
                .matcher(upperName).matches();
        }
        return Pattern.compile("^" + upperModel + "([-_ ].*)?$").matcher(upperName).matches();
    }

    static boolean filter(String name) {
        if (name == null || name.isBlank()
            || name.regionMatches(true, name.length() - 3, "BLE", 0, 3)) {
            return false;
        }
        for (String model : MODELS) {
            if (matches(name, model)) return true;
        }
        return false;
    }

    public static void main(String[] args) {
        for (String name : args) System.out.println(filter(name));
    }
}
