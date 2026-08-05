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
