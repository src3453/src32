from sol_repl import run_repl
from sol_vm import SolVM


def test_repl_stack_and_quit():
    lines = iter(["1 2 add", ".stack", ".quit"])
    outputs: list[str] = []

    rc = run_repl(
        vm=SolVM(),
        input_fn=lambda _prompt: next(lines),
        output_fn=lambda message: outputs.append(message),
    )

    assert rc == 0
    assert any("stack: [3]" in line for line in outputs)


def test_repl_reports_error_and_continues():
    lines = iter(["add", "1", ".stack", ".quit"])
    outputs: list[str] = []

    rc = run_repl(
        vm=SolVM(),
        input_fn=lambda _prompt: next(lines),
        output_fn=lambda message: outputs.append(message),
    )

    assert rc == 0
    assert any(line.startswith("error: stack underflow") for line in outputs)
    assert any("stack: [1]" in line for line in outputs)
