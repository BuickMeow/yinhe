#!/usr/bin/env python3
"""生成简单 SMF：单轨单音符长音（无 CC）。用法: gen_simple.py <out.mid> <key> <start_ticks> <dur_ticks>"""
import struct, sys

def vlq(n):
    out = []
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            break
    return bytes(out)

def main():
    out, key, start, dur = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
    ppq = 480
    # track events: on @start, off @start+dur
    ev = b'\x00\x90' + bytes([key, 100])          # note on at tick start
    ev += vlq(dur) + b'\x80' + bytes([key, 0])    # note off
    ev += b'\x00\xff\x2f\x00'                      # end of track
    track = b'MTrk' + struct.pack('>I', len(ev)) + ev
    header = b'MThd' + struct.pack('>IHHH', 6, 0, 1, ppq)
    with open(out, 'wb') as f:
        f.write(header + track)
    print(f"wrote {out}: key={key} start={start} dur={dur} ppq={ppq}")

main()
