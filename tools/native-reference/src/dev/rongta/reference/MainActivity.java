package dev.rongta.reference;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
import android.widget.TextView;
import com.rt.sdk.NativeApis;
import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import org.json.JSONArray;
import org.json.JSONObject;

/** Executes the stock library locally. No Bluetooth or network permissions. */
public final class MainActivity extends Activity {
    static String hex(byte[] bytes) {
        StringBuilder text = new StringBuilder(bytes.length * 2);
        for (byte value : bytes) {
            text.append(Character.forDigit((value & 0xff) >>> 4, 16));
            text.append(Character.forDigit(value & 15, 16));
        }
        return text.toString();
    }

    private static void command(JSONArray vectors, String name, int superCode,
                                int subCode, byte[] payload) throws Exception {
        byte[] wire = NativeApis.getCommandWithBytArray(superCode, subCode, payload);
        vectors.put(new JSONObject()
            .put("name", name)
            .put("method", "getCommandWithBytArray")
            .put("superclass", superCode)
            .put("subclass", subCode)
            .put("payload_hex", hex(payload))
            .put("wire_hex", hex(wire)));
    }

    @Override public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        TextView status = new TextView(this);
        setContentView(status);
        try {
            NativeApis.initProtocol();
            NativeApis.setODMPrinterType(28);
            JSONArray vectors = new JSONArray();
            vectors.put(new JSONObject()
                .put("name", "discovery")
                .put("method", "getProtocolData")
                .put("wire_hex", hex(NativeApis.getProtocolData())));
            command(vectors, "empty", 1, 9, new byte[0]);
            command(vectors, "one-byte", 1, 9, new byte[] {(byte) 0xa3});
            command(vectors, "embedded-magic", 0xf0, 0xfe,
                new byte[] {0, (byte) 0xa3, 0x1e, 0x1c, (byte) 0xff});
            for (int length : new int[] {255, 256, 257, 2045, 4096}) {
                byte[] payload = new byte[length];
                for (int i = 0; i < payload.length; i++) payload[i] = (byte) i;
                command(vectors, "sequence-" + length, 1, 9, payload);
            }
            JSONObject result = new JSONObject()
                .put("schema_version", 1)
                .put("odm_printer_type", 28)
                .put("vectors", vectors);
            if ("handshake".equals(getIntent().getStringExtra("suite"))) {
                result = HandshakeReference.generate(this);
            }
            if ("image".equals(getIntent().getStringExtra("suite"))) {
                result = ImageReference.generate(this);
            }
            // Publish atomically, so the host never reads a partial result.
            File pending = new File(getFilesDir(), "reference.pending");
            try (FileOutputStream out = new FileOutputStream(pending)) {
                out.write(result.toString(2).getBytes(StandardCharsets.UTF_8));
            }
            if (!pending.renameTo(new File(getFilesDir(), "reference.json"))) {
                throw new IllegalStateException("Cannot publish reference output");
            }
            status.setText("Generated SDK " + getIntent().getStringExtra("suite") + " reference output");
        } catch (Throwable error) {
            Log.e("RongtaReference", "Native reference generation failed", error);
            status.setText(error.toString());
            try (FileOutputStream out = new FileOutputStream(new File(getFilesDir(), "error.txt"))) {
                out.write(Log.getStackTraceString(error).getBytes(StandardCharsets.UTF_8));
            } catch (Exception writeError) {
                Log.e("RongtaReference", "Cannot write error report", writeError);
            }
        }
    }
}
