"""Inspect release PE imports and size without executing the binary or extra packages."""
import argparse
import hashlib
import json
import pathlib
import struct
import sys

SYSTEM_DLLS = {
    "kernel32.dll", "user32.dll", "gdi32.dll", "advapi32.dll", "shell32.dll",
    "ole32.dll", "comdlg32.dll", "comctl32.dll", "netapi32.dll", "rstrtmgr.dll",
    "ws2_32.dll", "bcrypt.dll", "userenv.dll", "ntdll.dll", "msvcrt.dll",
    "psapi.dll", "crypt32.dll", "secur32.dll", "version.dll", "rpcrt4.dll",
    "uxtheme.dll", "dwmapi.dll", "shlwapi.dll", "oleaut32.dll",
}
NEWER_APIS = {
    "CopyFile2", "IsProcessCritical", "GetUserDefaultGeoName", "SetThreadDescription",
    "GetSystemTimePreciseAsFileTime", "WaitOnAddress", "WakeByAddressSingle",
    "WakeByAddressAll", "GetDpiForWindow", "SetProcessDpiAwarenessContext",
    "GetOverlappedResultEx", "SetProcessInformation", "GetProcessInformation",
}

def audit(path, limit=5 * 1024 * 1024):
    data = pathlib.Path(path).read_bytes()
    def u16(n): return struct.unpack_from("<H", data, n)[0]
    def u32(n): return struct.unpack_from("<I", data, n)[0]
    if data[:2] != b"MZ": raise ValueError("Not a PE file")
    pe = u32(0x3C)
    if data[pe:pe + 4] != b"PE\0\0": raise ValueError("Invalid PE signature")
    optional = pe + 24
    magic = u16(optional)
    width = 8 if magic == 0x20B else 4 if magic == 0x10B else 0
    if not width: raise ValueError("Unsupported PE optional header")
    directories = optional + (112 if width == 8 else 96)
    sections = []
    start = optional + u16(pe + 20)
    for index in range(u16(pe + 6)):
        at = start + index * 40
        sections.append((u32(at + 12), max(u32(at + 8), u32(at + 16)), u32(at + 20)))
    def offset(rva):
        for va, size, raw in sections:
            if va <= rva < va + size:
                result = raw + rva - va
                if not 0 <= result < len(data): raise ValueError("RVA outside file")
                return result
        raise ValueError("Unmapped RVA")
    def string(rva):
        at = offset(rva)
        end = data.find(b"\0", at)
        if end < 0 or end - at > 4096: raise ValueError("Invalid PE string")
        return data[at:end].decode("ascii")
    imports = {}
    import_rva = u32(directories + 8)
    at = offset(import_rva) if import_rva else 0
    for _ in range(4096):
        if not at or data[at:at + 20] == b"\0" * 20: break
        lookup, _, _, name, first = struct.unpack_from("<IIIII", data, at)
        dll = string(name).lower()
        thunk = offset(lookup or first)
        names = []
        for index in range(65536):
            value = struct.unpack_from("<Q" if width == 8 else "<I", data, thunk + index * width)[0]
            if not value: break
            names.append("ordinal:" + str(value & 0xFFFF) if value & (1 << (width * 8 - 1)) else string(value + 2))
        imports[dll] = names
        at += 20
    bad_dlls = sorted(set(imports) - SYSTEM_DLLS)
    bad_apis = sorted({name for names in imports.values() for name in names} & NEWER_APIS)
    delayed = u32(directories + 13 * 8)
    issues = []
    if len(data) > limit: issues.append("EXE exceeds size limit")
    if bad_dlls: issues.append("Unapproved runtime DLL imports: " + ", ".join(bad_dlls))
    if bad_apis: issues.append("Static APIs newer than Windows 7: " + ", ".join(bad_apis))
    if delayed: issues.append("Delay imports require separate review")
    return {
        "file": pathlib.Path(path).name, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "limit_bytes": limit,
        "machine": hex(u16(pe + 4)), "os_version": [u16(optional + 40), u16(optional + 42)],
        "subsystem_version": [u16(optional + 48), u16(optional + 50)],
        "imports": imports, "issues": issues, "passed": not issues,
        "actual_windows7_runtime_tested": False,
    }

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("files", nargs="+")
    parser.add_argument("--out")
    parser.add_argument("--max-bytes", type=int, default=5 * 1024 * 1024)
    args = parser.parse_args()
    results = [audit(file, args.max_bytes) for file in args.files]
    encoded = json.dumps(results, indent=2, ensure_ascii=False)
    if args.out: pathlib.Path(args.out).write_text(encoded + "\n", encoding="utf-8")
    else: print(encoded)
    for result in results:
        print(result["file"], result["bytes"], "PASS" if result["passed"] else result["issues"], file=sys.stderr)
    raise SystemExit(0 if all(result["passed"] for result in results) else 1)
