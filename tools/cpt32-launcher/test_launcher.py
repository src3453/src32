"""Headless checks for the launcher's build/run pipeline."""

import importlib.util
from pathlib import Path
import queue
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest


spec = importlib.util.spec_from_file_location("cpt32_launcher", Path(__file__).with_name("launcher.py"))
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)


class LaunchPipelineTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="cpt32 launcher 日本語 ")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.run_dir = self.base / "run"
        self.run_dir.mkdir()
        self.runner = SimpleNamespace(
            result_queue=queue.Queue(), process=None,
            process_lock=threading.Lock(), stop_requested=threading.Event(),
        )

    def run_steps(self, steps):
        launcher.Launcher._spawn_and_read(self.runner, steps, self.run_dir)
        events = []
        while not self.runner.result_queue.empty():
            events.append(self.runner.result_queue.get_nowait())
        return events

    def test_sol_builds_with_relative_include_before_launch(self):
        source = self.base / "sample.SOL"
        source.write_text('!include "values.sol"\nVALUE\n', encoding="utf-8")
        (self.base / "values.sol").write_text("!const VALUE 42\n", encoding="utf-8")
        binary, steps = launcher.prepare_program(str(source), launcher.REPO, self.run_dir)
        steps.append(("emulator", [sys.executable, "-c", "from pathlib import Path; import sys; assert Path(sys.argv[1]).stat().st_size > 0; print('emulator launched')", binary]))
        events = self.run_steps(steps)
        self.assertIn(("exited", 0), events, events)
        self.assertIn(("line", "emulator launched"), events)
        self.assertEqual(len([event for event in events if event[0] == "started"]), 1)
        self.assertEqual(sorted(path.name for path in self.base.iterdir()), ["run", "sample.SOL", "values.sol"])
        self.assertTrue(Path(binary).is_file())

    def test_compile_error_prevents_assembly_and_launch(self):
        source = self.base / "invalid.sol"
        source.write_text("unknown_word", encoding="utf-8")
        binary, steps = launcher.prepare_program(str(source), launcher.REPO, self.run_dir)
        steps.append(("emulator", [sys.executable, "-c", "raise AssertionError('must not launch')"]))
        events = self.run_steps(steps)
        self.assertTrue(any(kind == "error" for kind, _ in events), events)
        self.assertTrue(any(kind == "line" and "unknown_word" in value for kind, value in events), events)
        self.assertFalse(any(kind == "started" for kind, _ in events))
        self.assertFalse(Path(binary).exists())

    def test_assembler_failure_prevents_launch(self):
        events = self.run_steps([
            ("compile", [sys.executable, "-c", "pass"]),
            ("assemble", [sys.executable, "-c", "import sys; sys.exit(7)"]),
            ("emulator", [sys.executable, "-c", "raise AssertionError('must not launch')"]),
        ])
        self.assertTrue(any(kind == "error" and "7" in value for kind, value in events), events)
        self.assertFalse(any(kind == "started" for kind, _ in events))

    def test_stop_between_stages_prevents_next_process(self):
        result_queue = self.runner.result_queue
        original_put = result_queue.put

        def stop_after_output(event):
            original_put(event)
            if event == ("line", "compiled"):
                self.runner.stop_requested.set()

        result_queue.put = stop_after_output
        events = self.run_steps([
            ("compile", [sys.executable, "-c", "print('compiled')"]),
            ("emulator", [sys.executable, "-c", "raise AssertionError('must not launch')"]),
        ])
        self.assertIn(("exited", 0), events)
        self.assertFalse(any(kind == "started" for kind, _ in events))

    def test_stop_before_first_process(self):
        self.runner.stop_requested.set()
        events = self.run_steps([("emulator", ["missing-executable"])])
        self.assertEqual(events, [("exited", 0)])
        self.assertIsNone(self.runner.process)

    def test_missing_process_reports_error(self):
        events = self.run_steps([("compile", [str(self.base / "missing-executable")])])
        self.assertEqual(events[0][0], "error")

    def test_binary_and_empty_monitor_need_no_tools(self):
        for program in (None, str(self.base / "game.bin"), str(self.base / "game.wax"), str(self.base / "game.cpt32pkg")):
            with self.subTest(program=program):
                self.assertEqual(launcher.prepare_program(program, self.base, self.run_dir), (program, []))

    def test_missing_build_tools_fail_early(self):
        with self.assertRaises(launcher.ConfigError):
            launcher.prepare_program(str(self.base / "game.sol"), self.base, self.run_dir)

    def test_compiled_binary_is_used_by_cargo_and_existing_executable(self):
        source = str(self.base / "game.sol")
        binary, _ = launcher.prepare_program(source, launcher.REPO, self.run_dir)
        executable = self.base / "cpt32.exe"
        executable.touch()
        for mode in ("gui", "monitor"):
            for exe in ("", str(executable)):
                with self.subTest(mode=mode, exe=exe):
                    values = dict(executable=exe, workspace=str(launcher.REPO), build_kind="release", cpu="jit", run_mode=mode, debug_gui=False, profile_cpu=False, start_paused=False, music_stream="")
                    values.update(cpu_execution_mode="cycle-independent", cpu_time_slice_ms="12")
                    ui = SimpleNamespace(**{key: SimpleNamespace(get=lambda value=value: value) for key, value in values.items()})
                    command = launcher.Launcher._launch_command(ui, self.run_dir, binary)
                    self.assertIn(binary, command)
                    self.assertNotIn(source, command)
                    self.assertEqual("--monitor" in command, mode == "monitor")
                    if mode == "gui":
                        self.assertEqual(command[command.index("--cpu-execution") + 1], "cycle-independent")
                        self.assertEqual(command[command.index("--cpu-time-slice-ms") + 1], "12")
                    else:
                        self.assertNotIn("--cpu-execution", command)
                        self.assertNotIn("--cpu-time-slice-ms", command)
                    if not exe:
                        self.assertIn("--features", command)


    def test_cpu_time_slice_validation_rejects_out_of_range_and_non_numeric_values(self):
        executable = self.base / "cpt32.exe"
        executable.touch()
        for time_slice in ("0", "17", "invalid"):
            with self.subTest(time_slice=time_slice):
                values = dict(
                    executable=str(executable),
                    workspace=str(launcher.REPO),
                    build_kind="release",
                    cpu="pipeline",
                    run_mode="gui",
                    debug_gui=False,
                    profile_cpu=False,
                    start_paused=False,
                    music_stream="",
                    cpu_execution_mode="best-effort",
                    cpu_time_slice_ms=time_slice,
                )
                ui = SimpleNamespace(
                    **{
                        key: SimpleNamespace(get=lambda value=value: value)
                        for key, value in values.items()
                    }
                )
                with self.assertRaises(launcher.ConfigError):
                    launcher.Launcher._launch_command(ui, self.run_dir, str(self.base / "game.bin"))

    def test_execution_settings_round_trip_through_saved_profile(self):
        class Variable:
            def __init__(self, value):
                self.value = value

            def get(self):
                return self.value

            def set(self, value):
                self.value = value

        values = {
            "config_source": "config.yaml",
            "program": "game.bin",
            "cpu": "pipeline",
            "run_mode": "gui",
            "debug_gui": False,
            "profile_cpu": False,
            "start_paused": False,
            "music_stream": "",
            "build_kind": "release",
            "log_level": "info",
            "workspace": str(launcher.REPO),
            "executable": "",
            "cpu_execution_mode": "cycle-independent",
            "cpu_time_slice_ms": "12",
        }
        ui = SimpleNamespace(
            **{key: Variable(value) for key, value in values.items()},
            _effective_document=lambda: {},
            _populate_from_document=lambda: None,
        )
        profile = launcher.Launcher._collect_profile(ui)
        profile["config_yaml"] = "config: {}\n"
        self.assertEqual(profile["cpu_time_slice_ms"], "12")

        launcher.Launcher._apply_profile(ui, profile)
        self.assertEqual(ui.cpu_execution_mode.get(), "cycle-independent")
        self.assertEqual(ui.cpu_time_slice_ms.get(), "12")

        legacy_profile = dict(profile)
        legacy_profile.pop("cpu_execution_mode")
        legacy_profile.pop("cpu_time_slice_ms")
        launcher.Launcher._apply_profile(ui, legacy_profile)
        self.assertEqual(ui.cpu_execution_mode.get(), launcher.DEFAULT_CPU_EXECUTION_MODE)
        self.assertEqual(ui.cpu_time_slice_ms.get(), str(launcher.DEFAULT_CPU_TIME_SLICE_MS))

    def test_execution_policy_command_has_no_effect_on_monitor_mode(self):
        executable = self.base / "cpt32.exe"
        executable.touch()
        values = dict(
            executable=str(executable),
            workspace=str(launcher.REPO),
            build_kind="release",
            cpu="pipeline",
            run_mode="monitor",
            debug_gui=False,
            profile_cpu=False,
            start_paused=False,
            music_stream="",
            cpu_execution_mode="cycle-independent",
            cpu_time_slice_ms="8",
        )
        ui = SimpleNamespace(
            **{key: SimpleNamespace(get=lambda value=value: value) for key, value in values.items()}
        )

        command = launcher.Launcher._launch_command(ui, self.run_dir, None)

        self.assertIn("--monitor", command)
        self.assertNotIn("--cpu-execution", command)
        self.assertNotIn("--cpu-time-slice-ms", command)
if __name__ == "__main__":
    unittest.main()
