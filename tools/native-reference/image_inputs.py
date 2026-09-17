"""Deterministic image inputs. Expected raster and compressed bytes come from JNI."""
from handshake_inputs import reply, number


def inputs():
    rasters = []
    def argb(name, width, colors, threshold=208):
        rasters.append(dict(name=name, width=width, height=len(colors)//width,
                            threshold=threshold, argb_hex=bytes(sum((list(c) for c in colors), [])).hex()))
    ramp = [(255, x, x, x) for x in range(256)]
    for threshold in (0, 127, 208, 255):
        argb(f'gray-ramp-{threshold}', 16, ramp, threshold)
    argb('colors-alpha', 8, [(a, r, g, b) for a in (0, 128, 255)
         for r, g, b in [(0,0,0),(255,255,255),(255,0,0),(0,255,0),(0,0,255),(128,128,128),(208,208,208),(209,209,209)]])
    argb('color-pattern', 32, [(i*17%256, i*73%256, i*151%256, i*199%256) for i in range(512)], 127)
    argb('asymmetric-bits', 16, [(255,)*(4) if i%17 != 0 else (255,0,0,0) for i in range(128)])
    images = []
    def packed(name, width, data, buffer_length=4096):
        images.append(dict(name=name, width=width, height=len(data)*8//width,
                           buffer_length=buffer_length, raster_hex=data.hex()))
    packed('white', 8, bytes(8))
    packed('black', 8, b'\xff'*8)
    packed('one-byte', 8, b'\x80')
    images[-1]['unsupported_reason'] = 'Native emits a malformed empty packet before the one-byte raster'
    packed('packet-padding', 8, bytes([0, 0, 0, 1, 1, 2])*50, 64)
    packed('all-symbols', 128, bytes(range(256)))
    packed('equal-weight-ties', 16, bytes([0x80,0x40,0x20,0x10]*8))
    packed('skewed', 64, bytes([0]*7+[1]*3+[2]*2+[3,4,5,6])*16)
    packed('wide-multipacket', 384, bytes(range(256))*24)
    packed('small-buffer-sections', 64, bytes(range(256))*2, 256)
    packed('2048-buffer-sections', 128, bytes(range(256))*2, 2048)
    packed('solid-wide', 384, bytes(48*80))
    packed('non-byte-code-padding', 16, bytes([0,0,0,1,1,2,3,4])*16)
    for case in rasters + images:
        limit = case.setdefault('buffer_length', 4096)
        case['protocol_reply_hex'] = reply(1, number(1, 1)+number(2, 0)+number(3, limit))
        case['serial_reply_hex'] = reply(6, b'R22-TEST-0001')
    return dict(raster_vectors=rasters, image_vectors=images)
