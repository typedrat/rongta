package dev.rongta.reference;

import android.content.Context;
import com.rt.sdk.NativeApis;
import com.rt.sdk.responedata.DataPackage;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import org.json.JSONArray;
import org.json.JSONObject;

final class HandshakeReference {
    static JSONObject generate(Context context) throws Exception {
        ByteArrayOutputStream contents = new ByteArrayOutputStream();
        try (InputStream input = context.getAssets().open("handshake-input.json")) {
            byte[] buffer = new byte[4096];
            int length;
            while ((length = input.read(buffer)) != -1) contents.write(buffer, 0, length);
        }
        JSONArray scenarios = new JSONArray(new String(contents.toByteArray(), StandardCharsets.UTF_8));
        for (int i = 0; i < scenarios.length(); i++) {
            JSONObject scenario = scenarios.getJSONObject(i);
            NativeApis.initProtocol();
            NativeApis.setODMPrinterType(28);
            scenario.put("discovery_hex", MainActivity.hex(NativeApis.getProtocolData()));
            JSONArray responses = scenario.getJSONArray("responses");
            for (int j = 0; j < responses.length(); j++) {
                JSONObject response = responses.getJSONObject(j);
                String wire = response.getString("wire_hex");
                byte[] bytes = new byte[wire.length() / 2];
                for (int k = 0; k < bytes.length; k++) {
                    bytes[k] = (byte) Integer.parseInt(wire.substring(k * 2, k * 2 + 2), 16);
                }
                JSONArray events = new JSONArray();
                for (DataPackage packet : NativeApis.decodePackages(bytes)) {
                    events.put(new JSONObject().put("superclass", packet.funCode.superCode)
                        .put("subclass", packet.funCode.subCode).put("success", packet.isSucceed)
                        .put("json", packet.jsonData));
                }
                response.put("sdk_events", events);
                if (j == 1) scenario.put("handshake_hex", MainActivity.hex(NativeApis.getShakeData()));
            }
        }
        return new JSONObject().put("schema_version", 1).put("odm_printer_type", 28)
            .put("scenarios", scenarios);
    }
}
