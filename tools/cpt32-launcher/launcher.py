#!/usr/bin/env python3
"""Small Tk launcher for CPT32. Python 3.10+, standard library only."""

from __future__ import annotations

import json
import os
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import copy
from pathlib import Path
import tkinter as tk
from tkinter import filedialog, messagebox, ttk


CPU_EXECUTION_MODES = ("strict", "best-effort", "cycle-independent")
DEFAULT_CPU_EXECUTION_MODE = "best-effort"
DEFAULT_CPU_TIME_SLICE_MS = 8
CPU_TIME_SLICE_RANGE_MS = range(1, 17)
REPO = Path(__file__).resolve().parents[2]
CPU_KINDS = ("pipeline", "cached", "cached-relaxed", "jit", "scalar")
if os.name == "nt" and os.environ.get("LOCALAPPDATA"):
    PROFILE_FILE = Path(os.environ["LOCALAPPDATA"]) / "CPT32" / "Launcher" / "profiles.json"
else:
    PROFILE_FILE = Path.home() / ".config" / "CPT32" / "Launcher" / "profiles.json"
SAFE_CONFIG_PATHS = (
    ("bios", "image"), ("pcg", "font"), ("disk_sound", "asset_root"),
    ("expansion_card", "package"),
)


class ConfigError(ValueError):
    pass


def _strip_comment(line: str) -> str:
    quote = None
    escaped = False
    for i, char in enumerate(line):
        if escaped:
            escaped = False
        elif char == "\\" and quote == '"':
            escaped = True
        elif char in "'\"":
            if quote == char:
                quote = None
            elif quote is None:
                quote = char
        elif char == "#" and quote is None and (i == 0 or line[i - 1].isspace()):
            return line[:i].rstrip()
    return line.rstrip()


def _key_value(text: str, line_number: int) -> tuple[str, str]:
    quote = None
    escaped = False
    for i, char in enumerate(text):
        if escaped:
            escaped = False
        elif char == "\\" and quote == '"':
            escaped = True
        elif char in "'\"":
            if quote == char:
                quote = None
            elif quote is None:
                quote = char
        elif char == ":" and quote is None and (i + 1 == len(text) or text[i + 1].isspace()):
            raw_key = text[:i].strip()
            if raw_key.startswith('"'):
                key = json.loads(raw_key)
            elif raw_key.startswith("'") and raw_key.endswith("'"):
                key = raw_key[1:-1].replace("''", "'")
            else:
                key = raw_key
            if not isinstance(key, str) or not key:
                raise ConfigError(f"YAML line {line_number}: expected a non-empty text key")
            return key, text[i + 1 :].strip()
    raise ConfigError(f"YAML line {line_number}: expected a mapping entry")


def _scalar(text: str, line_number: int):
    text = text.strip()
    if not text or text in ("null", "Null", "NULL", "~"):
        return None
    if text.startswith(("&", "*", "!", "|", ">")):
        raise ConfigError(f"YAML line {line_number}: aliases, tags, and block strings need PyYAML")
    if text.startswith('"'):
        try:
            return json.loads(text)
        except json.JSONDecodeError as error:
            raise ConfigError(f"YAML line {line_number}: invalid quoted string: {error}") from error
    if text.startswith("'") and text.endswith("'"):
        return text[1:-1].replace("''", "'")
    if text.startswith(("[", "{")):
        try:
            return json.loads(text)
        except json.JSONDecodeError as error:
            raise ConfigError(f"YAML line {line_number}: flow collections must use JSON syntax or install PyYAML") from error
    lowered = text.lower()
    if lowered in ("true", "false"):
        return lowered == "true"
    if lowered in ("null", "~"):
        return None
    if re.fullmatch(r"[-+]?[0-9]+", text):
        return int(text, 10)
    if re.fullmatch(r"[-+]?(?:0[xX][0-9a-fA-F_]+|0[oO][0-7_]+|0[bB][01_]+|[0-9][0-9_]*_+[0-9_]*)", text):
        return int(text.replace("_", ""), 0 if text.lstrip("+-").lower().startswith(("0x", "0o", "0b")) else 10)
    if re.fullmatch(r"[-+]?(?:[0-9]+\.[0-9]*|\.[0-9]+)(?:[eE][-+]?[0-9]+)?", text):
        return float(text)
    return text


def parse_yaml(text: str):
    """Read common config.yaml mappings/sequences without a third-party module."""
    try:
        import yaml  # type: ignore
    except ImportError:
        yaml = None
    if yaml is not None:
        try:
            value = yaml.safe_load(text)
        except yaml.YAMLError as error:
            raise ConfigError(f"Invalid YAML: {error}") from error
        return {} if value is None else value

    rows = []
    for number, source in enumerate(text.splitlines(), 1):
        if "\t" in source[: len(source) - len(source.lstrip())]:
            raise ConfigError(f"YAML line {number}: indentation must use spaces")
        cleaned = _strip_comment(source)
        if not cleaned.strip() or cleaned.lstrip().startswith("---") or cleaned.lstrip() == "...":
            continue
        indent = len(cleaned) - len(cleaned.lstrip(" "))
        rows.append((indent, cleaned.strip(), number))
    if not rows:
        return {}

    def parse_block(index, indent):
        if index >= len(rows) or rows[index][0] != indent:
            raise ConfigError(f"YAML line {rows[index][2] if index < len(rows) else '?'}: invalid indentation")
        is_sequence = rows[index][1] == "-" or rows[index][1].startswith("- ")
        result = [] if is_sequence else {}
        while index < len(rows) and rows[index][0] == indent:
            _, content, number = rows[index]
            if is_sequence:
                if not (content == "-" or content.startswith("- ")):
                    raise ConfigError(f"YAML line {number}: cannot mix list and mapping items")
                item = content[1:].strip()
                index += 1
                if not item:
                    if index < len(rows) and rows[index][0] > indent:
                        value, index = parse_block(index, rows[index][0])
                    else:
                        value = None
                    result.append(value)
                    continue
                try:
                    key, raw = _key_value(item, number)
                except ConfigError:
                    value = _scalar(item, number)
                else:
                    mapping = {}
                    mapping[key], index = parse_value(index, indent, raw, number)
                    if index < len(rows) and rows[index][0] > indent:
                        continuation_indent = rows[index][0]
                        continuation, index = parse_block(index, continuation_indent)
                        if not isinstance(continuation, dict):
                            raise ConfigError(f"YAML line {rows[index - 1][2]}: expected mapping fields")
                        mapping.update(continuation)
                    result.append(mapping)
                    continue
                if index < len(rows) and rows[index][0] > indent:
                    raise ConfigError(f"YAML line {rows[index][2]}: scalar list item cannot have nested fields")
                result.append(value)
                continue
            if content.startswith("- ") or content == "-":
                raise ConfigError(f"YAML line {number}: cannot mix mapping and list items")
            key, raw = _key_value(content, number)
            if key in result:
                raise ConfigError(f"YAML line {number}: duplicate key {key!r}")
            index += 1
            result[key], index = parse_value(index, indent, raw, number)
        return result, index

    def parse_value(index, parent_indent, raw, number):
        if raw:
            return _scalar(raw, number), index
        if index < len(rows) and rows[index][0] > parent_indent:
            return parse_block(index, rows[index][0])
        return None, index

    parsed, end = parse_block(0, rows[0][0])
    if end != len(rows) or rows[0][0] != 0:
        raise ConfigError(f"YAML line {rows[end][2] if end < len(rows) else rows[0][2]}: unsupported document structure")
    return parsed


def dump_yaml(value) -> str:
    try:
        import yaml  # type: ignore
    except ImportError:
        yaml = None
    if yaml is not None:
        return yaml.safe_dump(value, allow_unicode=True, sort_keys=False, default_flow_style=False)

    def scalar(item):
        if item is None:
            return "null"
        if isinstance(item, bool):
            return "true" if item else "false"
        if isinstance(item, str):
            # JSON strings are valid YAML scalars and preserve Windows paths.
            return json.dumps(item, ensure_ascii=False)
        if isinstance(item, (int, float)):
            return repr(item)
        raise ConfigError(f"Cannot write YAML value: {type(item).__name__}")

    def emit(item, indent=0):
        pad = " " * indent
        if isinstance(item, dict):
            if not item:
                return [pad + "{}"]
            lines = []
            for key, child in item.items():
                if not isinstance(key, str):
                    raise ConfigError("Configuration keys must be strings")
                rendered_key = json.dumps(key, ensure_ascii=False)
                if isinstance(child, (dict, list)) and child:
                    lines.append(pad + rendered_key + ":")
                    lines.extend(emit(child, indent + 2))
                elif isinstance(child, dict):
                    lines.append(pad + rendered_key + ": {}")
                elif isinstance(child, list):
                    lines.append(pad + rendered_key + ": []")
                else:
                    lines.append(pad + rendered_key + ": " + scalar(child))
            return lines
        if isinstance(item, list):
            if not item:
                return [pad + "[]"]
            lines = []
            for child in item:
                if isinstance(child, dict) and child:
                    lines.append(pad + "-")
                    lines.extend(emit(child, indent + 2))
                elif isinstance(child, (dict, list)):
                    lines.append(pad + "-")
                    lines.extend(emit(child, indent + 2))
                else:
                    lines.append(pad + "- " + scalar(child))
            return lines
        return [pad + scalar(item)]

    return "\n".join(emit(value)) + "\n"


def resolve_config_path(value: str, base: Path) -> str:
    path = Path(value).expanduser()
    if not path.is_absolute():
        path = base / path
    return str(path.resolve())


def prepare_run_config(document, config_base: Path, selected_cpu: str) -> dict:
    if document is None:
        document = {}
    if not isinstance(document, dict):
        raise ConfigError("The configuration root must be a YAML mapping")
    staged = copy.deepcopy(document)
    cpu = staged.get("cpu") or {}
    if not isinstance(cpu, dict):
        raise ConfigError("`cpu` must be a mapping")
    cpu["microarchitecture"] = selected_cpu
    staged["cpu"] = cpu
    for section, key in SAFE_CONFIG_PATHS:
        value = staged.get(section)
        if section == "expansion_card":
            if isinstance(value, list):
                for card in value:
                    if isinstance(card, dict) and isinstance(card.get(key), str):
                        card[key] = resolve_config_path(card[key], config_base)
        elif isinstance(value, dict) and isinstance(value.get(key), str):
            value[key] = resolve_config_path(value[key], config_base)
    sound = staged.get("disk_sound")
    if isinstance(sound, dict) and "asset_root" not in sound:
        # The emulator's sound search roots normally include config.yaml's
        # directory. The staged YAML lives elsewhere, so retain that lookup.
        sound["asset_root"] = str(config_base.resolve())
    disks = staged.get("disk")
    if isinstance(disks, list):
        for disk in disks:
            if isinstance(disk, dict) and isinstance(disk.get("image"), str):
                disk["image"] = resolve_config_path(disk["image"], config_base)
    return staged


def user_profile_path() -> Path:
    if os.name == "nt" and os.environ.get("LOCALAPPDATA"):
        base = Path(os.environ["LOCALAPPDATA"])
    else:
        base = Path.home() / ".config"
    return base / "CPT32" / "Launcher" / "profiles.json"


def prepare_program(program: str | None, workspace: Path, run_dir: Path):
    """Plan source compilation without changing the selected source or its directory."""
    if not program or Path(program).suffix.lower() != ".sol":
        return program, []
    compiler = workspace / "tools" / "solc" / "solc.py"
    assembler = workspace / "tools" / "asm" / "asm.py"
    for tool in (compiler, assembler):
        if not tool.is_file():
            raise ConfigError(f"solのビルドツールが見つかりません: {tool}")
    assembly = run_dir / "program.asm"
    binary = run_dir / "program.bin"
    steps = [
        ("solをコンパイルしています…", [sys.executable, str(compiler), "compile", program, "-o", str(assembly)]),
        ("アセンブルしています…", [sys.executable, str(assembler), str(assembly), "-o", str(binary)]),
    ]
    return str(binary), steps


class Launcher(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("CPT32 ランチャー")
        self.geometry("960x820")
        self.minsize(820, 680)
        self.result_queue: queue.Queue = queue.Queue()
        self.process: subprocess.Popen | None = None
        self.running = False
        self.stop_requested = threading.Event()
        self.process_lock = threading.Lock()
        self.run_directory: Path | None = None
        self.document: dict = {}
        self.config_source = tk.StringVar(value=str(REPO / "config.yaml"))
        self.program = tk.StringVar()
        self.cpu = tk.StringVar(value="pipeline")
        self.run_mode = tk.StringVar(value="gui")
        self.cpu_execution_mode = tk.StringVar(value=DEFAULT_CPU_EXECUTION_MODE)
        self.cpu_time_slice_ms = tk.StringVar(value=str(DEFAULT_CPU_TIME_SLICE_MS))
        self.debug_gui = tk.BooleanVar(value=False)
        self.profile_cpu = tk.BooleanVar(value=False)
        self.start_paused = tk.BooleanVar(value=False)
        self.music_stream = tk.StringVar()
        self.bios = tk.StringVar()
        self.font = tk.StringVar()
        self.sound_root = tk.StringVar()
        self.sound_volume = tk.StringVar(value="1.0")
        self.fdd_profile = tk.StringVar()
        self.ram_mib = tk.StringVar(value="16")
        self.vram_mib = tk.StringVar(value="4")
        self.log_level = tk.StringVar(value="info")
        self.build_kind = tk.StringVar(value="release")
        self.workspace = tk.StringVar(value=str(REPO))
        self.executable = tk.StringVar()
        self.profile_name = tk.StringVar(value="default")
        self.drives = []
        self.cards = []
        self._build_ui()
        self.load_config()
        self._load_saved_state()
        self.after(100, self._poll_process)
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    def _build_ui(self):
        self.columnconfigure(0, weight=1)
        self.rowconfigure(2, weight=1)
        top = ttk.Frame(self, padding=(12, 10, 12, 5))
        top.grid(row=0, column=0, sticky="ew")
        top.columnconfigure(1, weight=1)
        ttk.Label(top, text="ROM / sol / カートリッジ").grid(row=0, column=0, sticky="w", padx=(0, 8), pady=4)
        ttk.Entry(top, textvariable=self.program).grid(row=0, column=1, sticky="ew", pady=4)
        ttk.Button(top, text="選択…", command=self._choose_program).grid(row=0, column=2, padx=(6, 0))
        ttk.Label(top, text="設定 YAML").grid(row=1, column=0, sticky="w", padx=(0, 8), pady=4)
        ttk.Entry(top, textvariable=self.config_source).grid(row=1, column=1, sticky="ew", pady=4)
        ttk.Button(top, text="選択…", command=self._choose_config).grid(row=1, column=2, padx=(6, 0))
        actions = ttk.Frame(self, padding=(12, 3))
        actions.grid(row=1, column=0, sticky="ew")
        actions.columnconfigure(5, weight=1)
        ttk.Label(actions, text="CPU").grid(row=0, column=0, padx=(0, 5))
        self.cpu_box = ttk.Combobox(actions, textvariable=self.cpu, values=CPU_KINDS, state="readonly", width=16)
        self.cpu_box.grid(row=0, column=1, padx=(0, 15))
        ttk.Label(actions, text="起動").grid(row=0, column=2, padx=(0, 5))
        ttk.Combobox(actions, textvariable=self.run_mode, values=("gui", "monitor"), state="readonly", width=12).grid(row=0, column=3, padx=(0, 12))
        ttk.Label(actions, text="プロファイル").grid(row=0, column=4, padx=(0, 5))
        self.profile_box = ttk.Combobox(actions, textvariable=self.profile_name, width=18)
        self.profile_box.grid(row=0, column=5, sticky="w")
        ttk.Button(actions, text="読込", command=self.load_profile).grid(row=0, column=6, padx=(4, 0))
        self.tabs = ttk.Notebook(self)
        self.tabs.grid(row=2, column=0, sticky="nsew", padx=12, pady=6)
        general = ttk.Frame(self.tabs, padding=12)
        general.columnconfigure(1, weight=1)
        self.tabs.add(general, text="起動設定")
        self._path_row(general, 0, "音楽ストリーム", self.music_stream, self._choose_music)
        self._path_row(general, 1, "Cargo workspace", self.workspace, self._choose_workspace, directory=True)
        self._path_row(general, 2, "既存エミュレーター", self.executable, self._choose_executable)
        ttk.Label(general, text="ビルド").grid(row=3, column=0, sticky="w", pady=6)
        build_row = ttk.Frame(general)
        build_row.grid(row=3, column=1, sticky="w")
        ttk.Radiobutton(build_row, text="release", value="release", variable=self.build_kind).pack(side="left")
        ttk.Label(general, text="CPU実行").grid(row=4, column=0, sticky="w", pady=6)
        execution_row = ttk.Frame(general)
        execution_row.grid(row=4, column=1, sticky="w")
        ttk.Combobox(execution_row, textvariable=self.cpu_execution_mode, values=CPU_EXECUTION_MODES, state="readonly", width=20).pack(side="left")
        ttk.Label(execution_row, text="時間枠 (ms)").pack(side="left", padx=(12, 5))
        ttk.Spinbox(execution_row, textvariable=self.cpu_time_slice_ms, from_=1, to=16, width=5).pack(side="left")
        checks = ttk.Frame(general)
        checks.grid(row=5, column=0, columnspan=2, sticky="w", pady=(10, 0))
        ttk.Checkbutton(checks, text="デバッグGUI", variable=self.debug_gui).pack(side="left", padx=(0, 12))
        ttk.Checkbutton(checks, text="CPUプロファイル", variable=self.profile_cpu, command=self._profile_changed).pack(side="left", padx=(0, 12))
        ttk.Checkbutton(checks, text="一時停止で開始", variable=self.start_paused).pack(side="left")
        ttk.Label(general, text="ログレベル").grid(row=6, column=0, sticky="w", pady=(14, 5))
        ttk.Combobox(general, textvariable=self.log_level, values=("trace", "debug", "info", "warn", "error", "off"), state="readonly", width=12).grid(row=6, column=1, sticky="w", pady=(14, 5))
        ttk.Label(general, text="ROMは .bin / .wax / .cpt32pkg / .sol（自動コンパイル）。monitor起動ではROMを省略できます。", foreground="#555").grid(row=7, column=0, columnspan=2, sticky="w", pady=(16, 4))
        hardware = ttk.Frame(self.tabs, padding=12)
        hardware.columnconfigure(1, weight=1)
        self.tabs.add(hardware, text="ROM・ディスク・拡張")
        self._path_row(hardware, 0, "BIOS ROM", self.bios, lambda: self._choose_into(self.bios, "BIOS / WAX ROM", "*.rom *.bin *.wax"))
        self._path_row(hardware, 1, "PCG font", self.font, lambda: self._choose_into(self.font, "PCG font", "*.bin *.f08 *.chr"))
        self._path_row(hardware, 2, "Disk sound assets", self.sound_root, lambda: self._choose_into(self.sound_root, "86Box assets", directory=True))
        ttk.Label(hardware, text="サウンド音量").grid(row=3, column=0, sticky="w", pady=5)
        sound = ttk.Frame(hardware)
        sound.grid(row=3, column=1, sticky="w")
        ttk.Entry(sound, textvariable=self.sound_volume, width=8).pack(side="left")
        ttk.Label(sound, text=" FDD profile").pack(side="left")
        ttk.Entry(sound, textvariable=self.fdd_profile, width=24).pack(side="left", padx=(4, 10))
        ttk.Label(sound, text="HDD profile").pack(side="left")
        ttk.Entry(sound, textvariable=self.hdd_profile, width=24).pack(side="left", padx=4)
        ttk.Separator(hardware).grid(row=4, column=0, columnspan=3, sticky="ew", pady=9)
        ttk.Label(hardware, text="ディスクイメージ").grid(row=5, column=0, columnspan=3, sticky="w")
        for slot in range(4):
            self._drive_row(hardware, slot, 6 + slot)
        advanced = ttk.Frame(self.tabs, padding=12)
        advanced.columnconfigure(1, weight=1)
        self.tabs.add(advanced, text="詳細設定")
        ttk.Label(advanced, text="メインRAM (MiB)").grid(row=0, column=0, sticky="w", pady=6)
        ttk.Spinbox(advanced, textvariable=self.ram_mib, from_=1, to=256, width=8).grid(row=0, column=1, sticky="w")
        ttk.Label(advanced, text="VRAM (MiB)").grid(row=1, column=0, sticky="w", pady=6)
        ttk.Spinbox(advanced, textvariable=self.vram_mib, from_=4, to=256, width=8).grid(row=1, column=1, sticky="w")
        ttk.Label(advanced, text="RAM: 1〜256 MiB、VRAM: 4〜256 MiB。既定値は16 MiB / 4 MiBです。", foreground="#555").grid(row=2, column=0, columnspan=2, sticky="w", pady=(8, 0))
        cards_tab = ttk.Frame(self.tabs, padding=12)
        cards_tab.columnconfigure(1, weight=1)
        self.tabs.add(cards_tab, text="WASM拡張カード")
        ttk.Label(cards_tab, text="カードパッケージをスロット0〜7に割り当てます").grid(row=0, column=0, columnspan=3, sticky="w", pady=(0, 8))
        for slot in range(8):
            self._card_row(cards_tab, slot, 1 + slot)
        footer = ttk.Frame(self, padding=(12, 5))
        footer.grid(row=3, column=0, sticky="ew")
        footer.columnconfigure(0, weight=1)
        buttons = ttk.Frame(footer)
        buttons.grid(row=0, column=1, sticky="e")
        self.start_button = ttk.Button(buttons, text="この設定で起動", command=self.start_emulator)
        self.start_button.pack(side="left", padx=4)
        self.stop_button = ttk.Button(buttons, text="停止", command=self.stop_emulator, state="disabled")
        self.stop_button.pack(side="left", padx=4)
        ttk.Button(buttons, text="設定を保存", command=self.save_config).pack(side="left", padx=4)
        ttk.Button(buttons, text="プロファイル保存", command=self.save_profile).pack(side="left", padx=4)
        self.status = tk.StringVar(value="起動待ち")
        ttk.Label(footer, textvariable=self.status).grid(row=0, column=0, sticky="w")
        log_frame = ttk.LabelFrame(self, text="起動ログ", padding=6)
        log_frame.grid(row=4, column=0, sticky="ew", padx=12, pady=(0, 12))
        log_frame.columnconfigure(0, weight=1)
        log_frame.rowconfigure(0, weight=1)
        self.log = tk.Text(log_frame, height=7, wrap="word", state="disabled", font=("Consolas", 9))
        scrollbar = ttk.Scrollbar(log_frame, orient="vertical", command=self.log.yview)
        self.log.configure(yscrollcommand=scrollbar.set)
        self.log.grid(row=0, column=0, sticky="nsew")
        scrollbar.grid(row=0, column=1, sticky="ns")

    def _path_row(self, parent, row, label, variable, command, directory=False):
        ttk.Label(parent, text=label).grid(row=row, column=0, sticky="w", padx=(0, 8), pady=5)
        ttk.Entry(parent, textvariable=variable).grid(row=row, column=1, sticky="ew", pady=5)
        ttk.Button(parent, text="選択…", command=command).grid(row=row, column=2, padx=(6, 0))

    def _choose_into(self, variable, title, pattern="*", directory=False):
        if directory:
            path = filedialog.askdirectory(title=title)
        else:
            path = filedialog.askopenfilename(title=title, filetypes=[(title, pattern), ("すべて", "*.*")])
        if path:
            variable.set(path)

    def _choose_program(self):
        self._choose_into(self.program, "CPT32 ROM / sol / cartridge", "*.bin *.wax *.cpt32pkg *.sol")

    def _choose_config(self):
        self._choose_into(self.config_source, "設定 YAML", "*.yaml *.yml")
        if self.config_source.get():
            self.load_config()

    def _choose_music(self):
        self._choose_into(self.music_stream, "Music stream", "*.sgub")

    def _choose_workspace(self):
        self._choose_into(self.workspace, "Cargo workspace", directory=True)

    def _choose_executable(self):
        self._choose_into(self.executable, "CPT32 executable", "*.exe")

    def _drive_row(self, parent, slot, row):
        enabled = tk.BooleanVar(value=False)
        kind = tk.StringVar(value="fdd")
        read_only = tk.BooleanVar(value=False)
        image = tk.StringVar()
        rate = tk.StringVar()
        latency = tk.StringVar()
        policy = tk.StringVar(value="reject")
        ttk.Checkbutton(parent, text=f"{slot}", variable=enabled, width=3).grid(row=row, column=0, sticky="w")
        ttk.Entry(parent, textvariable=image).grid(row=row, column=1, sticky="ew", pady=2)
        ttk.Button(parent, text="選択…", command=lambda v=image, s=slot: self._choose_into(v, f"Drive {s} image", "*.img *.ima *.vhd *.bin")).grid(row=row, column=2, padx=3)
        ttk.Combobox(parent, textvariable=kind, values=("fdd", "hdd"), state="readonly", width=5).grid(row=row, column=3, padx=3)
        ttk.Checkbutton(parent, text="読込専用", variable=read_only).grid(row=row, column=4, padx=3)
        ttk.Label(parent, text="B/s").grid(row=row, column=5, padx=(6, 2))
        ttk.Entry(parent, textvariable=rate, width=10).grid(row=row, column=6)
        ttk.Label(parent, text="ms").grid(row=row, column=7, padx=(6, 2))
        ttk.Entry(parent, textvariable=latency, width=6).grid(row=row, column=8)
        ttk.Combobox(parent, textvariable=policy, values=("reject", "pad", "truncate"), state="readonly", width=9).grid(row=row, column=9, padx=(6, 0))
        self.drives.append(dict(slot=slot, enabled=enabled, kind=kind, read_only=read_only, image=image, rate=rate, latency=latency, policy=policy))

    def _card_row(self, parent, slot, row):
        enabled = tk.BooleanVar(value=False)
        package = tk.StringVar()
        ttk.Checkbutton(parent, text=f"{slot}", variable=enabled, width=3).grid(row=row, column=0, sticky="w")
        ttk.Entry(parent, textvariable=package).grid(row=row, column=1, sticky="ew", pady=2)
        ttk.Button(parent, text="選択…", command=lambda v=package, s=slot: self._choose_into(v, f"WASM package {s}", "*.cpt32card")).grid(row=row, column=2, padx=3)
        self.cards.append(dict(slot=slot, enabled=enabled, package=package))

    def _profile_changed(self):
        if self.profile_cpu.get():
            self.debug_gui.set(True)

    def _log(self, text):
        self.log.configure(state="normal")
        self.log.insert("end", text.rstrip() + "\n")
        self.log.see("end")
        self.log.configure(state="disabled")

    def load_config(self):
        path = Path(self.config_source.get()).expanduser()
        try:
            self.document = parse_yaml(path.read_text(encoding="utf-8-sig")) if path.is_file() else {}
            if not isinstance(self.document, dict):
                raise ConfigError("YAMLの最上位はmappingにしてください")
            cpu = self.document.get("cpu", {}) or {}
            self.cpu.set(cpu.get("microarchitecture", "pipeline") if isinstance(cpu, dict) else "pipeline")
            bios = self.document.get("bios", {}) or {}
            pcg = self.document.get("pcg", {}) or {}
            sound = self.document.get("disk_sound", {}) or {}
            logging = self.document.get("logging", {}) or {}
            memory = self.document.get("memory", {}) or {}
            self.ram_mib.set(str(memory.get("ram_mib", 16)) if isinstance(memory, dict) else "16")
            self.vram_mib.set(str(memory.get("vram_mib", 4)) if isinstance(memory, dict) else "4")
            self.bios.set(bios.get("image", "") if isinstance(bios, dict) else "")
            self.font.set(pcg.get("font", "") if isinstance(pcg, dict) else "")
            self.sound_root.set(sound.get("asset_root", "") if isinstance(sound, dict) else "")
            self.sound_volume.set(str(sound.get("volume", 1.0)) if isinstance(sound, dict) else "1.0")
            self.fdd_profile.set(sound.get("default_fdd_profile", "") if isinstance(sound, dict) else "")
            self.hdd_profile.set(sound.get("default_hdd_profile", "") if isinstance(sound, dict) else "")
            drives = self.document.get("disk", []) or []
            by_slot = {item.get("slot"): item for item in drives if isinstance(item, dict) and isinstance(item.get("slot"), int)} if isinstance(drives, list) else {}
            for row in self.drives:
                item = by_slot.get(row["slot"], {})
                row["enabled"].set(bool(item))
                row["kind"].set(item.get("type", "fdd"))
                row["read_only"].set(bool(item.get("read_only", False)))
                row["image"].set(item.get("image", ""))
                row["rate"].set(str(item.get("transfer_rate_bytes_per_second", "")))
                row["latency"].set(str(item.get("access_latency_ms", "")))
                row["policy"].set(item.get("incomplete_sector_policy", "reject"))
            cards = self.document.get("expansion_card", []) or []
            by_slot = {item.get("slot"): item for item in cards if isinstance(item, dict) and isinstance(item.get("slot"), int)} if isinstance(cards, list) else {}
            for row in self.cards:
                item = by_slot.get(row["slot"], {})
                row["enabled"].set(bool(item))
                row["package"].set(item.get("package", ""))
            self.status.set(f"設定読み込み: {path}")
        except (OSError, ConfigError, AttributeError) as error:
            messagebox.showerror("設定を読み込めません", str(error), parent=self)

    def _effective_document(self):
        document = copy.deepcopy(self.document)
        cpu = document.get("cpu") or {}
        if not isinstance(cpu, dict):
            raise ConfigError("`cpu` はmappingで指定してください")
        memory = document.get("memory") or {}
        if not isinstance(memory, dict):
            raise ConfigError("`memory` はmappingで指定してください")
        for key, control, minimum in (("ram_mib", self.ram_mib, 1), ("vram_mib", self.vram_mib, 4)):
            raw = control.get().strip()
            if not raw.isdecimal() or not minimum <= int(raw) <= 256:
                raise ConfigError(f"`memory.{key}` は{minimum}〜256 MiBの整数で入力してください")
            value = int(raw)
            if value == (16 if key == "ram_mib" else 4):
                memory.pop(key, None)
            else:
                memory[key] = value
        if memory or "memory" in document:
            document["memory"] = memory
        else:
            document.pop("memory", None)
        for section, key, value in (("bios", "image", self.bios.get()), ("pcg", "font", self.font.get())):
            if value.strip():
                nested = document.get(section) or {}
                if not isinstance(nested, dict):
                    raise ConfigError(f"`{section}` はmappingで指定してください")
                nested[key] = value.strip()
                document[section] = nested
            elif isinstance(document.get(section), dict):
                document[section].pop(key, None)
                if not document[section]:
                    document.pop(section)
        disks = document.get("disk", []) or []
        if not isinstance(disks, list):
            raise ConfigError("`disk` はsequenceで指定してください")
        by_slot = {item.get("slot"): item for item in disks if isinstance(item, dict)}
        new_disks = []
        for row in self.drives:
            if not row["enabled"].get():
                continue
            image = row["image"].get().strip()
            if not image:
                raise ConfigError(f"ディスクスロット{row['slot']}のイメージを選択してください")
            path = Path(image).expanduser()
            if not path.is_file():
                raise ConfigError(f"ディスクイメージが見つかりません: {path}")
            entry = dict(by_slot.get(row["slot"], {}))
            entry.update(slot=row["slot"], type=row["kind"].get(), image=str(path.resolve()), read_only=row["read_only"].get())
            for field, control, minimum in (("transfer_rate_bytes_per_second", row["rate"], 1), ("access_latency_ms", row["latency"], 0)):
                raw = control.get().strip()
                if raw:
                    if not raw.isdecimal() or int(raw) < minimum:
                        raise ConfigError(f"ディスクスロット{row['slot']}の{field}は{minimum}以上の整数で入力してください")
                    entry[field] = int(raw)
                else:
                    entry.pop(field, None)
            entry["incomplete_sector_policy"] = row["policy"].get()
            new_disks.append(entry)
        if new_disks:
            document["disk"] = new_disks
        else:
            document.pop("disk", None)
        sound = document.get("disk_sound") or {}
        if not isinstance(sound, dict):
            raise ConfigError("`disk_sound` はmappingで指定してください")
        for key, value in (("asset_root", self.sound_root.get().strip()), ("default_fdd_profile", self.fdd_profile.get().strip()), ("default_hdd_profile", self.hdd_profile.get().strip())):
            if value:
                sound[key] = value
            else:
                sound.pop(key, None)
        try:
            volume = float(self.sound_volume.get())
        except ValueError as error:
            raise ConfigError("サウンド音量は0〜4の数値を入力してください") from error
        if not 0 <= volume <= 4:
            raise ConfigError("サウンド音量は0〜4の範囲にしてください")
        if sound or "disk_sound" in document:
            sound["volume"] = volume
            document["disk_sound"] = sound
        entries = document.get("expansion_card", []) or []
        if not isinstance(entries, list):
            raise ConfigError("`expansion_card` はsequenceで指定してください")
        existing = {item.get("slot"): item for item in entries if isinstance(item, dict)}
        cards = []
        for row in self.cards:
            if row["enabled"].get():
                path = row["package"].get().strip()
                if not path:
                    raise ConfigError(f"拡張カードスロット{row['slot']}のpackageを選択してください")
                if not Path(path).expanduser().is_file():
                    raise ConfigError(f"拡張カードpackageが見つかりません: {path}")
                item = dict(existing.get(row["slot"], {}))
                item.update(slot=row["slot"], package=str(Path(path).expanduser().resolve()))
                cards.append(item)
        if cards:
            document["expansion_card"] = cards
        else:
            document.pop("expansion_card", None)
        logging = document.get("logging") or {}
        if not isinstance(logging, dict):
            raise ConfigError("`logging` はmappingで指定してください")
        if self.log_level.get() == "info":
            logging.pop("level", None)
        else:
            logging["level"] = self.log_level.get()
        if logging:
            document["logging"] = logging
        else:
            document.pop("logging", None)
        return document

    def save_config(self):
        path = Path(self.config_source.get()).expanduser()
        try:
            document = self._effective_document()
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(dump_yaml(document), encoding="utf-8")
            self.document = document
            self.status.set(f"設定を保存しました: {path}")
        except (OSError, ConfigError) as error:
            messagebox.showerror("設定を保存できません", str(error), parent=self)

    def _collect_profile(self):
        return {
            "config_source": self.config_source.get(), "config_yaml": dump_yaml(self._effective_document()),
            "program": self.program.get(), "cpu": self.cpu.get(), "run_mode": self.run_mode.get(),
            "debug_gui": self.debug_gui.get(), "profile_cpu": self.profile_cpu.get(), "start_paused": self.start_paused.get(),
            "cpu_execution_mode": self.cpu_execution_mode.get(), "cpu_time_slice_ms": self.cpu_time_slice_ms.get(),
            "music_stream": self.music_stream.get(), "build_kind": self.build_kind.get(), "log_level": self.log_level.get(),
            "workspace": self.workspace.get(), "executable": self.executable.get(),
        }

    def _apply_profile(self, profile):
        self.config_source.set(profile.get("config_source", self.config_source.get()))
        self.document = parse_yaml(profile.get("config_yaml", "{}"))
        self.program.set(profile.get("program", ""))
        self.cpu.set(profile.get("cpu", "pipeline"))
        self.run_mode.set(profile.get("run_mode", "gui"))
        self.debug_gui.set(profile.get("debug_gui", False))
        self.profile_cpu.set(profile.get("profile_cpu", False))
        self.start_paused.set(profile.get("start_paused", False))
        self.cpu_execution_mode.set(profile.get("cpu_execution_mode", DEFAULT_CPU_EXECUTION_MODE))
        self.cpu_time_slice_ms.set(str(profile.get("cpu_time_slice_ms", DEFAULT_CPU_TIME_SLICE_MS)))
        self.build_kind.set(profile.get("build_kind", "release"))
        self.workspace.set(profile.get("workspace", str(REPO)))
        self.executable.set(profile.get("executable", ""))
        self.log_level.set(profile.get("log_level", "info"))
        # Reload hardware fields from the profile's saved YAML.
        temporary = self.config_source.get()
        self.config_source.set("(profile)")
        self._populate_from_document()
        self.config_source.set(temporary)

    def _populate_from_document(self):
        cpu = self.document.get("cpu", {}) or {}
        self.cpu.set(cpu.get("microarchitecture", self.cpu.get()) if isinstance(cpu, dict) else self.cpu.get())
        memory = self.document.get("memory", {}) or {}
        self.ram_mib.set(str(memory.get("ram_mib", 16)) if isinstance(memory, dict) else "16")
        self.vram_mib.set(str(memory.get("vram_mib", 4)) if isinstance(memory, dict) else "4")
        bios, pcg, sound = (self.document.get(k, {}) or {} for k in ("bios", "pcg", "disk_sound"))
        self.bios.set(bios.get("image", "") if isinstance(bios, dict) else "")
        self.font.set(pcg.get("font", "") if isinstance(pcg, dict) else "")
        self.sound_root.set(sound.get("asset_root", "") if isinstance(sound, dict) else "")
        self.sound_volume.set(str(sound.get("volume", 1.0)) if isinstance(sound, dict) else "1.0")
        self.fdd_profile.set(sound.get("default_fdd_profile", "") if isinstance(sound, dict) else "")
        self.hdd_profile.set(sound.get("default_hdd_profile", "") if isinstance(sound, dict) else "")
        logging = self.document.get("logging", {}) or {}
        self.log_level.set(logging.get("level", "info") if isinstance(logging, dict) else "info")
        disks = self.document.get("disk", []) or []
        by_slot = {item.get("slot"): item for item in disks if isinstance(item, dict)} if isinstance(disks, list) else {}
        for row in self.drives:
            item = by_slot.get(row["slot"], {})
            row["enabled"].set(bool(item))
            row["kind"].set(item.get("type", "fdd"))
            row["read_only"].set(item.get("read_only", False))
            row["image"].set(item.get("image", ""))
            row["rate"].set(str(item.get("transfer_rate_bytes_per_second", "")))
            row["latency"].set(str(item.get("access_latency_ms", "")))
            row["policy"].set(item.get("incomplete_sector_policy", "reject"))
        entries = self.document.get("expansion_card", []) or []
        by_slot = {item.get("slot"): item for item in entries if isinstance(item, dict)} if isinstance(entries, list) else {}
        for row in self.cards:
            item = by_slot.get(row["slot"], {})
            row["enabled"].set(bool(item))
            row["package"].set(item.get("package", ""))

    def _load_saved_state(self):
        try:
            data = json.loads(PROFILE_FILE.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return
        if isinstance(data, dict):
            self.profile_box.configure(values=sorted(data.get("profiles", {})))
            for name, profile in data.get("profiles", {}).items():
                if name == "default" and isinstance(profile, dict):
                    try:
                        self._apply_profile(profile)
                    except (ConfigError, TypeError):
                        pass

    def save_profile(self):
        name = self.profile_name.get().strip()
        if not name:
            messagebox.showerror("プロファイル", "名前を入力してください", parent=self)
            return
        try:
            profile = self._collect_profile()
            data = {"profiles": {}}
            if PROFILE_FILE.is_file():
                data = json.loads(PROFILE_FILE.read_text(encoding="utf-8"))
            data.setdefault("profiles", {})[name] = profile
            PROFILE_FILE.parent.mkdir(parents=True, exist_ok=True)
            PROFILE_FILE.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
            self.profile_box.configure(values=sorted(data["profiles"]))
            self.status.set(f"プロファイルを保存しました: {name}")
        except (OSError, ConfigError, json.JSONDecodeError) as error:
            messagebox.showerror("プロファイルを保存できません", str(error), parent=self)

    def load_profile(self):
        name = self.profile_name.get().strip()
        try:
            data = json.loads(PROFILE_FILE.read_text(encoding="utf-8"))
            profile = data.get("profiles", {}).get(name)
            if not isinstance(profile, dict):
                raise ConfigError(f"プロファイルがありません: {name}")
            self._apply_profile(profile)
            self.status.set(f"プロファイルを読み込みました: {name}")
        except (OSError, json.JSONDecodeError, ConfigError, TypeError) as error:
            messagebox.showerror("プロファイルを読み込めません", str(error), parent=self)

    def _launch_command(self, run_dir: Path, program: str | None):
        binary = self.executable.get().strip()
        if binary:
            exe = Path(binary).expanduser().resolve()
            if not exe.is_file():
                raise ConfigError(f"エミュレーターが見つかりません: {exe}")
            command = [str(exe)]
        else:
            workspace = Path(self.workspace.get()).expanduser().resolve()
            if not (workspace / "Cargo.toml").is_file():
                raise ConfigError(f"Cargo.toml がありません: {workspace}")
            command = ["cargo", "run", "--manifest-path", str(workspace / "Cargo.toml"), "--bin", "cpt32"]
            if self.build_kind.get() == "release":
                command.append("--release")
            if self.cpu.get() == "jit":
                command += ["--features", "jit"]
            command.append("--")
        if self.run_mode.get() == "monitor":
            command.append("--monitor")
            if program:
                command.append(program)
        else:
            if not program:
                raise ConfigError("GUI起動にはROMまたはカートリッジを指定してください")
            mode = self.cpu_execution_mode.get()
            if mode not in CPU_EXECUTION_MODES:
                raise ConfigError(f"CPU実行モードが無効です: {mode}")
            try:
                time_slice_ms = int(self.cpu_time_slice_ms.get())
            except ValueError as error:
                raise ConfigError("CPU時間枠は1〜16 msで指定してください") from error
            if time_slice_ms not in CPU_TIME_SLICE_RANGE_MS:
                raise ConfigError("CPU時間枠は1〜16 msで指定してください")
            command.append(program)
            command += ["--cpu-execution", mode, "--cpu-time-slice-ms", str(time_slice_ms)]
            if self.debug_gui.get() or self.profile_cpu.get():
                command.append("--debug-gui")
            if self.profile_cpu.get():
                command.append("--profile-cpu")
            if self.start_paused.get():
                if not self.debug_gui.get() and not self.profile_cpu.get():
                    raise ConfigError("一時停止開始にはデバッグGUIを有効にしてください")
                command.append("--start-paused")
            if self.music_stream.get().strip():
                stream = Path(self.music_stream.get()).expanduser()
                if not stream.is_file():
                    raise ConfigError(f"music streamが見つかりません: {stream}")
                command += ["--music-stream", str(stream.resolve())]
        command += ["--cpu", self.cpu.get()]
        return command

    def start_emulator(self):
        if self.running:
            messagebox.showinfo("起動中", "エミュレーターは既に起動しています", parent=self)
            return
        try:
            source = Path(self.config_source.get()).expanduser()
            document = self._effective_document()
            source_base = source.parent.resolve() if source.parent else REPO
            staged = prepare_run_config(document, source_base, self.cpu.get())
            program = self.program.get().strip()
            program_path = str(Path(program).expanduser().resolve()) if program else None
            if self.run_mode.get() == "gui" and (not program_path or not Path(program_path).is_file()):
                raise ConfigError(f"ROMが見つかりません: {program or '(未指定)'}")
            if program_path and not Path(program_path).is_file():
                raise ConfigError(f"ROMが見つかりません: {program_path}")
            self.run_directory = Path(tempfile.mkdtemp(prefix="cpt32-launch-"))
            (self.run_directory / "config.yaml").write_text(dump_yaml(staged), encoding="utf-8")
            program_path, steps = prepare_program(
                program_path, Path(self.workspace.get()).expanduser().resolve(), self.run_directory,
            )
            command = self._launch_command(self.run_directory, program_path)
            steps.append(("エミュレーターを起動しています…", command))
            self.status.set(steps[0][0])
            self.start_button.configure(state="disabled")
            self.stop_button.configure(state="normal")
            self.stop_requested.clear()
            self.running = True
            self.worker = threading.Thread(target=self._spawn_and_read, args=(steps, self.run_directory), daemon=True)
            self.worker.start()
        except (OSError, ConfigError) as error:
            if self.run_directory and self.process is None:
                shutil.rmtree(self.run_directory, ignore_errors=True)
                self.run_directory = None
            messagebox.showerror("起動できません", str(error), parent=self)

    def _spawn_and_read(self, steps, cwd):
        try:
            code = 0
            for index, (label, command) in enumerate(steps):
                # Serialize stop with process creation, including between build stages.
                with self.process_lock:
                    if self.stop_requested.is_set():
                        break
                    self.process = subprocess.Popen(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace", bufsize=1)
                    process = self.process
                self.result_queue.put(("line", "$ " + subprocess.list2cmdline(command)))
                self.result_queue.put(("phase", label))
                if index == len(steps) - 1:
                    self.result_queue.put(("started", process.pid))
                with process.stdout:
                    for line in process.stdout:
                        self.result_queue.put(("line", line.rstrip()))
                code = process.wait()
                if self.stop_requested.is_set():
                    break
                if code != 0 and index < len(steps) - 1:
                    self.result_queue.put(("error", f"{label} 失敗 (終了コード {code})。ログを確認してください。"))
                    return
            self.result_queue.put(("exited", code))
        except (OSError, subprocess.SubprocessError) as error:
            self.result_queue.put(("error", str(error)))

    def _poll_process(self):
        try:
            while True:
                kind, value = self.result_queue.get_nowait()
                if kind == "line":
                    self._log(value)
                elif kind == "phase":
                    self.status.set(value)
                elif kind == "started":
                    self.status.set(f"起動中 (PID {value})")
                elif kind == "exited":
                    self.status.set(f"終了コード {value}")
                    self._finish_process()
                elif kind == "error":
                    self._log("起動エラー: " + value)
                    self.status.set("起動に失敗しました")
                    self._finish_process()
        except queue.Empty:
            pass
        self.after(100, self._poll_process)

    def _finish_process(self):
        self.process = None
        self.running = False
        self.start_button.configure(state="normal")
        self.stop_button.configure(state="disabled")
        if self.run_directory:
            shutil.rmtree(self.run_directory, ignore_errors=True)
            self.run_directory = None

    def stop_emulator(self):
        with self.process_lock:
            self.stop_requested.set()
            process = self.process
        if process is not None and process.poll() is None:
            if os.name == "nt":
                subprocess.run(
                    ["taskkill", "/T", "/F", "/PID", str(process.pid)],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    check=False,
                )
            else:
                process.terminate()
            self.status.set("停止しています…")

    def _on_close(self):
        if self.running:
            if not messagebox.askyesno("終了", "起動中のエミュレーターを停止してランチャーを閉じますか？", parent=self):
                return
            self.stop_emulator()
            if self.run_directory:
                run_directory = self.run_directory
                worker = self.worker
                threading.Thread(target=lambda: (worker.join(), shutil.rmtree(run_directory, ignore_errors=True)), daemon=True).start()
        try:
            PROFILE_FILE.parent.mkdir(parents=True, exist_ok=True)
            data = json.loads(PROFILE_FILE.read_text(encoding="utf-8")) if PROFILE_FILE.is_file() else {"profiles": {}}
            data.setdefault("profiles", {})["default"] = self._collect_profile()
            PROFILE_FILE.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
        except (OSError, ConfigError, json.JSONDecodeError):
            pass
        self.destroy()


def main():
    try:
        app = Launcher()
        app.mainloop()
    except tk.TclError as error:
        print(f"CPT32 launcher could not open its GUI: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
