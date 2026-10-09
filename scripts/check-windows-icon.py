#!/usr/bin/env python3
"""Check the built Windows icon resources against the shared application icon.

Runs on the Windows packaging runner without executing the application.
"""

import ctypes
from ctypes import wintypes
from pathlib import Path
import struct
import sys


def main():
    binary = Path(sys.argv[1]).resolve()
    icon = (Path(__file__).resolve().parent.parent / "assets/icons/mtty.ico").read_bytes()
    reserved, kind, count = struct.unpack_from("<HHH", icon)
    assert (reserved, kind) == (0, 1)
    expected = []
    for index in range(count):
        entry = struct.unpack_from("<BBBBHHII", icon, 6 + index * 16)
        width, height, colors, reserved, planes, depth, size, offset = entry
        expected.append(((width, height, colors, reserved, planes, depth, size), icon[offset:offset + size]))
    sizes = {(entry[0] or 256) for entry, data in expected}
    missing = {16, 24, 32, 48, 64, 128, 256} - sizes
    assert not missing, f"shared icon is missing required Windows sizes: {sorted(missing)}"

    if binary.read_bytes()[:6] == icon[:6]:
        assert binary.read_bytes() == icon, "registered icon differs from shared artwork"
        print(f"Verified {count} registered icon sizes in {binary.name}")
        return

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, ctypes.c_void_p, wintypes.DWORD]
    kernel.LoadLibraryExW.restype = ctypes.c_void_p
    kernel.FindResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = ctypes.c_void_p
    kernel.SizeofResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.SizeofResource.restype = wintypes.DWORD
    kernel.LoadResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.LoadResource.restype = ctypes.c_void_p
    kernel.LockResource.argtypes = [ctypes.c_void_p]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.FreeLibrary.argtypes = [ctypes.c_void_p]
    kernel.FreeLibrary.restype = wintypes.BOOL
    module = kernel.LoadLibraryExW(str(binary), None, 2)  # LOAD_LIBRARY_AS_DATAFILE
    if not module:
        raise ctypes.WinError(ctypes.get_last_error())

    def resource(ordinal, resource_type):
        handle = kernel.FindResourceW(module, ordinal, resource_type)
        if not handle:
            raise ctypes.WinError(ctypes.get_last_error())
        size = kernel.SizeofResource(module, handle)
        loaded = kernel.LoadResource(module, handle)
        pointer = kernel.LockResource(loaded)
        if not pointer or not size:
            raise ctypes.WinError(ctypes.get_last_error())
        return ctypes.string_at(pointer, size)

    try:
        user = ctypes.WinDLL("user32", use_last_error=True)
        user.LoadImageW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, wintypes.UINT, ctypes.c_int, ctypes.c_int, wintypes.UINT]
        user.LoadImageW.restype = ctypes.c_void_p
        user.DestroyIcon.argtypes = [ctypes.c_void_p]
        user.DestroyIcon.restype = wintypes.BOOL
        for header, _image in expected:
            size = header[0] or 256
            handle = user.LoadImageW(module, 1, 1, size, size, 0)  # IMAGE_ICON
            if not handle:
                raise ctypes.WinError(ctypes.get_last_error())
            assert user.DestroyIcon(handle)
        group = resource(1, 14)  # RT_GROUP_ICON
        assert struct.unpack_from("<HHH", group) == (0, 1, count)
        for index, (header, image) in enumerate(expected):
            entry = struct.unpack_from("<BBBBHHIH", group, 6 + index * 14)
            assert entry[:-1] == header
            assert resource(entry[-1], 3) == image  # RT_ICON
    finally:
        kernel.FreeLibrary(module)
    print(f"Verified {count} embedded icon sizes in {binary.name}")


if __name__ == "__main__":
    main()
