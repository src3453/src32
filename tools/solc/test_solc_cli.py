from solc import main


def test_run_command_supports_trace(capsys, tmp_path):
    source_path = tmp_path / "sample.sol"
    source_path.write_text("1 2 add", encoding="utf-8")

    exit_code = main(["run", "--trace", str(source_path)])

    captured = capsys.readouterr()
    assert exit_code == 0
    assert "trace:" in captured.out
    assert "op=push" in captured.out
    assert "op=add" in captured.out


def test_compile_command_can_keep_unused_functions(capsys, tmp_path):
    source_path = tmp_path / "sample.sol"
    source_path.write_text("fn unused () : 1 ret ;\n", encoding="utf-8")

    exit_code = main(["compile", "--keep-unused-functions", str(source_path)])

    captured = capsys.readouterr()
    assert exit_code == 0
    assert "unused:" in captured.out


def test_compile_command_warns_about_unused_functions(capsys, caplog, tmp_path):
    source_path = tmp_path / "sample.sol"
    source_path.write_text("fn unused () : 1 ret ;\n", encoding="utf-8")

    exit_code = main(["compile", str(source_path)])

    captured = capsys.readouterr()
    assert exit_code == 0
    assert "unused function: unused" in caplog.text
