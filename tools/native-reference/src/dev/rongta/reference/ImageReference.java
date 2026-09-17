package dev.rongta.reference;

import android.content.Context;
import com.rt.sdk.NativeApis;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import org.json.JSONArray;
import org.json.JSONObject;

final class ImageReference {
    private static byte[] bytes(String hex) {
        byte[] out = new byte[hex.length()/2];
        for (int i=0; i<out.length; i++) out[i] = (byte)Integer.parseInt(hex.substring(i*2, i*2+2),16);
        return out;
    }
    private static void configure(JSONObject vector) throws Exception {
        NativeApis.initProtocol();
        NativeApis.setODMPrinterType(28);
        NativeApis.getProtocolData();
        NativeApis.decodePackages(bytes(vector.getString("protocol_reply_hex")));
        NativeApis.decodePackages(bytes(vector.getString("serial_reply_hex")));
    }
    static JSONObject generate(Context context) throws Exception {
        ByteArrayOutputStream contents = new ByteArrayOutputStream();
        try (InputStream input = context.getAssets().open("image-input.json")) {
            byte[] buffer = new byte[4096];
            int length;
            while ((length=input.read(buffer)) != -1) contents.write(buffer,0,length);
        }
        JSONObject result = new JSONObject(new String(contents.toByteArray(),StandardCharsets.UTF_8));
        JSONArray rasters = result.getJSONArray("raster_vectors");
        for (int i=0; i<rasters.length(); i++) {
            JSONObject v=rasters.getJSONObject(i);
            configure(v);
            NativeApis.setThreshold(v.getInt("threshold"));
            byte[] argb=bytes(v.getString("argb_hex"));
            int width=v.getInt("width"), height=v.getInt("height");
            v.put("raster_hex",MainActivity.hex(NativeApis.rgb2PrintData(argb,width,height,0,0)));
            v.put("wire_hex",MainActivity.hex(NativeApis.getImagePackageData(argb,width,height,0,0,0,0,0)));
        }
        JSONArray images = result.getJSONArray("image_vectors");
        for (int i=0; i<images.length(); i++) {
            JSONObject v=images.getJSONObject(i);
            configure(v);
            v.put("wire_hex",MainActivity.hex(NativeApis.getImagePackageData(
                bytes(v.getString("raster_hex")),v.getInt("width"),v.getInt("height"),0,255,0,0,0)));
        }
        return result.put("schema_version",1).put("odm_printer_type",28);
    }
}
