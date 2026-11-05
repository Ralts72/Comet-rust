#!/usr/bin/env python3
"""Prepare the matched Editor/Release App pair used by start.sh."""
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
from program_cache import ProgramCache, publish

ROOT = Path(__file__).resolve().parents[2]
DEMO = ROOT / "demo"
PRODUCTS = ROOT / "target/products/demo"
PATH_OPTIONS = {"--project", "--products", "--capture", "--report", "--export-game"}


def application_arguments(arguments, parser):
    result, supplied = [], set()
    arguments = iter(arguments)
    for argument in arguments:
        if argument == "--app-template":
            parser.error("App 模板由 start.sh 准备，不接受覆盖")
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
    files = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "tools/launch/launch.py", "tools/launch/program_cache.py", "tools/launch/windows_icon.py", "tools/launch/build.rs")]
    files.extend((ROOT / ".cargo").glob("*.toml"))
    for directory in ("engine", "editor", "app", "tools/asset"):
        files.extend(p for p in (ROOT / directory).rglob("*") if p.is_file()
                     and (p.suffix in (".rs", ".wgsl") or p.name == "Cargo.toml" or "resources" in p.relative_to(ROOT).parts)
                     and "tests" not in p.relative_to(ROOT).parts and p.name != "tests.rs")
    value = hashlib.sha256()
    for path in sorted(set(files)):
        value.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0")
    for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "MACOSX_DEPLOYMENT_TARGET"):
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


def prepare_editor_app(editor, snapshot):
    if sys.platform != "darwin":
        snapshot.mkdir(parents=True)
        shutil.copy2(editor, snapshot / editor.name)
        if file_digest(snapshot / editor.name) != file_digest(editor):
            raise RuntimeError("编辑器程序复制校验失败，请重新运行 ./start.sh。")
        return snapshot / editor.name
    icon = ROOT / "editor/resources/icons/editor.icns"
    plist = plistlib.dumps({"CFBundleExecutable": "comet-editor", "CFBundleName": "Comet Editor",
                           "CFBundleDisplayName": "Comet 编辑器", "CFBundleIdentifier": "org.comet.editor",
                           "CFBundleIconFile": "editor.icns", "CFBundlePackageType": "APPL",
                           "CFBundleVersion": "1", "NSHighResolutionCapable": True,
                           "LSMinimumSystemVersion": "14.2"})
    inputs = {"editor_sha256": file_digest(editor), "icon_sha256": icns_digest(icon),
              "plist_sha256": hashlib.sha256(plist).hexdigest()}
    bundle = snapshot / "Comet Editor.app"
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
    (snapshot / "bundle.json").write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n")
    if not editor_app_valid(snapshot, inputs, plist):
        raise RuntimeError("编辑器 App 校验失败，请重新运行 ./start.sh。")
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


def prepare_windows_program(executable, icon, snapshot):
    data = icon.read_bytes()
    windows_icon.icon_resources(data)
    inputs = {"name": executable.name, "program_sha256": file_digest(executable),
              "icon_sha256": hashlib.sha256(data).hexdigest()}
    snapshot.mkdir(parents=True)
    binary = snapshot / executable.name
    shutil.copy2(executable, binary)
    binary.chmod(binary.stat().st_mode | stat.S_IWRITE)
    if file_digest(binary) != inputs["program_sha256"]:
        raise RuntimeError("复制期间程序产物发生变化，请重新准备程序。")
    windows_icon.apply_icon(binary, data)
    receipt = {"inputs": inputs, "executable_sha256": file_digest(binary)}
    (snapshot / "program.json").write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n")
    if not windows_program_valid(snapshot, inputs, data):
        raise RuntimeError("Windows 程序图标校验失败，请重新准备程序。")
    return snapshot / executable.name


def program_paths(snapshot, inputs):
    editor = snapshot / "editor" / inputs["editor"]
    if sys.platform == "darwin":
        editor = snapshot / "editor/Comet Editor.app/Contents/MacOS/comet-editor"
    return {"editor": editor, "app": snapshot / "app" / inputs["app"],
            "template": snapshot / "app/template.json", "build_id": inputs["build_id"]}


def programs_valid(snapshot, inputs):
    try:
        receipt = json.loads((snapshot / "prepared.json").read_text())
        paths = program_paths(snapshot, inputs)
        required = {str(paths[name].relative_to(snapshot)) for name in ("editor", "app", "template")}
        if receipt["inputs"] != inputs or not required.issubset(receipt["files"]):
            return False
        for name, digest in receipt["files"].items():
            path = snapshot / name
            if not path.resolve().is_relative_to(snapshot.resolve()) or file_digest(path) != digest:
                return False
        if sys.platform == "darwin":
            subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(paths["editor"].parents[2])],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        elif sys.platform == "win32":
            for name in ("editor", "app"):
                windows_icon.verify_icon(paths[name], (ROOT / name / "resources/icons" / f"{name}.ico").read_bytes())
        return True
    except (OSError, ValueError, TypeError, KeyError, AttributeError, subprocess.CalledProcessError):
        return False


def prepare_programs(slot):
    """Prepare one reusable slot; the caller retains its lease through all consumers."""
    cargo = cargo_path()
    rustup = shutil.which("rustup")
    if not rustup:
        raise RuntimeError("启动需要 rustup 管理仓库锁定工具链。")
    subprocess.run([rustup, "show", "active-toolchain"], cwd=ROOT, check=True)
    build_id = source_fingerprint()
    environment = dict(os.environ, COMET_BUILD_ID=build_id)
    print("Comet: 准备 Debug Editor…", flush=True)
    editor = build_program(cargo, "comet-editor", False, environment)
    print("Comet: 准备 Release App 模板…", flush=True)
    app = build_program(cargo, "comet-app", True, environment)
    if source_fingerprint() != build_id:
        raise RuntimeError("准备程序期间源码发生变化，请重新运行 ./start.sh。")
    info = json.loads(subprocess.check_output([str(app), "--template-info"], env=environment, text=True))
    if info["debug"] or info["identity"]["build_id"] != build_id:
        raise RuntimeError("App 构建身份或 Release 配置不匹配。")
    app_icon = ROOT / "app/resources/icons/app.icns"
    notice_sources = {"NotoSansSC-OFL.txt": ROOT / "engine/application/resources/fonts/OFL.txt"}
    sources = [editor, app, app_icon, *notice_sources.values()]
    if sys.platform == "darwin":
        sources.append(ROOT / "editor/resources/icons/editor.icns")
    elif sys.platform == "win32":
        sources.extend(ROOT / name / "resources/icons" / f"{name}.ico" for name in ("editor", "app"))
    inputs = {"build_id": build_id, "identity": info["identity"], "platform": sys.platform,
              "editor": editor.name, "app": app.name,
              "sources": {str(path): file_digest(path) for path in sources}}
    snapshot = slot / "programs"
    if programs_valid(snapshot, inputs):
        if source_fingerprint() != build_id:
            raise RuntimeError("准备程序期间源码发生变化，请重新运行 ./start.sh。")
        return program_paths(snapshot, inputs)
    with tempfile.TemporaryDirectory(prefix=".prepare-", suffix=".tmp", dir=slot) as directory:
        candidate = Path(directory) / "programs"
        candidate.mkdir()
        if sys.platform == "win32":
            prepared_editor = prepare_windows_program(editor, ROOT / "editor/resources/icons/editor.ico", candidate / "editor")
            prepared_app = prepare_windows_program(app, ROOT / "app/resources/icons/app.ico", candidate / "app")
            prepared_info = json.loads(subprocess.check_output([str(prepared_app), "--template-info"], env=environment, text=True))
            if prepared_info != info:
                raise RuntimeError("写入图标后的 App 构建身份发生变化，请重新准备程序。")
        else:
            prepared_editor = prepare_editor_app(editor, candidate / "editor")
            (candidate / "app").mkdir()
            shutil.copy2(app, candidate / "app" / app.name)
        template = candidate / "app"
        (template / "licenses").mkdir()
        (template / "icons").mkdir()
        shutil.copyfile(app_icon, template / "icons/app.icns")
        for name, source in notice_sources.items():
            shutil.copyfile(source, template / "licenses" / name)
        manifest = {"version": 4, "identity": info["identity"], "profile": "release", "app": app.name,
                    "app_sha256": file_digest(template / app.name), "editor_sha256": file_digest(prepared_editor),
                    "app_icon_sha256": icns_digest(app_icon),
                    "notices": {name: file_digest(path) for name, path in notice_sources.items()}}
        (template / "template.json").write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n")
        if ({str(path): file_digest(path) for path in sources} != inputs["sources"]
                or source_fingerprint() != build_id):
            raise RuntimeError("准备程序期间输入发生变化，请重新运行 ./start.sh。")
        if ((sys.platform != "win32" and file_digest(template / app.name) != inputs["sources"][str(app)])
                or file_digest(template / "icons/app.icns") != inputs["sources"][str(app_icon)]
                or any(file_digest(template / "licenses" / name) != inputs["sources"][str(source)] for name, source in notice_sources.items())):
            raise RuntimeError("程序模板复制校验失败，请重新运行 ./start.sh。")
        receipt = {"inputs": inputs, "files": {str(path.relative_to(candidate)): file_digest(path)
                                             for path in candidate.rglob("*") if path.is_file()}}
        (candidate / "prepared.json").write_text(json.dumps(receipt, sort_keys=True, indent=2) + "\n")
        if not programs_valid(candidate, inputs):
            raise RuntimeError("程序候选校验失败，请重新运行 ./start.sh。")
        snapshot = publish(slot, candidate)
    return program_paths(snapshot, inputs)


def launch_editor(command, cache):
    if sys.platform == "win32":
        # Keep the console parent until the GUI child exits and preserve its exit status.
        result = subprocess.run(command)
    else:
        # The editor inherits the lease, so terminating the launcher cannot free its slot.
        result = subprocess.run(command, pass_fds=(cache.lock.fileno(),))
    raise SystemExit(result.returncode if result.returncode >= 0 else 128 - result.returncode)


def main():
    parser = argparse.ArgumentParser(prog="python tools/launch/launch.py" if sys.platform == "win32" else "./start.sh", usage="%(prog)s [编辑器参数...]",
                                     description="准备 Debug Editor 和匹配的 Release App 模板，启动编辑器。",
                                     epilog="--project <目录> --products <目录> --scene <assets 内的 .scene 或 .prefab 文件>", allow_abbrev=False)
    if sys.argv[1:] in (["--help"], ["-h"]):
        parser.print_help()
        return
    arguments, supplied = application_arguments(sys.argv[1:], parser)
    with ProgramCache(ROOT / "target/launch") as cache:
        programs = prepare_programs(cache.slot)
        defaults = []
        for flag, path in (("--project", DEMO), ("--products", PRODUCTS)):
            if flag not in supplied:
                defaults.extend((flag, str(path)))
        command = [str(programs["editor"]), "--app-template", str(programs["template"]), *defaults, *arguments]
        launch_editor(command, cache)


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
