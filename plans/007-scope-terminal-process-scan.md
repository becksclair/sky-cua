# Plan 007: Scan only terminal descendants when enriching terminal windows, instead of every process's fd table on every discovery

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-linux/src/windowing/terminal.rs crates/sky-cua-linux/src/windowing/registry.rs`
> (no `..HEAD`, so uncommitted edits count). If either file changed since
> this plan was written, compare the "Current state" excerpts against the
> live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts marked condensed are rustfmt multi-line code shown on
> fewer lines; formatting differences are not drift.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: 001 (so the `sky-cua-linux` tests actually run in the gate; not a code dependency)
- **Category**: perf
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

Window discovery on Linux ends with `enrich_terminal_windows`, which attaches
PTY session context to terminal-emulator windows. Its first action is
`read_process_table()`: for every pid in `/proc` it reads `stat`, `comm` and
`cmdline`, `readlink`s `cwd`, and lists and `readlink`s every entry under
`/proc/<pid>/fd`. Only afterwards does it check whether any window is a
terminal. On the development host that is about 850 processes and 13,000 fd
links, roughly 50 ms of synchronous syscalls, run inside the async discovery
path. Discovery itself runs three times per desktop observe and once more per
CUA action (audit finding `PERF-01`), so this cost lands three to four times
per tool call even when no terminal window is open. Restricting the full read
to descendants of terminal windows' pids, and skipping it entirely when there
are none, removes most of the work with identical output.

## Current state

- `crates/sky-cua-linux/src/windowing/terminal.rs` (598 lines; production
  is lines 1-331, `#[cfg(test)] mod tests` runs 333-598 with helpers
  `terminal_window(id, pid)` at 339 and `process(pid, ppid, start, name, tty)`
  at 365).
  - Lines 19-24:

    ```rust
    pub fn enrich_terminal_windows(windows: &mut [LinuxWindowInfo]) {
        let processes = read_process_table();
        if !processes.is_empty() {
            enrich_terminal_windows_with_processes(windows, &processes);
        }
    }
    ```

  - Lines 26-41: `enrich_terminal_windows_with_processes(windows, processes)`
    builds `windows_by_terminal_pid: BTreeMap<u32, Vec<usize>>` from windows
    where `looks_like_terminal_window(window)` and `window.pid` is `Some`,
    and returns early if the map is empty. It then builds
    `by_pid: HashMap<u32, &ProcessInfo>` and `terminal_pids: HashSet<u32>`
    and calls `terminal_session_index(processes, &by_pid, &terminal_pids)`.
  - Lines 116-148, `terminal_session_index`: for each process, walks
    ancestors via `by_pid.get(&current)` → `.ppid`; when an ancestor's
    `parent` is in `terminal_pids`, the process is indexed under that
    terminal pid by each of its `tty_paths`. The walk breaks when an ancestor
    is missing from `by_pid`, or `parent == 0`, or `parent == current`. Only
    processes that are descendants of a terminal pid ever get indexed. The
    walk also looks up each terminal pid in `by_pid` to continue upward
    (`current = parent` then `by_pid.get(&current)`), which is how a nested
    terminal's processes also get indexed under the outer terminal.
  - Lines 245-257 (condensed; the live code is rustfmt multi-line):

    ```rust
    fn read_process_table() -> Vec<ProcessInfo> {
        let Ok(entries) = fs::read_dir("/proc") else { return Vec::new(); };
        entries.flatten().filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            read_process_info(pid)
        }).collect()
    }
    ```

  - Lines 259-281, `read_process_info(pid)`: `parse_stat(pid)` (ppid,
    start_ticks), then `comm`, `read_command_line`, `cwd` readlink, and
    `read_tty_paths(pid)` (lines 312-327: `read_dir("/proc/{pid}/fd")` plus
    `read_link` per entry, keeping `/dev/pts/*`).
  - `parse_stat(pid)` (lines 296-299) reads only `/proc/<pid>/stat`;
    `parse_stat_contents` (301-310) is pure and already unit-tested
    (`parses_proc_stat_with_parenthesized_command`).
  - `ProcessInfo` (lines 9-17): `pid, ppid, start_ticks, command_name, command_line, cwd, tty_paths`.
  - Tests (333-598) construct `ProcessInfo` values with the `process(...)`
    helper and call `enrich_terminal_windows_with_processes` directly, e.g.
    `assigns_terminal_sessions_by_window_and_pty_creation_order` (line 403).
    They never touch `/proc`, so they are unaffected by this change and must
    keep passing unchanged. `terminal_window(id, pid)` builds a
    `LinuxWindowInfo` with `title: "Ghostty"`, `app_id: "com.mitchellh.ghostty.desktop"`
    and `wm_class: "com.mitchellh.ghostty"`; `looks_like_terminal_window`
    (216-243) checks all three fields, so a non-terminal window must override
    all three. `LinuxWindowInfo` has all-public fields and derives
    `Clone`/`PartialEq`, so struct-update syntax works.

- `crates/sky-cua-linux/src/windowing/registry.rs` lines 569-583:
  `finalize_window_list` calls `enrich_terminal_windows(windows)` (line 575)
  after dedupe and before sorting. It is a sync fn called from async
  discovery. Do not change it.

- `LinuxWindowInfo.pid: Option<u32>` exists (used at terminal.rs line 33).
  `HashMap`, `HashSet` and `BTreeMap` are already imported (line 1), and
  let-chains are already used (lines 32-33).

Conventions: Rust 2024; `cargo nextest run`; conventional commits, e.g.
`fix(linux): make X11 display-liveness tests hermetic`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy -p sky-cua-linux --all-targets -- -D warnings` | exit 0 |
| Module tests | `cargo nextest run -p sky-cua-linux windowing::terminal` | all pass (9 at `e928a40`, plus yours) |
| Crate tests | `cargo nextest run -p sky-cua-linux` | all pass (385 at `e928a40`) |

## Scope

**In scope** (the only file you should modify):
- `crates/sky-cua-linux/src/windowing/terminal.rs`

**Out of scope** (do NOT touch):
- `registry.rs`, including moving `finalize_window_list` to `spawn_blocking`.
- Caching the window list across discoveries (`PERF-01`); a separate, riskier plan.
- `looks_like_terminal_window` heuristics.
- The shape of `TerminalWindowContext` / `TerminalTargetSession` (public
  contract in `sky-cua-platform`).

## Git workflow

- Branch: `bex/terminal-scan-descendants-only`
- Commit only if the operator authorised it; style:
  `perf(linux): scan only terminal descendants during window enrichment`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Extract the terminal-window pid map so it can be computed before any `/proc` read

Factor the loop at lines 30-37 into:

```rust
fn terminal_window_pids(windows: &[LinuxWindowInfo]) -> BTreeMap<u32, Vec<usize>> {
    let mut windows_by_terminal_pid: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (index, window) in windows.iter().enumerate() {
        if looks_like_terminal_window(window)
            && let Some(pid) = window.pid
        {
            windows_by_terminal_pid.entry(pid).or_default().push(index);
        }
    }
    windows_by_terminal_pid
}
```

and have `enrich_terminal_windows_with_processes` call it (behaviour
unchanged).

**Verify**: `cargo nextest run -p sky-cua-linux windowing::terminal` → 9 pass.

### Step 2: Read a cheap ppid map and compute descendants

Add two pure-ish helpers:

```rust
/// `/proc/<pid>/stat` only: pid -> ppid for every process. One small read per
/// process, no fd listing.
fn read_parent_map() -> HashMap<u32, u32> {
    let Ok(entries) = fs::read_dir("/proc") else { return HashMap::new(); };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let (ppid, _) = parse_stat(pid)?;
            Some((pid, ppid))
        })
        .collect()
}

/// Every pid whose ancestry passes through one of `roots`. Pure, so it is
/// unit-testable without `/proc`.
fn descendant_pids(parents: &HashMap<u32, u32>, roots: &HashSet<u32>) -> HashSet<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, &ppid) in parents {
        children.entry(ppid).or_default().push(pid);
    }
    let mut out = HashSet::new();
    let mut stack: Vec<u32> = roots.iter().copied().collect();
    while let Some(pid) = stack.pop() {
        if let Some(kids) = children.get(&pid) {
            for &kid in kids {
                if out.insert(kid) {
                    stack.push(kid);
                }
            }
        }
    }
    out
}
```

**Verify**: `cargo check -p sky-cua-linux` compiles. Two `dead_code` warnings
for `read_parent_map` and `descendant_pids` are expected until Step 3 wires
them in; do not run the `-D warnings` clippy gate yet.

### Step 3: Rewrite `enrich_terminal_windows` to scan descendants only

Replace lines 19-24 with:

```rust
pub fn enrich_terminal_windows(windows: &mut [LinuxWindowInfo]) {
    let terminal_pids: HashSet<u32> = terminal_window_pids(windows).into_keys().collect();
    if terminal_pids.is_empty() {
        return;
    }
    let parents = read_parent_map();
    let wanted = descendant_pids(&parents, &terminal_pids);
    let processes: Vec<ProcessInfo> = wanted
        .into_iter()
        .filter_map(read_process_info)
        .collect();
    if !processes.is_empty() {
        enrich_terminal_windows_with_processes(windows, &processes);
    }
}
```

Why the output is identical: `terminal_session_index` only indexes a process
when its ancestor walk reaches a parent in `terminal_pids`, and the walk
stops at the first ancestor missing from `by_pid`. Every process that was
indexed before is a descendant of some terminal pid, so it is in `wanted`
along with every ancestor between it and that terminal; the walk therefore
reaches the same terminal pid as before. Indexing under a higher (outer)
terminal requires every node on the path, the inner terminal included, to be
a descendant of the outer one, so the descendant set contains them all and
the walk through `by_pid[inner_terminal]` still succeeds; this is why the
existing `terminal_session_index_preserves_nested_terminal_ancestor_semantics`
test keeps passing. A non-nested terminal's own absence from `by_pid` only
causes a `break` where the old walk would have indexed nothing anyway.
Processes that are not descendants were never indexed, and
`active_terminal_process`/`process_count` read only the per-session vectors
of indexed processes. Output is also independent of `processes` order (the
new set iterates in `HashSet` order, the old in readdir order): per-tty
vectors are totally sorted by `(depth, start_ticks, pid)` and sessions come
from a `BTreeMap`. The two-phase read (parent map, then per-pid details) can
miss a process forked or reparented between the phases; the old single pass
had a comparable race, so "identical" holds for a process tree that is not
changing mid-read. `read_process_table` becomes unused; delete it
(`read_process_info` and `parse_stat` keep other callers, so no new
dead-code warnings). Run `cargo fmt -p sky-cua-linux` after editing; the
Step 2 snippets are not rustfmt-shaped.

**Verify**: `cargo clippy -p sky-cua-linux --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-linux windowing::terminal` → all 9 pass.

### Step 4: Add tests for the pure helper

In the existing `mod tests`:

1. `descendant_pids_collects_transitive_children_only`: parents
   `{200:100, 201:200, 300:1, 301:300, 100:1}`, roots `{100}` → exactly
   `{200, 201}`.
2. `descendant_pids_handles_nested_terminal_roots`: parents
   `{200:100, 300:200, 301:300}`, roots `{100, 200}` →
   `HashSet::from([200, 300, 301])` whichever root pops first. (200 must be
   in the read set so that the walk from 300 can pass through `by_pid[200]`
   and also index under 100.)
3. `descendant_pids_terminates_on_cycles`: parents `{2:3, 3:2}`, roots `{2}`
   → exactly `HashSet::from([2, 3])` (pop 2 inserts 3; pop 3 inserts 2; pop 2
   finds 3 already present and stops). Assert equality.
4. `terminal_window_pids_skips_non_terminal_and_pidless_windows`:

   ```rust
   let windows = vec![
       terminal_window("11", 100),
       terminal_window("12", 200),
       LinuxWindowInfo {
           title: Some("Firefox".into()),
           app_id: Some("firefox".into()),
           wm_class: Some("firefox".into()),
           ..terminal_window("13", 300)
       },
       LinuxWindowInfo { pid: None, ..terminal_window("14", 0) },
   ];
   assert_eq!(
       terminal_window_pids(&windows),
       BTreeMap::from([(100, vec![0]), (200, vec![1])])
   );
   ```

   (All three of `title`, `app_id`, `wm_class` must be overridden; the
   helper's defaults all contain "ghostty".)

**Verify**: `cargo nextest run -p sky-cua-linux windowing::terminal` → all
pass, 13 tests.

### Step 5: Whole-crate run and a manual timing spot check

Run `cargo nextest run -p sky-cua-linux` → all pass (`isolated_*` tests take
about 30 s each; that is normal).

Optional, read-only, on a desktop session: stop any running installed
`sky-cua-service` daemon first (otherwise `get-app-state` connects to it via
`ServiceClient::connect_or_spawn()` and measures the old binary), then before
and after the change run
`cargo run -p sky-cua-client -- get-app-state --detail compact` a few times
and compare wall time with `time`. Expect a reduction on the order of tens of
milliseconds per call on a host with ~800 processes (rough measurement on the
dev host: the stat-only pass is ~25 % of the old full read; the fd scan it
drops is the bulk). Record the numbers if you have them; do not block on this
step.

## Test plan

- New: four tests in Step 4.
- Existing: all nine `windowing::terminal` tests unchanged and passing.
- Verification: `cargo nextest run -p sky-cua-linux` → all pass.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -p sky-cua-linux --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-linux` exits 0 with 4 new tests
- [ ] `grep -n 'fn read_process_table' crates/sky-cua-linux/src/windowing/terminal.rs` prints nothing
- [ ] `grep -n 'fn descendant_pids\|fn read_parent_map\|fn terminal_window_pids' crates/sky-cua-linux/src/windowing/terminal.rs` shows all three
- [ ] `git status --porcelain -- crates/` lists only `crates/sky-cua-linux/src/windowing/terminal.rs` (untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- Any existing `windowing::terminal` test fails after Step 3 (that would mean
  the equivalence argument above is wrong for some case; report which).
- `read_process_table` or `read_process_info` is used from another module
  (`grep -rn 'read_process_table\|read_process_info' crates/sky-cua-linux/src`);
  report before deleting.
- The change seems to require editing `registry.rs`.

## Maintenance notes

- The enrichment still runs synchronously inside async discovery. After this
  change its cost is proportional to the number of processes under terminal
  windows, which is small, so moving it to `spawn_blocking` is no longer
  pressing; `PERF-09` in the audit tracks the remaining sync work.
- `read_parent_map` still touches `/proc/<pid>/stat` for every process (one
  small read each, no fd listing), roughly a quarter of today's cost. The
  early return removes everything, but on a developer host with agents
  running in terminals it rarely fires; the main win in practice is dropping
  the per-process fd scan. If the stat pass ever shows up in profiles, the
  next step is caching the parent map for the duration of one discovery
  request, which depends on the window-list caching plan (`PERF-01`).
- A reviewer should check the equivalence argument in Step 3 against
  `terminal_session_index`, especially the early `break` on a missing
  `by_pid` entry.
