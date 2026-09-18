package com.rt.sdk;

/** Only the original JNI entry points used by the offline reference harness. */
public final class NativeApis {
    static { System.loadLibrary("rt_printer_sdk"); }

    public static native void setThreshold(int threshold);
    public static native byte[] rgb2PrintData(byte[] argb, int width, int height, int algorithm, int invert);
    public static native byte[] getImagePackageData(byte[] pixels, int width, int height, int reversed, int model, int copies, int next, int newAlgo);
    public static native byte[] getCommand(int superCode, int subCode);
    public static native byte[] getCommandWithInt(int superCode, int subCode, int value);
    public static native byte[] getCommandWithString(int superCode, int subCode, String value);
    public static native byte[] feedPaper(String value);
    public static native byte[] setPrinterInfoConnectDeviceData(int value);
    public static native void initProtocol();
    public static native void setODMPrinterType(int type);
    public static native byte[] getProtocolData();
    public static native byte[] getShakeData();
    public static native java.util.ArrayList<com.rt.sdk.responedata.DataPackage> decodePackages(byte[] bytes);
    public static native byte[] getCommandWithBytArray(int superCode, int subCode, byte[] data);
}
