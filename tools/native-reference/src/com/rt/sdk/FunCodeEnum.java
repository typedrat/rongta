package com.rt.sdk;

/** Minimal JNI-compatible metadata; enum values do not implement protocol logic. */
public enum FunCodeEnum {
    RT_CONNECT_PROTOCOL(0, 0), RT_CONNECT_HANDSHAKE(0, 1),
    PROTOCOL(1, 1), PROOF(1, 2), INFO(1, 3), MODEL(1, 4),
    SERIAL(1, 6), BATTERY(1, 14), CHALLENGE(1, 23), NULL(255, 255);
    public final int superCode;
    public final int subCode;
    FunCodeEnum(int superCode, int subCode) {
        this.superCode = superCode;
        this.subCode = subCode;
    }
}
