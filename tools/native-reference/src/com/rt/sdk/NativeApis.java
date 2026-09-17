package com.rt.sdk;

/** Only the original JNI entry points used by the offline reference harness. */
public final class NativeApis {
    static { System.loadLibrary("rt_printer_sdk"); }

    public static native void initProtocol();
    public static native void setODMPrinterType(int type);
    public static native byte[] getProtocolData();
    public static native byte[] getCommandWithBytArray(int superCode, int subCode, byte[] data);
}
