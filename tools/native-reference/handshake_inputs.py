"""Simulated replies from static analysis, validated by the SDK, not hardware."""
import hashlib
import json
import struct
import zlib


def tlv(tag, value):
    return bytes([tag]) + struct.pack('<H', len(value)) + value


def number(tag, value):
    return tlv(tag, struct.pack('<H', value))


def reply(subclass, payload=b'', message=0x21):
    inner = bytes([message, 1, subclass]) + struct.pack('<H', len(payload)) + payload
    return (b'\xa3\x1e\x1c\x00' + struct.pack('<H', len(inner)) + inner
            + struct.pack('<I', zlib.crc32(inner, 0x76953521))).hex()


def scenarios():
    cases = []
    for name, serial, invalid in [
        ('success', 'R22-TEST-0001', False),
        ('different-serial', 'R22-TEST-0002', False),
        ('wrong-digest', 'R22-TEST-0001', True),
    ]:
        proof = hashlib.md5(serial.encode() + b'D932A43C3084B1D3').digest()[4:12].hex()
        # Intentionally independent of the SDK-generated request nonce. The SDK
        # authenticates the returned challenge; it does not require an echo.
        challenge = b'printer-nonce-01'
        digest = hashlib.md5(challenge + proof.encode()).hexdigest().encode()
        if invalid:
            digest = b'0' * 32
        info = {'DevName': 'R22 fixture', 'Firmware': 'test-1.0', 'HWVer': 'test-A', 'SN': serial}
        responses = [
            ('protocol', reply(1, number(1, 1) + number(2, 0) + number(3, 4096))),
            ('serial', reply(6, serial.encode())),
            ('proof-ack', reply(2)),
            ('info', reply(3, json.dumps(info, separators=(',', ':')).encode())),
            ('model', reply(4, number(1, 22) + number(2, 3) + number(3, 203) + number(4, 384))),
            ('battery', reply(14, number(1, 385) + number(2, 875) + number(3, 1))),
            ('challenge', reply(23, tlv(1, challenge) + tlv(2, digest)
                                + tlv(3, serial.encode()) + tlv(4, b'test-auth'))),
        ]
        cases.append({'name': name, 'serial': serial, 'expected_proof_ascii': proof,
                      'responses': [{'name': label, 'wire_hex': wire} for label, wire in responses]})
    return cases
