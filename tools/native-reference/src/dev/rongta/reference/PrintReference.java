package dev.rongta.reference;

import android.content.Context;
import com.rt.sdk.NativeApis;
import com.rt.sdk.responedata.DataPackage;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import org.json.JSONArray;
import org.json.JSONObject;

final class PrintReference {
    private static void add(JSONArray cases, String name, byte[] bytes) throws Exception {
        cases.put(new JSONObject().put("name",name).put("wire_hex",MainActivity.hex(bytes)));
    }
    static JSONObject generate(Context context) throws Exception {
        NativeApis.initProtocol(); NativeApis.setODMPrinterType(28);
        JSONArray commands = new JSONArray();
        add(commands,"status",NativeApis.getCommand(1,19));
        add(commands,"connected",NativeApis.setPrinterInfoConnectDeviceData(1));
        add(commands,"mode-0",NativeApis.getCommandWithInt(5,41,0));
        for (int density : new int[]{0,8,15}) add(commands,"density-"+density,NativeApis.getCommandWithString(5,4,"8,"+density));
        for (int speed : new int[]{8,16}) add(commands,"speed-"+speed,NativeApis.getCommandWithInt(5,7,speed));
        for (int feed : new int[]{1,256}) add(commands,"feed-"+feed,NativeApis.feedPaper("1,"+feed));
        ByteArrayOutputStream contents = new ByteArrayOutputStream();
        try (InputStream input=context.getAssets().open("print-input.json")) {
            byte[] buffer=new byte[4096]; int length;
            while ((length=input.read(buffer)) != -1) contents.write(buffer,0,length);
        }
        JSONArray responses=new JSONArray(new String(contents.toByteArray(),StandardCharsets.UTF_8));
        for (int i=0; i<responses.length(); i++) {
            JSONObject response=responses.getJSONObject(i);
            String wire=response.getString("wire_hex"); byte[] bytes=new byte[wire.length()/2];
            for (int j=0; j<bytes.length; j++) bytes[j]=(byte)Integer.parseInt(wire.substring(j*2,j*2+2),16);
            JSONArray events=new JSONArray();
            for (DataPackage packet:NativeApis.decodePackages(bytes)) events.put(new JSONObject()
                .put("superclass",packet.funCode.superCode).put("subclass",packet.funCode.subCode)
                .put("success",packet.isSucceed).put("json",packet.jsonData));
            response.put("sdk_events",events);
        }
        return new JSONObject().put("schema_version",1).put("odm_printer_type",28)
            .put("commands",commands).put("responses",responses);
    }
}
