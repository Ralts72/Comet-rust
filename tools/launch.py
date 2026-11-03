#!/usr/bin/env python3
"""Prepare the matched Editor/Release Player pair used by start.sh."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import stat
import subprocess
import sys
import tempfile
import windows_icon

ROOT = Path(__file__).resolve().parents[1]
DEMO = ROOT / "demo"
PRODUCTS = ROOT / "target/products/demo"
PATH_OPTIONS = {"--project", "--products", "--capture", "--report", "--export-game"}


def application_arguments(arguments, parser):
    result, supplied = [], set()
    arguments = iter(arguments)
    for argument in arguments:
        if argument == "--player-template":
            parser.error("Player 模板由 start.sh 准备，不接受覆盖")
        result.append(argument)
        if argument in PATH_OPTIONS or argument in ("--scene", "--include-scene"):
            value = next(arguments, None)
            if value is None:
                parser.error(f"{argument} 缺少参数")
            supplied.add(argument)
            result.append(str(Path(value).absolute()) if argument in PATH_OPTIONS else value)
        elif not argument.startswith("--"):
            parser.error(f"不接受动作或位置参数：{argument}")
    return result, supplied


def cargo_path():
    executable = "cargo.exe" if sys.platform == "win32" else "cargo"
    for directory in [None, Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")) / "bin", Path("/opt/homebrew/opt/rustup/bin"), Path("/usr/local/opt/rustup/bin")]:
        cargo = shutil.which("cargo") if directory is None else directory / executable
        if cargo and Path(cargo).is_file() and os.access(cargo, os.X_OK):
            cargo = Path(cargo).absolute()
            os.environ["PATH"] = str(cargo.parent) + os.pathsep + os.environ.get("PATH", "")
            return str(cargo)
    raise RuntimeError("找不到 cargo，请先安装 rustup，并将其命令目录加入 PATH。")


def file_digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def source_fingerprint():
    files = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "tools/launch.py", "tools/windows_icon.py")]
    files.extend((ROOT / ".cargo").glob("*.toml"))
    for directory in ("engine", "editor", "app", "tools/asset"):
        files.extend(p for p in (ROOT / directory).rglob("*") if p.is_file()
                     and (p.suffix in (".rs", ".wgsl") or p.name == "Cargo.toml" or "resources" in p.relative_to(ROOT).parts)
                     and "tests" not in p.relative_to(ROOT).parts)
    value = hashlib.sha256()
    for path in sorted(set(files)):
        value.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
        value.update(name.encode() + b"=" + os.environ.get(name, "").encode() + b"\0")
    return value.hexdigest()


def build_program(cargo, package, release, environment):
    command = [cargo, "build", "--locked", "-p", package, "--message-format=json-render-diagnostics"]
    if release:
        command.append("--release")
    executable = None
    with subprocess.Popen(command, cwd=ROOT, env=environment, stdout=subprocess.PIPE, text=True) as process:
        for line in process.stdout:
            try:
                message = json.loads(line)
            except ValueError:
                print(line, end="", flush=True)
                continue
            if message.get("reason") == "compiler-artifact" and message.get("executable") and message["target"]["name"] == package:
                executable = Path(message["executable"])
            elif message.get("reason") == "compiler-message":
                print(message["message"].get("rendered", ""), end="", file=sys.stderr, flush=True)
        if process.wait():
            raise subprocess.CalledProcessError(process.returncode, command)
    if executable is None:
        raise RuntimeError(f"Cargo 没有返回 {package} 的可执行文件")
    return executable


def icns_digest(path):
    data = path.read_bytes()
    if data[:4] != b"icns" or len(data) < 8 or int.from_bytes(data[4:8], "big") != len(data):
        raise RuntimeError(f"应用图标无效，请先执行 git lfs pull：{path}")
    return hashlib.sha256(data).hexdigest()


def editor_app_valid(snapshot, inputs, plist):
    bundle = snapshot / "Comet Editor.app"
    try:
        receipt = json.loads((snapshot / "bundle.json").read_text())
        binary = bundle / "Contents/MacOS/comet-editor"
        icon = bundle / "Contents/Resources/editor.icns"
        info = bundle / "Contents/Info.plist"
        if (receipt["inputs"] != inputs or file_digest(binary) != receipt["executable_sha256"]
                or file_digest(icon) != inputs["icon_sha256"] or info.read_bytes() != plist
                or any(not path.resolve().is_relative_to(snapshot.resolve()) for path in (binary, icon, info))):
            return False
        subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(bundle)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return True
    except (OSError, ValueError, TypeError, KeyError, subprocess.CalledProcessError):
        return False


def prepare_editor_app(editor):
    if sys.platform != "darwin":
        return editor
    icon = ROOT / "editor/resources/icons/editor.icns"
    plist = plistlib.dumps({"CFBundleExecutable": "comet-editor", "CFBundleName": "Comet Editor",
                           "CFBundleDisplayName": "Comet 编辑器", "CFBundleIdentifier": "org.comet.editor",
                           "CFBundleIconFile": "editor.icns", "CFBundlePackageType": "APPL",
                           "CFBundleVersion": "1", "NSHighResolutionCapable": True})
    inputs = {"editor_sha256": file_digest(editor), "icon_sha256": icns_digest(icon),
              "plist_sha256": hashlib.sha256(plist).hexdigest()}
    identity = hashlib.sha256(json.dumps(inputs, sort_keys=True).encode()).hexdigest()
    snapshots = ROOT / "target/editor-apps"
    snapshots.mkdir(parents=True, exist_ok=True)
    snapshot, suffix = snapshots / identity, 0
    while snapshot.exists():
        if editor_app_valid(snapshot, inputs, plist):
            return snapshot / "Comet Editor.app/Contents/MacOS/comet-editor"
        suffix += 1
        snapshot = snapshots / f"{identity}-{suffix}"
    with tempfile.TemporaryDirectory(prefix=".comet-", suffix=".tmp", dir=snapshots) as directory:
        candidate = Path(directory) / "snapshot"
        bundle = candidate / "Comet Editor.app"
        binary = bundle / "Contents/MacOS/comet-editor"
        resources = bundle / "Contents/Resources"
        binary.parent.mkdir(parents=True)
        resources.mkdir()
        shutil.copy2(editor, binary)
        shutil.copyfile(icon, resources / "editor.icns")
        (bundle / "Contents/Info.plist").write_bytes(plist)
        if file_digest(binary) != inputs["editor_sha256"] or file_digest(resources / "editor.icns") != inputs["icon_sha256"]:
            raise RuntimeError("复制期间编辑器或图标发生变化，请重新运行 ./start.sh。")
        subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", str(bundle)], check=True)
        receipt = {"inputs": inputs, "executable_sha256": file_digest(binary)}
        (candidate / "bundle.json").write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n")
        if not editor_app_valid(candidate, inputs, plist):
            raise RuntimeError("编辑器 App 校验失败，请重新运行 ./start.sh。")
        try:
            candidate.rename(snapshot)
        except OSError:
            if not editor_app_valid(snapshot, inputs, plist):
                raise
    return snapshot / "Comet Editor.app/Contents/MacOS/comet-editor"


def windows_program_valid(snapshot, inputs, icon):
    try:
        receipt = json.loads((snapshot / "program.json").read_text())
        binary = snapshot / inputs["name"]
        if (receipt["inputs"] != inputs or file_digest(binary) != receipt["executable_sha256"]
                or not binary.resolve().is_relative_to(snapshot.resolve())):
            return False
        windows_icon.verify_icon(binary, icon)
        return True
    except (OSError, ValueError, TypeError, KeyError):
        return False


def prepare_windows_program(executable, icon):
    data = icon.read_bytes()
    windows_icon.icon_resources(data)
    inputs = {"name": executable.name, "program_sha256": file_digest(executable),
              "icon_sha256": hashlib.sha256(data).hexdigest()}
    identity = hashlib.sha256(json.dumps(inputs, sort_keys=True).encode()).hexdigest()
    snapshots = ROOT / "target/windows-programs"
    snapshots.mkdir(parents=True, exist_ok=True)
    snapshot, suffix = snapshots / identity, 0
    while snapshot.exists():
        if windows_program_valid(snapshot, inputs, data):
            return snapshot / executable.name
        suffix += 1
        snapshot = snapshots / f"{identity}-{suffix}"
    with tempfile.TemporaryDirectory(prefix=".comet-", suffix=".tmp", dir=snapshots) as directory:
        candidate = Path(directory) / "snapshot"
        candidate.mkdir()
        binary = candidate / executable.name
        shutil.copy2(executable, binary)
        binary.chmod(binary.stat().st_mode | stat.S_IWRITE)
        if file_digest(binary) != inputs["program_sha256"]:
            raise RuntimeError("复制期间程序产物发生变化，请重新准备程序。")
        windows_icon.apply_icon(binary, data)
        receipt = {"inputs": inputs, "executable_sha256": file_digest(binary)}
        (candidate / "program.json").write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n")
        if not windows_program_valid(candidate, inputs, data):
            raise RuntimeError("Windows 程序图标校验失败，请重新准备程序。")
        try:
            candidate.rename(snapshot)
        except OSError:
            if not windows_program_valid(snapshot, inputs, data):
                raise
    return snapshot / executable.name


def prepare_programs():
    cargo = cargo_path()
    rustup = shutil.which("rustup")
    if not rustup:
        raise RuntimeError("启动需要 rustup 管理仓库锁定工具链。")
    subprocess.run([rustup, "show", "active-toolchain"], cwd=ROOT, check=True)
    build_id = source_fingerprint()
    environment = dict(os.environ, COMET_BUILD_ID=build_id)
    print("Comet: 准备 Debug Editor…", flush=True)
    editor = build_program(cargo, "comet-editor", False, environment)
    print("Comet: 准备 Release Player 模板…", flush=True)
    player = build_program(cargo, "comet-player", True, environment)
    if source_fingerprint() != build_id:
        raise RuntimeError("准备程序期间源码发生变化，请重新运行 ./start.sh。")
    info = json.loads(subprocess.check_output([str(player), "--template-info"], env=environment, text=True))
    if info["debug"] or info["identity"]["build_id"] != build_id:
        raise RuntimeError("Player 构建身份或 Release 配置不匹配。")
    if sys.platform == "win32":
        editor = prepare_windows_program(editor, ROOT / "editor/resources/icons/editor.ico")
        player = prepare_windows_program(player, ROOT / "app/resources/icons/app.ico")
        prepared_info = json.loads(subprocess.check_output([str(player), "--template-info"], env=environment, text=True))
        if prepared_info != info:
            raise RuntimeError("写入图标后的 Player 构建身份发生变化，请重新准备程序。")
    else:
        editor = prepare_editor_app(editor)
    app_icon = ROOT / "app/resources/icons/app.icns"
    notice_sources = {"NotoSansSC-OFL.txt": ROOT / "engine/application/resources/fonts/OFL.txt"}
    manifest = {"version": 3, "identity": info["identity"], "profile": "release", "player": player.name,
                "player_sha256": file_digest(player), "editor_sha256": file_digest(editor),
                "app_icon_sha256": icns_digest(app_icon),
                "notices": {name: file_digest(path) for name, path in notice_sources.items()}}
    encoded = json.dumps(manifest, sort_keys=True, indent=2) + "\n"
    snapshot_id = hashlib.sha256(encoded.encode()).hexdigest()
    templates = ROOT / "target/player-templates"
    templates.mkdir(parents=True, exist_ok=True)
    snapshot = templates / snapshot_id
    suffix = 0
    while snapshot.exists():
        try:
            valid = ((snapshot / "template.json").read_text() == encoded
                     and file_digest(snapshot / player.name) == manifest["player_sha256"]
                     and file_digest(snapshot / "icons/app.icns") == manifest["app_icon_sha256"]
                     and all(file_digest(snapshot / "licenses" / name) == expected for name, expected in manifest["notices"].items()))
        except OSError:
            valid = False
        if valid:
            break
        suffix += 1
        snapshot = templates / f"{snapshot_id}-{suffix}"
    if not snapshot.exists():
        with tempfile.TemporaryDirectory(prefix=".comet-", suffix=".tmp", dir=templates) as directory:
            candidate = Path(directory) / "snapshot"
            candidate.mkdir()
            shutil.copy2(player, candidate / player.name)
            if file_digest(candidate / player.name) != manifest["player_sha256"]:
                raise RuntimeError("复制期间 Player 产物发生变化，请重新运行 ./start.sh。")
            (candidate / "licenses").mkdir()
            (candidate / "icons").mkdir()
            shutil.copyfile(app_icon, candidate / "icons/app.icns")
            if file_digest(candidate / "icons/app.icns") != manifest["app_icon_sha256"]:
                raise RuntimeError("复制期间游戏图标发生变化，请重新运行 ./start.sh。")
            for name, source in notice_sources.items():
                shutil.copyfile(source, candidate / "licenses" / name)
                if file_digest(candidate / "licenses" / name) != manifest["notices"][name]:
                    raise RuntimeError("复制期间程序许可文件发生变化，请重新运行 ./start.sh。")
            (candidate / "template.json").write_text(encoded)
            try:
                candidate.rename(snapshot)
            except OSError:
                if not snapshot.is_dir() or (snapshot / "template.json").read_text() != encoded or file_digest(snapshot / player.name) != manifest["player_sha256"] or file_digest(snapshot / "icons/app.icns") != manifest["app_icon_sha256"] or any(file_digest(snapshot / "licenses" / name) != expected for name, expected in manifest["notices"].items()):
                    raise
    if source_fingerprint() != build_id:
        raise RuntimeError("准备程序期间源码发生变化，请重新运行 ./start.sh。")
    return {"editor": editor, "player": snapshot / player.name, "template": snapshot / "template.json", "build_id": build_id}


def launch_editor(command):
    if sys.platform == "win32":
        # Keep the console parent until the GUI child exits and preserve its exit status.
        raise SystemExit(subprocess.run(command).returncode)
    os.execv(command[0], command)


def main():
    parser = argparse.ArgumentParser(prog="python tools/launch.py" if sys.platform == "win32" else "./start.sh", usage="%(prog)s [编辑器参数...]",
                                     description="准备 Debug Editor 和匹配的 Release Player 模板，启动编辑器。",
                                     epilog="--project <目录> --products <目录> --scene <assets 内的场景文件>", allow_abbrev=False)
    if sys.argv[1:] in (["--help"], ["-h"]):
        parser.print_help()
        return
    arguments, supplied = application_arguments(sys.argv[1:], parser)
    programs = prepare_programs()
    defaults = []
    for flag, path in (("--project", DEMO), ("--products", PRODUCTS)):
        if flag not in supplied:
            defaults.extend((flag, str(path)))
    command = [str(programs["editor"]), "--player-template", str(programs["template"]), *defaults, *arguments]
    launch_editor(command)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode if error.returncode > 0 else 128 - error.returncode)
    except (OSError, RuntimeError, ValueError) as error:
        print(f"Comet: {error}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
