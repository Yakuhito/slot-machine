#!/usr/bin/env python3
"""Drive `xchandles continue-launch` through one pass, then exit.

Default is the precommit pass (precommit + eve NFT mint). Re-run with
`--register-only` for the register pass. The two are not chained.

The CLI `--skip` sent to each run is one handle behind the logical skip so it
still checks the previous on-chain coin. A reorg that undid recent work then
prints earlier handles and is still discovered. The script only answers `yes`
at `Proceed?` when the printed handles match the CSV rows at the logical skip.
If the CLI lists *earlier* handles (reorg of recent work), it answers `no`,
waits 30s then 300s, and retries; it exits only if the list is still early.
If push returns DOUBLE_SPEND after yes, wait 10s and retry the same skip
(new offer); skip is not increased. If that previous tx actually confirmed,
the CLI prints later handles and this script exits.

`slot-machine` prints `Error: ...` and still exits 0, so this script treats
`Confirmed!` / `Error:` in the output as the real success/failure signal.

Usage:
  ./continue-launch-loop.py [loop options] cargo r xchandles continue-launch ... \\
      --premine FILE --handles-per-spend N [--skip START]

Loop options (stripped before the CLI runs; default to --handles-per-spend):
  --precommit-handles-per-spend N   first pass (precommit + eve NFT mint)
  --register-handles-per-spend N    second pass (register eve NFTs)
  --register-only                   register pass only (do not run precommit)
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

EARLIER_RETRY_WAITS = (30, 300)
DOUBLE_SPEND_RETRY_WAIT = 10
PRECOMMIT_HANDLES_OPT = "--precommit-handles-per-spend"
REGISTER_HANDLES_OPT = "--register-handles-per-spend"
REGISTER_ONLY_OPT = "--register-only"
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


def take_opt(cmd: list[str], name: str) -> tuple[str | None, list[str]]:
    out: list[str] = []
    value: str | None = None
    i = 0
    while i < len(cmd):
        arg = cmd[i]
        if arg == name:
            if i + 1 >= len(cmd):
                raise SystemExit(f"error: {name} is missing a value")
            if value is not None:
                raise SystemExit(f"error: {name} specified more than once")
            value = cmd[i + 1]
            i += 2
            continue
        if arg.startswith(name + "="):
            if value is not None:
                raise SystemExit(f"error: {name} specified more than once")
            value = arg.split("=", 1)[1]
            i += 1
            continue
        out.append(arg)
        i += 1
    return value, out


def take_flag(cmd: list[str], name: str) -> tuple[bool, list[str]]:
    present = False
    out: list[str] = []
    for arg in cmd:
        if arg == name:
            present = True
            continue
        if arg.startswith(name + "="):
            raise SystemExit(f"error: {name} does not take a value")
        out.append(arg)
    return present, out


def parse_positive_int(name: str, raw: str) -> int:
    if not re.fullmatch(r"[1-9][0-9]*", raw):
        raise SystemExit(f"error: {name} must be a positive integer, got: {raw}")
    return int(raw)


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


def safe_skip_for(logical_skip: int) -> int:
    return max(0, logical_skip - 1)


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


def printed_starts_earlier(
    rows: list[HandleRow],
    printed: list[HandleRow],
    logical_skip: int,
) -> bool:
    if not printed:
        return False
    idx = csv_index(rows, printed[0].handle)
    return idx is not None and idx < logical_skip


def print_handle_mismatch(
    rows: list[HandleRow],
    expected: list[HandleRow],
    printed: list[HandleRow],
    logical_skip: int,
) -> None:
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
    print_handle_mismatch(rows, expected, printed, logical_skip)
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
) -> tuple[str, list[HandleRow]]:
    launch_cmd = strip_flag(cmd, "--yes")
    launch_cmd = set_opt(launch_cmd, "--skip", str(safe_skip))

    print()
    print(
        f"=== {pass_name} run {run}/{planned}  skip={logical_skip}  "
        f"safe-skip={safe_skip}  handles={len(expected)}  "
        f"handles-per-spend={opt_value(cmd, '--handles-per-spend')} ==="
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
                return "already_done", []
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
            if printed_starts_earlier(rows, printed, logical_skip):
                return "earlier", printed
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
            if "DOUBLE_SPEND" in err:
                return "double_spend", printed
            raise SystemExit(f"error: continue-launch failed after yes: {err}")
        if CONFIRMED not in full:
            raise SystemExit(
                "error: continue-launch did not print Confirmed! after yes "
                "(slot-machine exits 0 even on failure; aborting)"
            )
        return "ok", printed
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


def run_one_with_retries(
    cmd: list[str],
    rows: list[HandleRow],
    expected: list[HandleRow],
    logical_skip: int,
    safe_skip: int,
    pass_name: str,
    run: int,
    planned: int,
    env: dict[str, str] | None = None,
    sleep_fn: Callable[[float], None] = time.sleep,
) -> str:
    last_printed: list[HandleRow] = []
    earlier_attempt = 0
    while True:
        status, printed = run_one(
            cmd,
            rows,
            expected,
            logical_skip,
            safe_skip,
            pass_name,
            run,
            planned,
            env=env,
        )
        if status == "double_spend":
            print(
                f"DOUBLE_SPEND after yes at skip={logical_skip}; "
                f"waiting {DOUBLE_SPEND_RETRY_WAIT}s then retrying the same skip "
                "(new offer).",
                flush=True,
            )
            sleep_fn(DOUBLE_SPEND_RETRY_WAIT)
            continue
        if status != "earlier":
            return status
        last_printed = printed
        if earlier_attempt >= len(EARLIER_RETRY_WAITS):
            break
        wait = EARLIER_RETRY_WAITS[earlier_attempt]
        print(
            f"CLI listed earlier handles than skip={logical_skip}; "
            f"answering no and waiting {wait}s before retry "
            f"{earlier_attempt + 1}/{len(EARLIER_RETRY_WAITS)}.",
            flush=True,
        )
        print_handle_mismatch(rows, expected, printed, logical_skip)
        sleep_fn(wait)
        earlier_attempt += 1
    refuse_mismatch(rows, expected, last_printed, logical_skip)


def run_pass(
    cmd: list[str],
    rows: list[HandleRow],
    pass_name: str,
    skip: int,
    handles_per_spend: int,
    env: dict[str, str] | None = None,
    sleep_fn: Callable[[float], None] = time.sleep,
) -> None:
    total = len(rows)
    cmd = set_opt(cmd, "--handles-per-spend", str(handles_per_spend))
    planned = batch_count(total, skip, handles_per_spend)
    if planned == 0:
        print(
            f"=== {pass_name}: skip={skip} is past the last handle; skipping ==="
        )
        return

    run = 0
    while skip < total:
        expected = rows[skip : skip + handles_per_spend]
        safe_skip = safe_skip_for(skip)
        run += 1
        status = run_one_with_retries(
            cmd,
            rows,
            expected,
            skip,
            safe_skip,
            pass_name,
            run,
            planned,
            env=env,
            sleep_fn=sleep_fn,
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
            "usage: continue-launch-loop.py "
            "[--precommit-handles-per-spend N] [--register-handles-per-spend N] "
            "[--register-only] <continue-launch command...>"
        )
    if cmd == ["--self-test"]:
        self_test()
        return

    precommit_hps_raw, cmd = take_opt(cmd, PRECOMMIT_HANDLES_OPT)
    register_hps_raw, cmd = take_opt(cmd, REGISTER_HANDLES_OPT)
    register_only, cmd = take_flag(cmd, REGISTER_ONLY_OPT)

    premine = opt_value(cmd, "--premine")
    if not premine:
        raise SystemExit("error: command must include --premine <file>")
    hps_raw = opt_value(cmd, "--handles-per-spend")
    if not hps_raw:
        raise SystemExit("error: command must include --handles-per-spend <n>")
    handles_per_spend = parse_positive_int("--handles-per-spend", hps_raw)
    precommit_hps = (
        parse_positive_int(PRECOMMIT_HANDLES_OPT, precommit_hps_raw)
        if precommit_hps_raw is not None
        else handles_per_spend
    )
    register_hps = (
        parse_positive_int(REGISTER_HANDLES_OPT, register_hps_raw)
        if register_hps_raw is not None
        else handles_per_spend
    )

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
    precommit_batches = (
        0 if register_only else batch_count(total, start_skip, precommit_hps)
    )
    register_start = start_skip if register_only else 0
    register_batches = batch_count(total, register_start, register_hps)

    print(
        f"Premine {premine}: {total} handles; "
        f"precommit {precommit_hps}/spend, register {register_hps}/spend"
    )
    if register_only:
        print(
            f"Starting at register pass (--register-only), "
            f"skip={register_start} ({register_batches} runs)"
        )
    else:
        print(
            f"Precommit pass starts at skip={start_skip} ({precommit_batches} runs); "
            "exiting after precommit (re-run with --register-only for registrations)"
        )
    print(
        "CLI --skip is one handle behind the logical skip so the previous "
        "on-chain coin is still checked and recent reorgs stay visible"
    )

    if register_only:
        run_pass(cmd, rows, "register", register_start, register_hps)
        print()
        print(f"Done: register pass finished for {total} handles.")
        return

    run_pass(cmd, rows, "precommit", start_skip, precommit_hps)
    print()
    print(
        f"Done: precommit pass finished for {total} handles. "
        "Re-run with --register-only to register eve NFTs."
    )


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
expect = os.environ.get("MOCK_EXPECT_HPS")
if expect:
    got = None
    args = sys.argv[1:]
    for i, arg in enumerate(args):
        if arg == "--handles-per-spend" and i + 1 < len(args):
            got = args[i + 1]
        elif arg.startswith("--handles-per-spend="):
            got = arg.split("=", 1)[1]
    if got != expect:
        print(f"Error: expected --handles-per-spend {expect}, got {got}")
        sys.exit(0)
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
if mode == "double_spend":
    path = os.environ["MOCK_COUNTER"]
    with open(path) as f:
        n = int(f.read() or "0")
    with open(path, "w") as f:
        f.write(str(n + 1))
    if n == 0:
        print("Submitting transaction...")
        print(
            "Transaction submitted; status='', error='Failed to include "
            "transaction abc, error DOUBLE_SPEND'"
        )
        print(
            "Error: custom error: Failed to include transaction abc, "
            "error DOUBLE_SPEND"
        )
        sys.exit(0)
print("Confirmed!")
"""


def self_test() -> None:
    rows = load_premine(Path("xchandles_premine_testnet11.csv"))
    assert [r.handle for r in rows[:7]] == [
        "test1",
        "test2",
        "test3",
        "test4",
        "test5",
        "test6",
        "test7",
    ]
    rows7 = rows[:7]

    pre = parse_printed_handles(PRECOMMIT_SAMPLE.partition(PROMPT)[0])
    assert [r.handle for r in pre] == ["test3", "test4", "test5", "test6", "test7"]
    reg = parse_printed_handles(REGISTER_SAMPLE.partition(PROMPT)[0])
    assert [r.handle for r in reg] == ["test1", "test2", "test3", "test4", "test5"]

    assert printed_starts_earlier(rows7, rows7[2:7], 5)
    assert not printed_starts_earlier(rows7, rows7[2:7], 2)
    assert not printed_starts_earlier(rows7, rows7[2:7], 0)
    assert not printed_starts_earlier(rows7, [], 5)

    assert safe_skip_for(0) == 0
    assert safe_skip_for(1) == 0
    assert safe_skip_for(2) == 1
    assert safe_skip_for(5) == 4
    assert safe_skip_for(100) == 99
    assert batch_count(7, 0, 5) == 2
    assert batch_count(7, 5, 5) == 1
    assert batch_count(7, 7, 5) == 0
    assert batch_count(250, 0, 250) == 1
    assert batch_count(280, 0, 30) == 10

    taken, rest = take_opt(
        ["cargo", PRECOMMIT_HANDLES_OPT, "250", "r", REGISTER_HANDLES_OPT + "=30"],
        PRECOMMIT_HANDLES_OPT,
    )
    assert taken == "250"
    taken, rest = take_opt(rest, REGISTER_HANDLES_OPT)
    assert taken == "30" and rest == ["cargo", "r"]
    taken, rest = take_opt(["cargo", "r"], REGISTER_HANDLES_OPT)
    assert taken is None and rest == ["cargo", "r"]
    present, rest = take_flag(
        ["cargo", REGISTER_ONLY_OPT, "r", "--handles-per-spend", "30"],
        REGISTER_ONLY_OPT,
    )
    assert present and rest == ["cargo", "r", "--handles-per-spend", "30"]
    present, rest = take_flag(["cargo", "r"], REGISTER_ONLY_OPT)
    assert not present and rest == ["cargo", "r"]
    assert parse_positive_int("--handles-per-spend", "30") == 30
    try:
        parse_positive_int(REGISTER_HANDLES_OPT, "300x")
    except SystemExit:
        pass
    else:
        raise SystemExit("self-test: expected invalid register handles-per-spend")

    def preamble_for(batch: list[HandleRow]) -> str:
        lines = [
            "Some precommitment coins were not launched yet - they correspond to these handles:"
        ]
        for row in batch:
            lines.append(
                f"  handle: {row.handle}, recipient: {row.recipient}, "
                f"expiration: {row.expiration}, image_uris: \"x\""
            )
        lines.append("")
        return "\n".join(lines)

    mock = ["python3", "-c", MOCK_CLI]
    expected = rows7[2:7]
    preamble = preamble_for(expected)

    status, _printed = _run_mock(
        mock,
        {"MOCK_MODE": "ok", "MOCK_PREAMBLE": preamble},
        rows7,
        expected,
        logical_skip=2,
    )
    assert status == "ok"

    try:
        _run_mock(
            mock,
            {"MOCK_MODE": "ok", "MOCK_PREAMBLE": preamble},
            rows7,
            rows7[0:5],
            logical_skip=0,
        )
    except SystemExit as e:
        if e.code != 1:
            raise
    else:
        raise SystemExit("self-test: expected mismatch abort")

    status, _printed = _run_mock(
        mock,
        {"MOCK_MODE": "already_done", "MOCK_PREAMBLE": ""},
        rows7,
        expected,
        logical_skip=2,
    )
    assert status == "already_done"

    try:
        _run_mock(
            mock,
            {"MOCK_MODE": "fail_after", "MOCK_PREAMBLE": preamble},
            rows7,
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

    slept: list[float] = []
    try:
        _run_mock_with_retries(
            mock,
            {"MOCK_MODE": "ok", "MOCK_PREAMBLE": preamble_for(rows7[2:7])},
            rows7,
            rows7[5:7],
            logical_skip=5,
            sleep_fn=slept.append,
        )
    except SystemExit as e:
        if e.code != 1:
            raise
    else:
        raise SystemExit("self-test: expected earlier-handle abort after retries")
    assert slept == [30, 300], slept

    with tempfile.NamedTemporaryFile(mode="w", delete=False) as counter:
        counter.write("0")
        counter_path = counter.name
    slept = []
    status = _run_mock_with_retries(
        mock,
        {
            "MOCK_MODE": "double_spend",
            "MOCK_PREAMBLE": preamble,
            "MOCK_COUNTER": counter_path,
        },
        rows7,
        expected,
        logical_skip=2,
        sleep_fn=slept.append,
    )
    assert status == "ok"
    assert slept == [10], slept
    with open(counter_path) as f:
        assert f.read() == "2"
    os.unlink(counter_path)

    rows3 = rows7[:3]
    env = os.environ.copy()
    env.update(
        {
            "MOCK_MODE": "ok",
            "MOCK_PREAMBLE": preamble_for(rows3),
            "MOCK_EXPECT_HPS": "3",
        }
    )
    run_pass(
        ["python3", "-c", MOCK_CLI, "--handles-per-spend", "5"],
        rows3,
        "precommit",
        0,
        3,
        env=env,
        sleep_fn=lambda _s: None,
    )

    print("self-test ok")


def _run_mock(
    mock: list[str],
    extra_env: dict[str, str],
    rows: list[HandleRow],
    expected: list[HandleRow],
    logical_skip: int,
) -> tuple[str, list[HandleRow]]:
    env = os.environ.copy()
    env.update(extra_env)
    return run_one(
        mock,
        rows,
        expected,
        logical_skip,
        safe_skip_for(logical_skip),
        "self-test",
        1,
        1,
        env=env,
    )


def _run_mock_with_retries(
    mock: list[str],
    extra_env: dict[str, str],
    rows: list[HandleRow],
    expected: list[HandleRow],
    logical_skip: int,
    sleep_fn: Callable[[float], None],
) -> str:
    env = os.environ.copy()
    env.update(extra_env)
    return run_one_with_retries(
        mock,
        rows,
        expected,
        logical_skip,
        safe_skip_for(logical_skip),
        "self-test",
        1,
        1,
        env=env,
        sleep_fn=sleep_fn,
    )


if __name__ == "__main__":
    main()
