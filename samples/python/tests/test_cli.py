"""The command line, which is argument parsing and nothing else."""

from __future__ import annotations

from taskqueue.cli import main


def test_a_task_that_always_succeeds_exits_zero(capsys) -> None:
    exit_code = main(["resize", "--backoff", "constant", "--delay-ms", "1"])

    assert exit_code == 0
    assert "resize" in capsys.readouterr().out


def test_a_task_that_runs_out_of_attempts_exits_non_zero(capsys) -> None:
    # `flaky` refuses the first two attempts of any task named "flaky", so
    # one attempt is never enough.
    exit_code = main(["flaky", "--max-attempts", "1", "--backoff", "constant"])

    assert exit_code == 1
