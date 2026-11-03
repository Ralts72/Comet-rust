"""Embed and verify ICO resources on private Windows executable candidates."""
import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import struct

RT_ICON = 3
RT_GROUP_ICON = 14
ICON_GROUP_ID = 1


def icon_resources(data):
    if len(data) < 6:
        raise ValueError("ICO 图标不完整，请执行 git lfs pull。")
    reserved, kind, count = struct.unpack_from("<HHH", data)
    directory_size = 6 + count * 16
    if reserved != 0 or kind != 1 or count == 0 or directory_size > len(data):
        raise ValueError("ICO 图标目录无效，请执行 git lfs pull。")
    group = bytearray(data[:6])
    resources = []
    for index in range(count):
        offset = 6 + index * 16
        width, height, colors, reserved, planes, bits, size, position = struct.unpack_from("<BBBBHHII", data, offset)
        if reserved != 0 or size == 0 or position < directory_size or position + size > len(data):
            raise ValueError("ICO 图标图像超出文件边界。")
        identifier = index + 1
        resources.append((RT_ICON, identifier, data[position:position + size]))
        # PE groups refer to individual RT_ICON resources by WORD ID, not file offsets.
        group.extend(struct.pack("<BBBBHHIH", width, height, colors, 0, planes, bits, size, identifier))
    resources.append((RT_GROUP_ICON, ICON_GROUP_ID, bytes(group)))
    return resources


def _api():
    if os.name != "nt":
        raise RuntimeError("Windows EXE 图标写入和读取需要 Windows 原生环境。")
    api = ctypes.WinDLL("kernel32", use_last_error=True)
    functions = {
        "BeginUpdateResourceW": ([wintypes.LPCWSTR, wintypes.BOOL], wintypes.HANDLE),
        "UpdateResourceW": ([wintypes.HANDLE, ctypes.c_void_p, ctypes.c_void_p, wintypes.WORD, ctypes.c_void_p, wintypes.DWORD], wintypes.BOOL),
        "EndUpdateResourceW": ([wintypes.HANDLE, wintypes.BOOL], wintypes.BOOL),
        "LoadLibraryExW": ([wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD], wintypes.HMODULE),
        "FindResourceExW": ([wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p, wintypes.WORD], wintypes.HRSRC),
        "EnumResourceLanguagesW": ([wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, wintypes.LPARAM], wintypes.BOOL),
        "SizeofResource": ([wintypes.HMODULE, wintypes.HRSRC], wintypes.DWORD),
        "LoadResource": ([wintypes.HMODULE, wintypes.HRSRC], wintypes.HGLOBAL),
        "LockResource": ([wintypes.HGLOBAL], ctypes.c_void_p),
        "FreeLibrary": ([wintypes.HMODULE], wintypes.BOOL),
    }
    for name, (arguments, result) in functions.items():
        function = getattr(api, name)
        function.argtypes = arguments
        function.restype = result
    return api


def _group_languages(api, module):
    languages = {0}
    factory = getattr(ctypes, "WINFUNCTYPE", ctypes.CFUNCTYPE)
    callback_type = factory(wintypes.BOOL, wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p, wintypes.WORD, wintypes.LPARAM)
    @callback_type
    def collect(_module, _kind, _name, language, _context):
        languages.add(language)
        return True
    if not api.EnumResourceLanguagesW(module, RT_GROUP_ICON, ICON_GROUP_ID, ctypes.cast(collect, ctypes.c_void_p), 0):
        error = ctypes.get_last_error()
        # Fresh Cargo programs may have no resource section or icon yet.
        if error not in (1812, 1813, 1814, 1815):
            raise ctypes.WinError(error)
    return sorted(languages)


def _load(api, binary):
    # DATAFILE | IMAGE_RESOURCE maps the PE for resource reads without executing its code.
    module = api.LoadLibraryExW(str(Path(binary).resolve()), None, 0x2 | 0x20)
    if not module:
        raise ctypes.WinError(ctypes.get_last_error())
    return module


def _release(api, module):
    if not api.FreeLibrary(module):
        raise ctypes.WinError(ctypes.get_last_error())


def apply_icon(binary, icon):
    resources = icon_resources(icon)
    # Retain every payload buffer until EndUpdateResource commits the pending updates.
    buffers = [ctypes.create_string_buffer(data) for _, _, data in resources]
    api = _api()
    module = _load(api, binary)
    try:
        languages = _group_languages(api, module)
    finally:
        _release(api, module)
    # Preserve the program's manifest and other resources. The Cargo artifact is never modified.
    handle = api.BeginUpdateResourceW(str(Path(binary).resolve()), False)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        # Update existing language variants too so localized icons cannot shadow the new group.
        for language in languages:
            for (kind, identifier, data), buffer in zip(resources, buffers):
                if not api.UpdateResourceW(handle, kind, identifier, language, buffer, len(data)):
                    raise ctypes.WinError(ctypes.get_last_error())
    except BaseException:
        api.EndUpdateResourceW(handle, True)
        raise
    if not api.EndUpdateResourceW(handle, False):
        raise ctypes.WinError(ctypes.get_last_error())
    verify_icon(binary, icon)


def verify_icon(binary, icon):
    resources = icon_resources(icon)
    api = _api()
    module = _load(api, binary)
    try:
        for language in _group_languages(api, module):
            for kind, identifier, expected in resources:
                resource = api.FindResourceExW(module, kind, identifier, language)
                if not resource:
                    raise ctypes.WinError(ctypes.get_last_error())
                size = api.SizeofResource(module, resource)
                loaded = api.LoadResource(module, resource)
                pointer = api.LockResource(loaded) if loaded else None
                if not pointer:
                    raise ctypes.WinError(ctypes.get_last_error())
                if size != len(expected) or ctypes.string_at(pointer, size) != expected:
                    raise ValueError(f"Windows 图标资源 {kind}/{identifier}/{language} 与原始 ICO 不匹配。")
    finally:
        _release(api, module)
