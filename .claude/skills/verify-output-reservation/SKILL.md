---
name: verify-output-reservation
description: "Model-check how Toolbox reserves, publishes and cleans up output files, so a change can never delete or replace a file it did not write. Use when a change touches OutputNaming, OutputReservation, its Drop, publish_within_time, or the claim and temporary file names in ToolboxTauri/src-tauri/src/kit/common/mod.rs."
---

# Verify output reservation

Toolbox promises never to delete or replace a file it did not write. The promise rests on a small protocol in `ToolboxTauri/src-tauri/src/kit/common/mod.rs`. A writer claims a name, writes a private temporary file, and hard-links that file to the name, which fails if anything is already there. Dropping the reservation removes only the temporary file and the claim.

`ToolboxTauri/models/output-reservation` is a Stateright model of that protocol. It tries every order of steps for two Toolbox writers and a third party (the user or another app) that ignores claim files and saves into any empty name.

## When this applies

Run it when a diff touches anything in the coverage record below. A change elsewhere does not need it.

## Run

```sh
cargo test --locked --manifest-path ToolboxTauri/models/output-reservation/Cargo.toml -- --nocapture
cargo test --manifest-path ToolboxTauri/src-tauri/Cargo.toml kit::common::tests
```

The PR Checks workflow runs the first command on every pull request. The second command tests the real code on a real filesystem. The model checks the design, and only the second command can catch code that drifts from it, so run both.

## Properties

| Property | Harm if broken |
| --- | --- |
| A third party's file is never deleted or replaced | A user loses a file they saved |
| A published output is never deleted or replaced | A finished result disappears |
| Two Toolbox writers never pick the same name | One of two parallel jobs fails to save |
| Finished writers leave no claim or temporary file | Hidden debris piles up beside the user's files |

`both writers publish` and `a writer publishes beside a third party's file` are reachability checks. They fail when an edit makes the model unable to reach the interesting states, which would let every other property pass with nothing checked.

`the_f4d7116e_protocol_deletes_a_third_partys_file` keeps the protocol from commit f4d7116e as a variant. That protocol wrote the destination itself, and its Drop deleted an unpublished destination. The test expects a counterexample and prints it. If it stops finding one, the model has lost the power to see the bug.

## Reading a failure

A failing property prints the steps that break it. `Writer(i, Advance)` is the next filesystem call in `reserve_candidate` for writer `i`. `Publish` is the `hard_link` in `OutputReservation::publish`. `JobFails` drops the reservation without publishing. `ThirdPartySaves(n)` is someone outside Toolbox creating candidate name `n`.

Replay the steps against the code. If the code allows them, the bug is real. Report it, and do not weaken the property. If the code forbids a step the model allowed, the model is wrong. Fix the model and say which code line it misread.

## Coverage record

| Model element | Code |
| --- | --- |
| `Step::CheckExists`, `Claim`, `RecheckAfterClaim`, `CreateTemporary`, `RecheckAfterTemporary` | `OutputNaming::reserve_candidate` |
| `next_candidate` | `OutputNaming::allocate_in`, bounded to two names |
| `Move::Publish` | `OutputReservation::publish` (`std::fs::hard_link`) |
| `Step::Dropping` | `impl Drop for OutputReservation` |
| `Protocol::DeletesDestinationOnDrop` | `OutputReservation` at commit f4d7116e |

Out of scope: `FolderReservation`, `StagedFolderReservation`, `retry_while_busy` timing on Windows, and the time-limit bookkeeping in `publish_within_time`. Bounds: two writers, two candidate names, and one third-party save.

Sync commit: `ba2f6a83`. Move it forward only after you have read the covered code at the new commit and confirmed that every row above still describes it.
