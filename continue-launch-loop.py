#!/usr/bin/env python3
"""Drive `xchandles continue-launch` through precommit batches, then register batches.

The CLI `--skip` sent to each run is a lookbehind of 16 batches so a reorg that
undid recent work is still discovered. The script only answers `yes` at
`Proceed?` when the printed handles match the CSV rows at the logical skip.

`slot-machine` prints `Error: ...` and still exits 0, so this script treats
`Confirmed!` / `Error:` in the output as the real success/failure signal.

Usage:
  ./continue-launch-loop.py cargo r xchandles continue-launch ... \\
      --premine FILE --handles-per-spend N [--skip START]
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

LOOKBEHIND_BATCHES = 16
PROMPT = "Proceed? (yes/no): "
ALREADY_DONE = "All handles have already been registered"
CONFIRMED = "Confirmed!"
ERROR_PREFIX = "Error: "
HANDLE_LINE = re.compile(
    r"^[ \t]+handle: ([^,]+), recipient: ([^,]+), expiration: (\d+)",
    re.MULTILINE,
)


@dataclass(frozen=True)
class HandleRow:
    handle: str
    recipient: str
    expiration: str

    def label(self) -> str:
        return (
            f"  handle: {self.handle}, recipient: {self.recipient}, "
            f"expiration: {self.expiration}"
        )


def opt_value(cmd: list[str], name: str) -> str | None:
    for i, arg in enumerate(cmd):
        if arg == name:
            if i + 1 >= len(cmd):
                raise SystemExit(f"error: {name} is missing a value")
            return cmd[i + 1]
        if arg.startswith(name + "="):
            return arg.split("=", 1)[1]
    return None


def set_opt(cmd: list[str], name: str, value: str) -> list[str]:
    out = list(cmd)
    for i, arg in enumerate(out):
        if arg == name:
            if i + 1 >= len(out):
                raise SystemExit(f"error: {name} is missing a value")
            out[i + 1] = value
            return out
        if arg.startswith(name + "="):
            out[i] = f"{name}={value}"
            return out
    out.extend([name, value])
    return out


def strip_flag(cmd: list[str], name: str) -> list[str]:
    out: list[str] = []
    for arg in cmd:
        if arg == name or arg.startswith(name + "="):
            continue
        out.append(arg)
    return out


def load_premine(path: Path) -> list[HandleRow]:
    rows: list[HandleRow] = []
    with path.open() as f:
        header = f.readline()
        if not header:
            raise SystemExit(f"error: empty premine csv: {path}")
        for line in f:
            line = line.strip()
            if not line:
                continue
            parts = line.split(",", 2)
            if len(parts) < 3:
                raise SystemExit(f"error: bad premine row: {line}")
            handle, recipient, rest = parts
            expiration = rest.split(",", 1)[0]
            rows.append(HandleRow(handle, recipient, expiration))
    if not rows:
        raise SystemExit(f"error: no handle rows in {path}")
    return rows


def parse_printed_handles(text: str) -> list[HandleRow]:
    return [
        HandleRow(m.group(1), m.group(2), m.group(3))
        for m in HANDLE_LINE.finditer(text)
    ]


def safe_skip_for(logical_skip: int, handles_per_spend: int) -> int:
    return max(0, logical_skip - LOOKBEHIND_BATCHES * handles_per_spend)


def cli_error_line(text: str) -> str | None:
    # Prompt has no trailing newline, so "Error: ..." can land on the same line.
    for line in text.splitlines():
        idx = line.find(ERROR_PREFIX)
        if idx != -1:
            return line[idx:]
    return None


def echo_bytes(data: bytes) -> None:
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()


def read_until_prompt_or_exit(proc: subprocess.Popen[bytes]) -> str:
    chunks: list[bytes] = []
    prompt = PROMPT.encode()
    while True:
        assert proc.stdout is not None
        chunk = proc.stdout.read(4096)
        if chunk:
            echo_bytes(chunk)
            chunks.append(chunk)
            if prompt in b"".join(chunks):
                break
            continue
        break
    return b"".join(chunks).decode("utf-8", "replace")


def drain(proc: subprocess.Popen[bytes]) -> str:
    assert proc.stdout is not None
    chunks: list[bytes] = []
    while True:
        chunk = proc.stdout.read(4096)
        if chunk:
            echo_bytes(chunk)
            chunks.append(chunk)
            continue
        break
    proc.wait()
    return b"".join(chunks).decode("utf-8", "replace")


def answer(proc: subprocess.Popen[bytes], word: str) -> None:
    assert proc.stdin is not None
    proc.stdin.write(f"{word}\n".encode())
    proc.stdin.flush()
    proc.stdin.close()


def csv_index(rows: list[HandleRow], handle: str) -> int | None:
    for i, row in enumerate(rows):
        if row.handle == handle:
            return i
    return None


def refuse_mismatch(
    rows: list[HandleRow],
    expected: list[HandleRow],
    printed: list[HandleRow],
    logical_skip: int,
) -> None:
    print()
    print(
        "ERROR: continue-launch printed a different handle list than "
        f"CSV skip={logical_skip}."
    )
    print(
        "A reorg likely dropped recent precommits or registrations; "
        "refusing to proceed."
    )
    print()
    print("Expected:")
    for row in expected:
        print(row.label())
    print()
    print("CLI printed:")
    if printed:
        for row in printed:
            print(row.label())
        first = csv_index(rows, printed[0].handle)
        if first is not None:
            print(
                f"CLI list starts at CSV index {first} "
                f"({printed[0].handle}); this run expected index "
                f"{logical_skip}."
            )
    else:
        print("  (no handle lines found before Proceed?)")
    raise SystemExit(1)


def run_one(
    cmd: list[str],
    rows: list[HandleRow],
    expected: list[HandleRow],
    logical_skip: int,
    safe_skip: int,
    pass_name: str,
    run: int,
    planned: int,
    env: dict[str, str] | None = None,
) -> str:
    launch_cmd = strip_flag(cmd, "--yes")
    launch_cmd = set_opt(launch_cmd, "--skip", str(safe_skip))

    print()
    print(
        f"=== {pass_name} run {run}/{planned}  skip={logical_skip}  "
        f"safe-skip={safe_skip}  handles={len(expected)} ==="
    )
    print("Command:", " ".join(launch_cmd), flush=True)
    print("Expected from CSV:")
    for row in expected:
        print(row.label())
    print(flush=True)

    proc = subprocess.Popen(
        launch_cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        bufsize=0,
        cwd=str(Path(__file__).resolve().parent),
        env=env,
    )
    try:
        output = read_until_prompt_or_exit(proc)
        if PROMPT not in output:
            output += drain(proc)
            err = cli_error_line(output)
            if err:
                raise SystemExit(
                    f"error: continue-launch failed before Proceed?: {err}"
                )
            if ALREADY_DONE in output:
                return "already_done"
            raise SystemExit(
                "error: continue-launch exited before Proceed? "
                "(see output above)"
            )

        pre_prompt, _sep, _rest = output.partition(PROMPT)
        printed = parse_printed_handles(pre_prompt)
        if printed != expected:
            try:
                answer(proc, "no")
            except BrokenPipeError:
                pass
            drain(proc)
            refuse_mismatch(rows, expected, printed, logical_skip)

        print(
            f"Handle list matches CSV skip={logical_skip}; answering yes.",
            flush=True,
        )
        answer(proc, "yes")
        rest = drain(proc)
        full = output + rest
        err = cli_error_line(full)
        if err:
            raise SystemExit(f"error: continue-launch failed after yes: {err}")
        if CONFIRMED not in full:
            raise SystemExit(
                "error: continue-launch did not print Confirmed! after yes "
                "(slot-machine exits 0 even on failure; aborting)"
            )
        return "ok"
    except KeyboardInterrupt:
        if proc.poll() is None:
            try:
                if proc.stdin is not None and not proc.stdin.closed:
                    answer(proc, "no")
            except (BrokenPipeError, OSError):
                pass
            proc.terminate()
        raise


def batch_count(total: int, start: int, handles_per_spend: int) -> int:
    if start >= total:
        return 0
    remaining = total - start
    return (remaining + handles_per_spend - 1) // handles_per_spend


def run_pass(
    cmd: list[str],
    rows: list[HandleRow],
    pass_name: str,
    skip: int,
    handles_per_spend: int,
) -> None:
    total = len(rows)
    planned = batch_count(total, skip, handles_per_spend)
    if planned == 0:
        print(
            f"=== {pass_name}: skip={skip} is past the last handle; skipping ==="
        )
        return

    run = 0
    while skip < total:
        expected = rows[skip : skip + handles_per_spend]
        safe_skip = safe_skip_for(skip, handles_per_spend)
        run += 1
        status = run_one(
            cmd,
            rows,
            expected,
            skip,
            safe_skip,
            pass_name,
            run,
            planned,
        )
        if status == "already_done":
            print(f"=== {pass_name}: all handles already registered ===")
            return
        skip += handles_per_spend


def main() -> None:
    os.chdir(Path(__file__).resolve().parent)
    cmd = sys.argv[1:]
    if not cmd:
        raise SystemExit(
            "usage: continue-launch-loop.py <continue-launch command...>"
        )
    if cmd == ["--self-test"]:
        self_test()
        return

    premine = opt_value(cmd, "--premine")
    if not premine:
        raise SystemExit("error: command must include --premine <file>")
    hps_raw = opt_value(cmd, "--handles-per-spend")
    if not hps_raw:
        raise SystemExit("error: command must include --handles-per-spend <n>")
    if not re.fullmatch(r"[1-9][0-9]*", hps_raw):
        raise SystemExit(
            f"error: --handles-per-spend must be a positive integer, got: {hps_raw}"
        )
    handles_per_spend = int(hps_raw)

    skip_raw = opt_value(cmd, "--skip")
    if skip_raw in (None, "", "n"):
        start_skip = 0
    elif re.fullmatch(r"[0-9]+", skip_raw):
        start_skip = int(skip_raw)
    else:
        raise SystemExit(
            f"error: --skip must be a non-negative integer, got: {skip_raw}"
        )

    rows = load_premine(Path(premine))
    total = len(rows)
    precommit_batches = batch_count(total, start_skip, handles_per_spend)
    register_batches = batch_count(total, 0, handles_per_spend)

    print(f"Premine {premine}: {total} handles, {handles_per_spend} per spend")
    print(
        f"Precommit pass starts at skip={start_skip} ({precommit_batches} runs); "
        f"register pass starts at skip=0 ({register_batches} runs)"
    )
    print(
        f"CLI --skip is max(0, logical_skip - {LOOKBEHIND_BATCHES}*"
        f"{handles_per_spend}) so recent reorgs are still visible"
    )

    run_pass(cmd, rows, "precommit", start_skip, handles_per_spend)
    print()
    print("=== precommit pass complete; resetting skip to 0 for register pass ===")
    run_pass(cmd, rows, "register", 0, handles_per_spend)
    print()
    print(f"Done: continue-launch finished for {total} handles.")


PRECOMMIT_SAMPLE = """
Some precommitment coins were not launched yet - they correspond to these handles:
  handle: test3, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, image_uris: "https://testnet-nfts.xchandles.com/v1/test3.png"
  handle: test4, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, image_uris: "https://testnet-nfts.xchandles.com/v1/test4.png"
  handle: test5, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, image_uris: "https://testnet-nfts.xchandles.com/v1/test5.png"
  handle: test6, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, image_uris: "https://testnet-nfts.xchandles.com/v1/test6.png"
  handle: test7, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, image_uris: "https://testnet-nfts.xchandles.com/v1/test7.png"
NFTs will be minted with royalty address: txch1394lscgcq9ftal4747gu3c4n7q5tudhqas9hx8h5kur5v29gnsgsm3qacy
Royalty basis points: 420
A one-sided offer will be created; it will consume:
  - 40 payment CAT mojos for creating precommitment coins
Proceed? (yes/no): 
"""

REGISTER_SAMPLE = """
All precommitment coins have already been created :)
These handles will be launched (total number=5):
  handle: test1, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, buy_time: 1766199600, n: 1, image_uris: "https://testnet-nfts.xchandles.com/v1/test1.png"
  handle: test2, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, buy_time: 1766199600, n: 1, image_uris: "https://testnet-nfts.xchandles.com/v1/test2.png"
  handle: test3, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, buy_time: 1766199600, n: 1, image_uris: "https://testnet-nfts.xchandles.com/v1/test3.png"
  handle: test4, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, buy_time: 1766199600, n: 1, image_uris: "https://testnet-nfts.xchandles.com/v1/test4.png"
  handle: test5, recipient: txch1snrh897fv9rt7ckqmcak4kva3rxzefhxz9u027qs8ja45quuqfuqf7j8yy, expiration: 1797757200, buy_time: 1766199600, n: 1, image_uris: "https://testnet-nfts.xchandles.com/v1/test5.png"
Fetching eve NFTs for handles...
A one-sided offer will be created; it will consume:
  - 1 mojo for the sake of it
Proceed? (yes/no): 
"""

MOCK_CLI = r"""
import os, sys
mode = os.environ["MOCK_MODE"]
sys.stdout.reconfigure(line_buffering=True)
if mode == "already_done":
    print("All handles have already been registered - nothing to do!", file=sys.stderr)
    sys.exit(0)
if mode == "fail_before":
    print("Error: boom")
    sys.exit(0)
print(os.environ["MOCK_PREAMBLE"], end="")
print("Proceed? (yes/no): ", end="", flush=True)
line = sys.stdin.readline().strip().lower()
if line != "yes":
    print("Error: that's not a clear 'yes'")
    sys.exit(0)
if mode == "fail_after":
    print("Error: transaction rejected")
    sys.exit(0)
print("Confirmed!")
"""


def self_test() -> None:
    rows = load_premine(Path("xchandles_premine_testnet11.csv"))
    assert [r.handle for r in rows] == [
        "test1",
        "test2",
        "test3",
        "test4",
        "test5",
        "test6",
        "test7",
    ]

    pre = parse_printed_handles(PRECOMMIT_SAMPLE.partition(PROMPT)[0])
    assert pre == rows[2:7], pre
    reg = parse_printed_handles(REGISTER_SAMPLE.partition(PROMPT)[0])
    assert reg == rows[0:5], reg
    assert parse_printed_handles(PRECOMMIT_SAMPLE) != rows[0:5]

    assert safe_skip_for(0, 5) == 0
    assert safe_skip_for(5, 5) == 0
    assert safe_skip_for(80, 5) == 0
    assert safe_skip_for(81, 5) == 1
    assert safe_skip_for(100, 5) == 20
    assert batch_count(7, 0, 5) == 2
    assert batch_count(7, 5, 5) == 1
    assert batch_count(7, 7, 5) == 0

    preamble = PRECOMMIT_SAMPLE.partition(PROMPT)[0]
    mock = ["python3", "-c", MOCK_CLI]
    expected = rows[2:7]

    status = _run_mock(
        mock,
        {"MOCK_MODE": "ok", "MOCK_PREAMBLE": preamble},
        rows,
        expected,
        logical_skip=2,
    )
    assert status == "ok"

    try:
        _run_mock(
            mock,
            {"MOCK_MODE": "ok", "MOCK_PREAMBLE": preamble},
            rows,
            rows[0:5],
            logical_skip=0,
        )
    except SystemExit as e:
        if e.code != 1:
            raise
    else:
        raise SystemExit("self-test: expected mismatch abort")

    status = _run_mock(
        mock,
        {"MOCK_MODE": "already_done", "MOCK_PREAMBLE": ""},
        rows,
        expected,
        logical_skip=2,
    )
    assert status == "already_done"

    try:
        _run_mock(
            mock,
            {"MOCK_MODE": "fail_after", "MOCK_PREAMBLE": preamble},
            rows,
            expected,
            logical_skip=2,
        )
    except SystemExit as e:
        if e.code in (None, 0):
            raise SystemExit("self-test: fail_after did not abort") from e
        if e.code == 1:
            pass
        elif not (isinstance(e.code, str) and "failed after yes" in e.code):
            raise
    else:
        raise SystemExit("self-test: expected fail_after abort")

    print("self-test ok")


def _run_mock(
    mock: list[str],
    extra_env: dict[str, str],
    rows: list[HandleRow],
    expected: list[HandleRow],
    logical_skip: int,
) -> str:
    env = os.environ.copy()
    env.update(extra_env)
    return run_one(
        mock,
        rows,
        expected,
        logical_skip,
        safe_skip_for(logical_skip, 5),
        "self-test",
        1,
        1,
        env=env,
    )


if __name__ == "__main__":
    main()
