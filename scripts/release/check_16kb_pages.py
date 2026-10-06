#!/usr/bin/env python3
"""Fail when an ELF shared object's PT_LOAD segments are not 16 KB aligned.

Android can run devices with 16 KB pages; a library whose load segments are only
4 KB aligned may fail to map there. The release gate is the alignment of the
load segments themselves — a successful build and a successful `adb install` are
not evidence of it.

Usage: check_16kb_pages.py <library.so> [...]
"""
import struct
import sys

REQUIRED_ALIGNMENT = 16384
PT_LOAD = 1


def load_alignments(path):
    with open(path, "rb") as handle:
        data = handle.read()
    if data[:4] != b"\x7fELF":
        raise SystemExit(f"not an ELF file: {path}")
    if data[5] != 1:
        raise SystemExit(f"not little-endian: {path}")
    is64 = data[4] == 2
    if is64:
        phoff, = struct.unpack_from("<Q", data, 0x20)
        phentsize, phnum = struct.unpack_from("<HH", data, 0x36)
        align_offset = 0x30
    else:
        phoff, = struct.unpack_from("<I", data, 0x1C)
        phentsize, phnum = struct.unpack_from("<HH", data, 0x2A)
        align_offset = 0x1C
    alignments = []
    for index in range(phnum):
        base = phoff + index * phentsize
        p_type, = struct.unpack_from("<I", data, base)
        if p_type != PT_LOAD:
            continue
        if is64:
            p_align, = struct.unpack_from("<Q", data, base + align_offset)
        else:
            p_align, = struct.unpack_from("<I", data, base + align_offset)
        alignments.append(p_align)
    return alignments


def main(paths):
    failed = False
    for path in paths:
        alignments = load_alignments(path)
        if not alignments:
            print(f"FAIL {path}: no PT_LOAD segment")
            failed = True
            continue
        smallest = min(alignments)
        if smallest < REQUIRED_ALIGNMENT:
            print(f"FAIL {path}: smallest PT_LOAD alignment {smallest} < {REQUIRED_ALIGNMENT}")
            failed = True
        else:
            print(f"ok   {path}: PT_LOAD alignment {smallest}")
    return 1 if failed else 0


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    raise SystemExit(main(sys.argv[1:]))
