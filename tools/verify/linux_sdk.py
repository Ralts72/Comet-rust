"""Prepare the real ALSA SDK used by non-Linux Rust compile checks."""
import hashlib
import io
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[2]
TARGET = "x86_64-unknown-linux-gnu"
PACKAGES = (
    ("libasound2-dev_1.2.8-1+b1_amd64.deb", "6eddd5b43c03cdd769f6b6f4506abcbc84c19d4bada2b3036d1fd921a9875d7a"),
    ("libasound2_1.2.8-1+b1_amd64.deb", "44c77b076a7b11ae99712439022d822245b1994c435da564ebd320bb676faf4c"),
)
BASE_URL = "https://deb.debian.org/debian/pool/main/a/alsa-lib/"
MAX_DOWNLOAD = 4 * 1024 * 1024
MAX_EXTRACTED = 16 * 1024 * 1024


def extract_data(data, destination):
    """Read only a Debian package's data archive; never run its installation scripts."""
    if data[:8] != b"!<arch>\n":
        raise ValueError("invalid Debian archive")
    offset, archive = 8, None
    while offset < len(data):
        header = data[offset:offset + 60]
        if len(header) != 60 or header[58:60] != b"`\n":
            raise ValueError("invalid Debian member header")
        name = header[:16].decode("ascii").strip().rstrip("/")
        size = int(header[48:58].decode("ascii").strip())
        begin, end = offset + 60, offset + 60 + size
        if size < 0 or end > len(data):
            raise ValueError("truncated Debian member")
        if name.startswith("data.tar"):
            if archive is not None:
                raise ValueError("multiple Debian data archives")
            archive = data[begin:end]
        offset = end + size % 2
    if archive is None:
        raise ValueError("missing Debian data archive")
    destination = destination.resolve()
    total = 0
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:*") as source:
        for count, member in enumerate(source):
            if count >= 4096:
                raise ValueError("ALSA SDK contains too many files")
            relative = PurePosixPath(member.name)
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("SDK member escapes its destination")
            if not relative.parts:
                continue
            path = destination.joinpath(*relative.parts)
            if not path.resolve().is_relative_to(destination):
                raise ValueError("SDK member follows an outside link")
            if member.isdir():
                path.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                total += member.size
                if total > MAX_EXTRACTED:
                    raise ValueError("ALSA SDK exceeds its extraction budget")
                path.parent.mkdir(parents=True, exist_ok=True)
                contents = source.extractfile(member).read(member.size + 1)
                if len(contents) != member.size:
                    raise ValueError("truncated SDK file")
                if path.exists() and path.read_bytes() != contents:
                    raise ValueError("conflicting SDK package files")
                path.write_bytes(contents)
            elif member.issym():
                target = PurePosixPath(member.linkname)
                if target.is_absolute() or not (path.parent / member.linkname).resolve().is_relative_to(destination):
                    raise ValueError("SDK link escapes its destination")
                path.parent.mkdir(parents=True, exist_ok=True)
                if path.is_symlink() and os.readlink(path) == member.linkname:
                    continue
                path.symlink_to(member.linkname)
            else:
                raise ValueError("unsupported SDK archive member")


def inventory(root):
    files, links = {}, {}
    for path in sorted(root.rglob("*")):
        name = str(path.relative_to(root))
        if name == "sdk.json":
            continue
        if path.is_symlink():
            if not path.resolve(strict=True).is_relative_to(root):
                raise ValueError("SDK link leaves its root")
            links[name] = os.readlink(path)
        elif path.is_file():
            files[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    return {"files": files, "links": links}


def verify_layout(root):
    library = root / "usr/lib/x86_64-linux-gnu/libasound.so"
    header = library.read_bytes()[:20]
    if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[18:20], "little") != 62:
        raise ValueError("ALSA SDK library is not an x86_64 ELF")
    if not (root / "usr/include/alsa/asoundlib.h").is_file():
        raise ValueError("ALSA SDK headers are missing")
    if not (root / "usr/lib/x86_64-linux-gnu/pkgconfig/alsa.pc").is_file():
        raise ValueError("ALSA SDK pkg-config metadata is missing")


def valid_cache(root):
    try:
        receipt = json.loads((root / "sdk.json").read_text())
        if receipt["packages"] != [list(package) for package in PACKAGES]:
            return False
        verify_layout(root)
        return receipt["inventory"] == inventory(root)
    except (OSError, ValueError, KeyError, RuntimeError):
        return False


def prepare_sdk():
    parent = ROOT / "target/sysroots"
    parent.mkdir(parents=True, exist_ok=True)
    identity = hashlib.sha256(json.dumps(PACKAGES).encode()).hexdigest()[:16]
    path = parent / f"alsa-amd64-{identity}"
    suffix = 0
    while path.exists():
        if valid_cache(path):
            return path
        suffix += 1
        path = parent / f"alsa-amd64-{identity}-{suffix}"
    candidate = Path(tempfile.mkdtemp(prefix=".alsa-", dir=parent))
    try:
        for name, digest in PACKAGES:
            request = urllib.request.Request(BASE_URL + name, headers={"User-Agent": "Comet-verification"})
            with urllib.request.urlopen(request, timeout=60) as response:
                contents = response.read(MAX_DOWNLOAD + 1)
            if len(contents) > MAX_DOWNLOAD or hashlib.sha256(contents).hexdigest() != digest:
                raise ValueError(f"ALSA SDK package size or SHA256 mismatch: {name}")
            extract_data(contents, candidate)
        verify_layout(candidate)
        receipt = {"packages": PACKAGES, "inventory": inventory(candidate)}
        (candidate / "sdk.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if not valid_cache(candidate):
            raise ValueError("ALSA SDK candidate failed validation")
        try:
            candidate.rename(path)
        except OSError:
            if not valid_cache(path):
                raise
        return path
    finally:
        if candidate.exists():
            shutil.rmtree(candidate)


def environment(target):
    host = next(line.split(": ", 1)[1] for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines()
                if line.startswith("host: "))
    if target != TARGET or host == target:
        return os.environ.copy()
    pkg_config = shutil.which("pkg-config")
    if not pkg_config:
        raise RuntimeError("Linux 音频交叉检查需要 pkg-config；macOS 可用 brew install pkgconf 安装。")
    sdk = prepare_sdk()
    result = os.environ.copy()
    result["PKG_CONFIG_ALLOW_CROSS"] = "1"
    for suffix in (target, target.replace("-", "_")):
        result.update({f"PKG_CONFIG_{suffix}": pkg_config,
                       f"PKG_CONFIG_SYSROOT_DIR_{suffix}": str(sdk),
                       f"PKG_CONFIG_LIBDIR_{suffix}": str(sdk / "usr/lib/x86_64-linux-gnu/pkgconfig"),
                       f"PKG_CONFIG_PATH_{suffix}": ""})
    return result
