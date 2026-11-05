# srs

A small terminal spaced-repetition app built on [`fsrs`](https://crates.io/crates/fsrs) (FSRS-6).
It is intended for personal use and keeps all data in a human-readable RON file.

## Run

```sh
cargo run
cargo run -- path/to/data.ron
```

The data file defaults to `srs.ron` in the current directory. It is loaded at
startup, saved automatically after every change, and saved again on quit.
The startup banner says where it is loading from.

## Functionality added

### Home page

- Lists every item with due date, scheduled interval, review count, current
  stability/difficulty, and current retrievability.
- Sorts by due date (overdue first) or by name; `s` switches.
- Overdue due dates are red, retrievability is color-coded.
- Review the selected item with `1`-`4`.
- Undo the most recent review with `u`.
- Add, rename, and delete items.
- Non-blocking status messages in the top-right corner (last action, save
  errors, etc.).

### History page

- Open an item with `Enter` to see its complete review log.
- Every row shows the rating, elapsed time since the previous review, the
  memory state after that review, the interval scheduled at that time, and the
  resulting due date.
- The header summarizes the current state, due date, interval, retrievability,
  and the intervals the four answer buttons would schedule if the item were
  reviewed now (using `FSRS::next_states`).
- Edit a mistake:
  - `1`-`4` change the selected review's rating.
  - `x` deletes the selected review.
  - `t` edits the selected review's timestamp.
- Edits replay the whole review log, so all later states, intervals, and due
  dates are recalculated automatically.

### Persistence

- Human-readable RON (`serde` + `chrono` RFC 3339 timestamps).
- Missing/empty files start an empty collection; parse errors are shown in a
  dialog instead of being silently ignored.
- Writes go to a temporary file and are then renamed, so an interrupted save
  does not truncate the data.
- Duplicate names and out-of-order reviews are repaired on load.
- `desired_retention` is stored in the file (default `0.9`).

Example:

```ron
(
    version: 1,
    desired_retention: 0.9,
    histories: [
        (
            name: "kanji",
            created: "2026-11-05T12:15:35.678869346Z",
            reviews: [
                (when: "2026-11-05T12:15:35.687779761Z", rating: Good),
                (when: "2026-11-08T12:15:35.689189510Z", rating: Easy),
            ],
        ),
    ],
)
```

## Key bindings

### Home

| Key | Action |
| --- | --- |
| `Up` / `Down`, `k` / `j` | Move selection |
| `Enter`, `Right`, `l` | Open review history |
| `1` `2` `3` `4` | Review now: Again / Hard / Good / Easy |
| `a` | Add item |
| `e` | Rename item |
| `d` | Delete item |
| `u` | Undo the last review |
| `s` | Switch sort (due / name) |
| `?` | Help |
| `q`, `Esc` | Save and quit |

### History

| Key | Action |
| --- | --- |
| `Up` / `Down`, `k` / `j` | Select a review |
| `1` `2` `3` `4` | Change the selected review's rating |
| `x`, `Delete` | Delete the selected review |
| `t` | Change the selected review's time |
| `Esc`, `Left`, `h`, `q` | Back to Home |
| `?` | Help |

### Input dialogs

`Left`/`Right`/`Home`/`End` move the cursor, `Backspace`/`Delete` edit,
`Ctrl+U` clears the field, `Enter` confirms, `Esc` cancels.

## FSRS usage

The app stores only `(timestamp, rating)` pairs. All scheduling state is derived
on demand with the higher-level APIs of `fsrs` 6.6.2:

- `FSRS::historical_memory_states` replays the full review log and returns the
  memory state after each review.
- `FSRS::next_interval` turns each state into the interval that was scheduled.
- `FSRS::next_states` produces the four-button preview and the "if reviewed now"
  row on the history page.
- `fsrs::current_retrievability` computes the current recall probability.

Default FSRS parameters and a desired retention of 0.90 are used. Timestamps are
stored in UTC and displayed in local time. A never-reviewed item is considered
due immediately.

## Development

```sh
cargo test
cargo clippy --all-targets
```

The tests cover review replay, editing/deleting reviews, time-ordering
validation, save/load round trips, and rendering every page/dialog with a
`TestBackend`.
