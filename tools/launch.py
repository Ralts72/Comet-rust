#!/usr/bin/env python3
"""Shared launcher for the Editor and Player scripts."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
DEMO = ROOT / "demo"
PRODUCTS = ROOT / "target/products/demo"
ENTRY_SCENE = "40000000-0000-4000-8000-000000000001"
PATH_OPTIONS = {"--project", "--products", "--capture", "--report"}


def application_arguments(arguments, parser):
    """Resolve path values before Cargo changes cwd; scene stays relative to assets."""
    result = []
    supplied = set()
    arguments = iter(arguments)
    for argument in arguments:
        result.append(argument)
        if argument in PATH_OPTIONS or argument == "--scene":
            value = next(arguments, None)
            if value is None:
                parser.error(f"{argument} 缺少参数")
            supplied.add(argument)
            result.append(str(Path(value).absolute()) if argument in PATH_OPTIONS else value)
        elif not argument.startswith("--"):
            parser.error(f"不接受动作或位置参数：{argument}")
    return result, supplied


def cargo_path():
    cargo = shutil.which("cargo")
    if cargo:
        return str(Path(cargo).absolute())
    for directory in (
        Path.home() / ".cargo/bin",
        Path("/opt/homebrew/opt/rustup/bin"),
        Path("/usr/local/opt/rustup/bin"),
    ):
        cargo = directory / "cargo"
        if cargo.is_file() and os.access(cargo, os.X_OK):
            os.environ["PATH"] = str(directory) + os.pathsep + os.environ.get("PATH", "")
            return str(cargo)
    raise RuntimeError("找不到 cargo，请先安装 rustup，并将其命令目录加入 PATH。")


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("editor", "player"):
        raise RuntimeError("请使用仓库根目录的 editor.sh 或 run_game.sh")
    application = sys.argv[1]
    is_editor = application == "editor"
    script = "editor.sh" if is_editor else "run_game.sh"
    parser = argparse.ArgumentParser(
        prog=f"./{script}",
        usage="%(prog)s [应用参数...]",
        description="以 Debug 配置编译并启动编辑器。" if is_editor else "以 Release 配置准备资源并运行游戏。",
        epilog=("编辑器参数：--project <目录> --products <目录> --scene <作者场景文件名>"
                if is_editor else "游戏参数：--products <现成产品目录> --scene <资产 UUID>"),
        allow_abbrev=False,
    )
    arguments = sys.argv[2:]
    if arguments in (["--help"], ["-h"]):
        parser.print_help()
        return
    app_args, supplied = application_arguments(arguments, parser)

    cargo = cargo_path()
    profile = ["--profile", "dev" if is_editor else "release"]
    if is_editor:
        defaults = {"--project": str(DEMO), "--products": str(PRODUCTS)}
    elif "--products" not in supplied:
        subprocess.run(
            [cargo, "run", "--locked", *profile, "-p", "comet-assetc", "--",
             "prepare", str(DEMO), "--output", str(PRODUCTS)],
            cwd=ROOT, check=True,
        )
        defaults = {"--products": str(PRODUCTS), "--scene": ENTRY_SCENE}
    else:
        if "--scene" not in supplied:
            parser.error("指定现成 --products 时，还需提供 --scene <资产 UUID>")
        defaults = {}
    default_args = []
    for flag, value in defaults.items():
        if flag not in supplied:
            default_args.extend((flag, value))
    command = [cargo, "run", "--locked", *profile, "-p", f"comet-{application}",
               "--", *default_args, *app_args]
    os.chdir(ROOT)
    os.execv(cargo, command)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode if error.returncode > 0 else 128 - error.returncode)
    except (OSError, RuntimeError) as error:
        print(f"Comet: {error}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        sys.exit(130)
