#!/usr/bin/env python3
"""Tk launcher for Foundation: pick the optional modules and boot options, then run foundation/run.py.

    python foundation/launcher.py

Python 3.10+, standard library only. This is separate from tools/cpt32-launcher (which starts
arbitrary guest programs): it only builds and boots Foundation, and it exposes what
foundation/run.py and foundation/boot/build.py can switch:

- /bin commands (build.py --commands) and optional /lib WALs (--libs), chosen from a list;
- boot path (WAX direct or disk through the BIOS), Main RAM, disk size, Clang, jobs;
- FND_MM=nommu (kernel on physical memory only), external --bin-dir / --root.

The logic (catalog, dependency rules, command line) lives outside the Tk classes so that
foundation/tests/launcher can test it without a display.
"""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import threading

FOUNDATION = Path(__file__).resolve().parent
ROOT = FOUNDATION.parent
RUN = FOUNDATION / "run.py"
if os.name == "nt" and os.environ.get("LOCALAPPDATA"):
    STATE_FILE = Path(os.environ["LOCALAPPDATA"]) / "CPT32" / "Launcher" / "foundation.json"
else:
    STATE_FILE = Path.home() / ".config" / "CPT32" / "Launcher" / "foundation.json"

# One line per module for the list. A name missing here is still listed (with no description),
# so a new command added to build.py needs no change in this file to become selectable.
DESCRIPTIONS = {
    "cat": "ファイルの内容を表示", "clear": "画面を消去", "cp": "ファイルをコピー", "echo": "引数を表示",
    "free": "メモリの空き容量", "hex": "16進ダンプ", "ls": "ディレクトリ一覧", "mkdir": "ディレクトリ作成",
    "mode": "表示モードの切り替え", "mv": "移動・名前変更", "ps": "プロセス一覧", "pwd": "カレントディレクトリ",
    "rm": "ファイル削除", "rmdir": "空ディレクトリ削除", "top": "CPU 使用率の表示", "uname": "システム情報",
    "tinybasic": "Tiny BASIC", "chip8": "CHIP-8 エミュレータ（libsgc が必要）", "bmpview": "BMP 画像の表示",
    "sgubplay": "SGUB 音楽の再生", "wavplay": "WAV の再生",
    "lzma": "LZMA 圧縮・展開（conde）", "rle": "PackBits RLE（conde）", "code": "文字コード変換",
    "crypt": "ChaCha20 / Poly1305", "sgc": "libsgc（SGC 2D 描画）",
}


def load_build_module():
    """foundation/boot/build.py, the single source of the module lists."""
    spec = importlib.util.spec_from_file_location("foundation_launcher_build", FOUNDATION / "boot/build.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Catalog:
    """The selectable modules and what depends on what."""

    def __init__(self, commands, libs, sgc_commands):
        self.commands = list(commands)
        self.libs = list(libs)
        # command -> libs it imports (besides fnd, which is always built)
        self.needs = {name: ["sgc"] for name in sgc_commands}

    @classmethod
    def from_build(cls, build=None):
        build = build or load_build_module()
        return cls(build.COMMANDS, build.LIBS, build.SGC_COMMANDS)

    def toggle(self, selected, kind, name, on):
        """Return the selection after switching one module, keeping dependencies consistent.

        selected is {"commands": set, "libs": set}. Enabling a command also enables the libs it
        imports; disabling a lib also disables the commands that import it.
        """
        result = {key: set(value) for key, value in selected.items()}
        group = result[kind]
        if on:
            group.add(name)
            if kind == "commands":
                result["libs"].update(self.needs.get(name, ()))
        else:
            group.discard(name)
            if kind == "libs":
                for command, libs in self.needs.items():
                    if name in libs:
                        result["commands"].discard(command)
        return result

    def everything(self):
        return {"commands": set(self.commands), "libs": set(self.libs)}

    def nothing(self):
        return {"commands": set(), "libs": set()}


def selection_argument(selected, available):
    """None when everything is selected (build.py's default), otherwise the comma-separated subset."""
    chosen = [name for name in available if name in selected]
    return None if len(chosen) == len(available) else ",".join(chosen)


def default_settings(catalog):
    return {
        "commands": list(catalog.commands), "libs": list(catalog.libs),
        "disk": False, "build_only": False, "nommu": False,
        "ram_mib": "16", "disk_blocks": "", "clang": "", "jobs": "", "root": "", "bin_dirs": [],
        "extra_args": "",
    }


def validate(settings, catalog):
    """Return a list of problems in Japanese; empty means the settings can run."""
    problems = []
    if not settings["ram_mib"].isdigit() or not 1 <= int(settings["ram_mib"]) <= 256:
        problems.append("RAM は 1〜256 MiB の整数で指定してください")
    for key, label in (("disk_blocks", "ディスクブロック数"), ("jobs", "並列数")):
        value = settings[key].strip()
        if value and (not value.isdigit() or int(value) < 1):
            problems.append(f"{label}は 1 以上の整数で指定してください")
    for name in settings["commands"]:
        for lib in catalog.needs.get(name, ()):
            if lib not in settings["libs"]:
                problems.append(f"{name} には {lib} ライブラリが必要です")
    for key in ("root",):
        if settings[key].strip() and not Path(settings[key]).is_dir():
            problems.append(f"ディレクトリがありません: {settings[key]}")
    for directory in settings["bin_dirs"]:
        if not Path(directory).is_dir():
            problems.append(f"bin ディレクトリがありません: {directory}")
    return problems


def build_invocation(settings, catalog, python=None):
    """(argv, env_overrides) for foundation/run.py."""
    argv = [python or sys.executable, str(RUN)]
    for key, available, option in (("commands", catalog.commands, "--commands"),
                                   ("libs", catalog.libs, "--libs")):
        value = selection_argument(settings[key], available)
        if value is not None:
            argv += [option, value]
    argv += ["--ram-mib", settings["ram_mib"].strip()]
    if settings["disk_blocks"].strip():
        argv += ["--disk-blocks", settings["disk_blocks"].strip()]
    if settings["clang"].strip():
        argv += ["--clang", settings["clang"].strip()]
    if settings["jobs"].strip():
        argv += ["-j", settings["jobs"].strip()]
    if settings["root"].strip():
        argv += ["--root", settings["root"].strip()]
    for directory in settings["bin_dirs"]:
        argv += ["--bin-dir", directory]
    if settings["disk"]:
        argv.append("--disk")
    if settings["build_only"]:
        argv.append("--build-only")
    if settings["extra_args"].strip():
        import shlex
        # run.py's --arg consumes the rest of the line, so it goes last.
        argv += ["--arg", *shlex.split(settings["extra_args"], posix=os.name != "nt")]
    return argv, ({"FND_MM": "nommu"} if settings["nommu"] else {})


def load_state(catalog, path=STATE_FILE):
    """Saved settings merged over the defaults; unknown modules are dropped, new ones start enabled."""
    settings = default_settings(catalog)
    try:
        saved = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return settings
    if not isinstance(saved, dict):
        return settings
    known = {"commands": catalog.commands, "libs": catalog.libs}
    for key, names in known.items():
        if isinstance(saved.get(key), list):
            # A module that did not exist when the state was saved is enabled, like a fresh install.
            seen = set(saved.get(f"known_{key}", saved[key]))
            settings[key] = [n for n in names if n in saved[key] or n not in seen]
    for key, default in settings.items():
        if key in known or key not in saved:
            continue
        if isinstance(default, bool) and isinstance(saved[key], bool):
            settings[key] = saved[key]
        elif isinstance(default, str) and isinstance(saved[key], str):
            settings[key] = saved[key]
        elif isinstance(default, list) and isinstance(saved[key], list) and all(isinstance(i, str) for i in saved[key]):
            settings[key] = saved[key]
    return settings


def save_state(settings, catalog, path=STATE_FILE):
    document = dict(settings, known_commands=catalog.commands, known_libs=catalog.libs)
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(document, ensure_ascii=False, indent=2), encoding="utf-8")
    os.replace(temporary, path)


def run_gui():
    import tkinter as tk
    from tkinter import filedialog, messagebox, ttk

    catalog = Catalog.from_build()

    class FoundationLauncher(tk.Tk):
        CHECKED, UNCHECKED = "☑", "☐"

        def __init__(self):
            super().__init__()
            self.title("Foundation ランチャー")
            self.geometry("1000x720")
            self.minsize(820, 560)
            self.events: queue.Queue = queue.Queue()
            self.process: subprocess.Popen | None = None
            settings = load_state(catalog)
            self.selected = {"commands": set(settings["commands"]), "libs": set(settings["libs"])}
            self.disk = tk.BooleanVar(value=settings["disk"])
            self.build_only = tk.BooleanVar(value=settings["build_only"])
            self.nommu = tk.BooleanVar(value=settings["nommu"])
            self.text = {key: tk.StringVar(value=settings[key])
                         for key in ("ram_mib", "disk_blocks", "clang", "jobs", "root", "extra_args")}
            self._build_ui(settings)
            self.after(100, self._poll)
            self.protocol("WM_DELETE_WINDOW", self._close)

        def _build_ui(self, settings):
            self.columnconfigure(0, weight=3)
            self.columnconfigure(1, weight=2)
            self.rowconfigure(0, weight=1)
            modules = ttk.LabelFrame(self, text="モジュール（クリックで切り替え）")
            modules.grid(row=0, column=0, sticky="nsew", padx=8, pady=8)
            modules.columnconfigure(0, weight=1)
            modules.rowconfigure(0, weight=1)
            self.tree = ttk.Treeview(modules, columns=("state", "description"), show="tree headings",
                                     selectmode="browse")
            self.tree.heading("#0", text="名前")
            self.tree.heading("state", text="")
            self.tree.heading("description", text="説明")
            self.tree.column("#0", width=150)
            self.tree.column("state", width=40, anchor="center", stretch=False)
            self.tree.column("description", width=300)
            self.tree.grid(row=0, column=0, sticky="nsew")
            scroll = ttk.Scrollbar(modules, command=self.tree.yview)
            scroll.grid(row=0, column=1, sticky="ns")
            self.tree.configure(yscrollcommand=scroll.set)
            self.tree.insert("", "end", "group:commands", text="/bin コマンド", open=True)
            self.tree.insert("", "end", "group:libs", text="/lib ライブラリ（WAL）", open=True)
            for kind, names in (("commands", catalog.commands), ("libs", catalog.libs)):
                for name in names:
                    self.tree.insert(f"group:{kind}", "end", f"{kind}:{name}", text=name,
                                     values=("", DESCRIPTIONS.get(name, "")))
            self._refresh_tree()
            self.tree.bind("<Button-1>", self._click)
            self.tree.bind("<space>", self._space)
            buttons = ttk.Frame(modules)
            buttons.grid(row=1, column=0, columnspan=2, sticky="ew", pady=4)
            ttk.Button(buttons, text="すべて選択", command=lambda: self._set_all(True)).pack(side="left", padx=2)
            ttk.Button(buttons, text="すべて解除", command=lambda: self._set_all(False)).pack(side="left", padx=2)
            self.summary = ttk.Label(buttons)
            self.summary.pack(side="right", padx=4)
            self._update_summary()

            options = ttk.LabelFrame(self, text="起動オプション")
            options.grid(row=0, column=1, sticky="nsew", padx=(0, 8), pady=8)
            options.columnconfigure(1, weight=1)
            row = 0
            ttk.Checkbutton(options, text="ディスク起動（BIOS → stage1 → FNDBOOT）", variable=self.disk
                            ).grid(row=row, column=0, columnspan=3, sticky="w", padx=4, pady=2)
            row += 1
            ttk.Checkbutton(options, text="nommu（Extension V を使わず物理メモリで動かす）", variable=self.nommu
                            ).grid(row=row, column=0, columnspan=3, sticky="w", padx=4, pady=2)
            row += 1
            ttk.Checkbutton(options, text="ビルドのみ（エミュレータを開かない）", variable=self.build_only
                            ).grid(row=row, column=0, columnspan=3, sticky="w", padx=4, pady=2)
            row += 1
            for key, label, chooser in (("ram_mib", "Main RAM (MiB)", None),
                                        ("disk_blocks", "ディスク (512B ブロック)", None),
                                        ("jobs", "並列ビルド数", None),
                                        ("clang", "Clang", "file"),
                                        ("root", "--root ディレクトリ", "dir")):
                ttk.Label(options, text=label).grid(row=row, column=0, sticky="w", padx=4, pady=2)
                ttk.Entry(options, textvariable=self.text[key]).grid(row=row, column=1, sticky="ew", pady=2)
                if chooser:
                    ttk.Button(options, text="…", width=3,
                               command=lambda k=key, c=chooser: self._choose(k, c)).grid(row=row, column=2, padx=2)
                row += 1
            ttk.Label(options, text="エミュレータ引数").grid(row=row, column=0, sticky="w", padx=4, pady=2)
            ttk.Entry(options, textvariable=self.text["extra_args"]).grid(row=row, column=1, sticky="ew", pady=2)
            row += 1
            ttk.Label(options, text="外部 bin ディレクトリ\n（*.wax を /bin に追加）").grid(
                row=row, column=0, sticky="nw", padx=4, pady=2)
            self.bin_dirs = tk.Listbox(options, height=4)
            self.bin_dirs.grid(row=row, column=1, sticky="nsew", pady=2)
            for directory in settings["bin_dirs"]:
                self.bin_dirs.insert("end", directory)
            side = ttk.Frame(options)
            side.grid(row=row, column=2, sticky="n")
            ttk.Button(side, text="＋", width=3, command=self._add_bin_dir).pack(pady=1)
            ttk.Button(side, text="−", width=3, command=self._remove_bin_dir).pack(pady=1)

            actions = ttk.Frame(self)
            actions.grid(row=1, column=0, columnspan=2, sticky="ew", padx=8)
            self.launch_button = ttk.Button(actions, text="ビルドして起動", command=self.launch)
            self.launch_button.pack(side="left")
            self.stop_button = ttk.Button(actions, text="停止", command=self.stop, state="disabled")
            self.stop_button.pack(side="left", padx=4)
            ttk.Button(actions, text="設定を保存", command=self.save).pack(side="left", padx=4)
            ttk.Button(actions, text="コマンドをコピー", command=self._copy_command).pack(side="left", padx=4)
            self.rowconfigure(2, weight=2)
            log = ttk.Frame(self)
            log.grid(row=2, column=0, columnspan=2, sticky="nsew", padx=8, pady=8)
            log.columnconfigure(0, weight=1)
            log.rowconfigure(0, weight=1)
            self.log = tk.Text(log, height=10, state="disabled", wrap="none")
            self.log.grid(row=0, column=0, sticky="nsew")
            ys = ttk.Scrollbar(log, command=self.log.yview)
            ys.grid(row=0, column=1, sticky="ns")
            self.log.configure(yscrollcommand=ys.set)

        # --- module list -------------------------------------------------------------------------
        def _refresh_tree(self):
            for kind in ("commands", "libs"):
                for name in getattr(catalog, kind):
                    mark = self.CHECKED if name in self.selected[kind] else self.UNCHECKED
                    self.tree.set(f"{kind}:{name}", "state", mark)

        def _update_summary(self):
            self.summary.configure(text=f"コマンド {len(self.selected['commands'])}/{len(catalog.commands)}  "
                                        f"ライブラリ {len(self.selected['libs'])}/{len(catalog.libs)}")

        def _toggle(self, item):
            kind, _, name = item.partition(":")
            if kind not in ("commands", "libs"):
                return
            self.selected = catalog.toggle(self.selected, kind, name, name not in self.selected[kind])
            self._refresh_tree()
            self._update_summary()

        def _click(self, event):
            if self.tree.identify_region(event.x, event.y) == "cell" and self.tree.identify_column(event.x) == "#1":
                self._toggle(self.tree.identify_row(event.y))
            elif self.tree.identify_region(event.x, event.y) == "tree":
                item = self.tree.identify_row(event.y)
                if item and not item.startswith("group:") and event.x > 30:
                    self._toggle(item)

        def _space(self, _event):
            for item in self.tree.selection():
                self._toggle(item)
            return "break"

        def _set_all(self, on):
            self.selected = catalog.everything() if on else catalog.nothing()
            self._refresh_tree()
            self._update_summary()

        # --- options -----------------------------------------------------------------------------
        def _choose(self, key, kind):
            path = filedialog.askdirectory() if kind == "dir" else filedialog.askopenfilename()
            if path:
                self.text[key].set(path)

        def _add_bin_dir(self):
            path = filedialog.askdirectory(title="*.wax を含むディレクトリ")
            if path:
                self.bin_dirs.insert("end", path)

        def _remove_bin_dir(self):
            for index in reversed(self.bin_dirs.curselection()):
                self.bin_dirs.delete(index)

        def settings(self):
            result = {key: variable.get() for key, variable in self.text.items()}
            result.update(
                commands=[n for n in catalog.commands if n in self.selected["commands"]],
                libs=[n for n in catalog.libs if n in self.selected["libs"]],
                disk=self.disk.get(), build_only=self.build_only.get(), nommu=self.nommu.get(),
                bin_dirs=list(self.bin_dirs.get(0, "end")))
            return result

        def save(self):
            try:
                save_state(self.settings(), catalog)
            except OSError as error:
                messagebox.showerror("保存できません", str(error))

        # --- running -----------------------------------------------------------------------------
        def _write(self, text):
            self.log.configure(state="normal")
            self.log.insert("end", text)
            self.log.see("end")
            self.log.configure(state="disabled")

        def _invocation(self):
            settings = self.settings()
            problems = validate(settings, catalog)
            if problems:
                messagebox.showerror("設定を確認してください", "\n".join(problems))
                return None
            return build_invocation(settings, catalog)

        def _copy_command(self):
            invocation = self._invocation()
            if invocation:
                argv, env = invocation
                import shlex
                prefix = "".join(f"{k}={v} " for k, v in env.items())
                self.clipboard_clear()
                self.clipboard_append(prefix + " ".join(shlex.quote(a) for a in argv))

        def launch(self):
            if self.process is not None:
                return
            invocation = self._invocation()
            if not invocation:
                return
            argv, env = invocation
            self.save()
            self._write("$ " + " ".join(argv) + "\n")
            try:
                self.process = subprocess.Popen(argv, cwd=ROOT, env=dict(os.environ, **env), text=True,
                                                encoding="utf-8", errors="replace", stdout=subprocess.PIPE,
                                                stderr=subprocess.STDOUT,
                                                start_new_session=os.name != "nt")
            except OSError as error:
                self._write(f"起動できません: {error}\n")
                return
            self.launch_button.configure(state="disabled")
            self.stop_button.configure(state="normal")
            threading.Thread(target=self._read, args=(self.process,), daemon=True).start()

        def _read(self, process):
            for line in process.stdout:
                self.events.put(("line", line))
            self.events.put(("exit", process.wait()))

        def _poll(self):
            try:
                for _ in range(256):
                    kind, value = self.events.get_nowait()
                    if kind == "line":
                        self._write(value)
                    else:
                        self._write(f"[終了コード {value}]\n")
                        self.process = None
                        self.launch_button.configure(state="normal")
                        self.stop_button.configure(state="disabled")
            except queue.Empty:
                pass
            self.after(100, self._poll)

        def stop(self):
            process = self.process
            if process is None:
                return
            # run.py starts cargo and the emulator as children: stop the whole tree.
            if os.name == "nt":
                subprocess.run(["taskkill", "/T", "/F", "/PID", str(process.pid)], capture_output=True)
            else:
                import signal
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass

        def _close(self):
            self.stop()
            self.destroy()

    FoundationLauncher().mainloop()


if __name__ == "__main__":
    run_gui()
