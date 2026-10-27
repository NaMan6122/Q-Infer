//! Snapshot regression tests for the terminal dashboard.
//!
//! These exist because the drawn output of the dashboard has no other automated
//! witness. `ui()` is private to the binary, so nothing in a library test can
//! observe what the tabs actually look like; `qinfer --snapshot WxH --tab <name>`
//! is the one seam that renders a real frame without a TTY, and it was built for
//! exactly this. Driving the binary as a subprocess keeps the suite honest about
//! what a user over a pipe would see.
//!
//! FR-59 is the rule most of this file protects: a narrow terminal must not clip
//! a value or push it past the frame. Layout arithmetic that goes wrong tends to
//! go wrong only below some width, so the sizes here walk down into the narrow
//! branches (the overview switches layout below 76 columns, the frontier sheds
//! columns below 84 and 64, the bench table below 84) rather than testing one
//! comfortable terminal.
//!
//! Nothing here asserts a model name, a plan count, or a predicted number. The
//! catalogue is versioned separately from the layout, and a suite that broke
//! every time a measurement moved would be deleted within a month. What is
//! asserted is the shape of the frame and the presence of the kind of
//! information each tab exists to carry.

use std::process::{Command, Output};

/// The binary is run from the crate directory, not from wherever `cargo test`
/// happened to be invoked. `Workspace::discover()` walks up from the current
/// directory looking for `spikes/m0/fingerprint.json`; from the crate directory
/// that walk reaches the workspace root and finds the committed fingerprint.
/// Without this, a run from an unrelated directory finds no data and every tab
/// renders its empty state, which would leave the content assertions with
/// nothing meaningful to check.
const CRATE_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// The tabs the binary accepts. An unknown name falls back to `overview`, so a
/// typo in this list would silently test the overview tab five times.
const TABS: [&str; 6] = ["overview", "setup", "bench", "frontier", "plans", "help"];

// The frame's corner glyphs, extracted from real output of
// `qinfer --snapshot 100x30 --tab overview` (ratatui's default `Borders::ALL`
// set, Unicode box drawing light; one `char` per cell, which is why the width
// checks below count chars and not bytes), rather than typed from memory.
const TOP_LEFT: char = '┌';
const TOP_RIGHT: char = '┐';
const BOTTOM_LEFT: char = '└';
const BOTTOM_RIGHT: char = '┘';

fn run(width: u16, height: u16, tab: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qinfer"))
        .arg("--snapshot")
        .arg(format!("{width}x{height}"))
        .arg("--tab")
        .arg(tab)
        .current_dir(CRATE_DIR)
        .output()
        .unwrap_or_else(|e| panic!("could not run {}: {e}", env!("CARGO_BIN_EXE_qinfer")))
}

/// One frame, with a failure that names the tab and the size. Diagnosing a
/// snapshot failure from a bare `exit status: 1` is guesswork, so stderr is
/// carried into the panic message.
fn frame(width: u16, height: u16, tab: &str) -> String {
    let out = run(width, height, tab);
    assert!(
        out.status.success(),
        "`--tab {tab}` at {width}x{height} exited with {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr).trim_end()
    );
    String::from_utf8(out.stdout).expect("every cell holds a UTF-8 symbol")
}

/// The rows of a frame. The renderer terminates every row with '\n', so the
/// empty tail left by `split` is a terminator and not a row of its own.
fn frame_rows(frame: &str) -> Vec<&str> {
    let mut rows: Vec<&str> = frame.split('\n').collect();
    if rows.last() == Some(&"") {
        rows.pop();
    }
    rows
}

/// Every tab must survive a render on an ordinary terminal. A tab that panics
/// here is one the user cannot reach at all, and the snapshot path is the only
/// place that failure shows up as an exit code instead of a broken dashboard.
#[test]
fn every_tab_renders_without_failing() {
    for tab in TABS {
        let out = frame(100, 30, tab);
        let rows = frame_rows(&out);
        assert!(!out.is_empty(), "`{tab}` produced no output at all");
        assert!(
            rows.iter().any(|r| !r.trim().is_empty()),
            "`{tab}` produced a frame of blank rows only"
        );
        assert_eq!(
            rows.len(),
            30,
            "`{tab}` rendered {} rows, not the 30 it was asked for",
            rows.len()
        );
    }
}

/// FR-59, the narrow-terminal rule: the frame must be exactly the size that was
/// requested. An extra row means something overflowed the layout; a row one
/// character too wide means a value ran past the border, which is what a user on
/// a 46-column terminal actually sees as corruption rather than as a warning.
#[test]
fn the_frame_is_exactly_the_requested_size() {
    for (w, h) in [(100u16, 30u16), (80, 24), (64, 20), (46, 16)] {
        for tab in TABS {
            let out = frame(w, h, tab);
            let rows = frame_rows(&out);
            assert_eq!(
                rows.len(),
                h as usize,
                "`{tab}` at {w}x{h} rendered {} rows, expected {h}",
                rows.len()
            );
            for (y, row) in rows.iter().enumerate() {
                let n = row.chars().count();
                assert!(
                    n <= w as usize,
                    "`{tab}` at {w}x{h}: row {y} is {n} chars wide, past the {w}-column frame (FR-59): {row:?}"
                );
            }
        }
    }
}

/// A frame with a hole in it reads as a rendering bug to the user even when the
/// data behind it is fine, and a missing left or right edge is the first symptom
/// of a layout that is drawing outside its area. The bottom edge is checked as
/// the last *bordered* row rather than the literal last row because the status
/// bar owns the final row by design (layout: header, body, `Length(1)` status);
/// asserting the literal last row would be asserting a layout this dashboard
/// has never had. Both facts are pinned below, so a status bar that vanished
/// (letting the border slide down into the last row) fails just as loudly.
#[test]
fn the_border_is_closed() {
    for (w, h) in [(100u16, 30u16), (64, 20)] {
        for tab in TABS {
            let out = frame(w, h, tab);
            let rows = frame_rows(&out);
            assert_eq!(rows.len(), h as usize, "`{tab}` at {w}x{h}");

            let top = rows[0];
            assert_eq!(
                top.chars().next(),
                Some(TOP_LEFT),
                "`{tab}` at {w}x{h}: the top line does not open with the top-left corner: {top:?}"
            );
            assert_eq!(
                top.chars().last(),
                Some(TOP_RIGHT),
                "`{tab}` at {w}x{h}: the top line does not close with the top-right corner: {top:?}"
            );

            let bottom = rows[h as usize - 2];
            assert_eq!(
                bottom.chars().next(),
                Some(BOTTOM_LEFT),
                "`{tab}` at {w}x{h}: the frame's bottom edge does not open with the bottom-left corner: {bottom:?}"
            );
            assert_eq!(
                bottom.chars().last(),
                Some(BOTTOM_RIGHT),
                "`{tab}` at {w}x{h}: the frame's bottom edge does not close with the bottom-right corner: {bottom:?}"
            );

            let status = rows[h as usize - 1];
            assert_ne!(
                status.chars().next(),
                Some(BOTTOM_LEFT),
                "`{tab}` at {w}x{h}: the status bar row has become the border, so the status line is gone: {status:?}"
            );
        }
    }
}

/// The frontier tab exists to answer "what should I run", so a frame from it
/// that carries no recommendation and no explanation of why there is none is a
/// regression regardless of which models the catalogue happens to hold. The
/// assertion is deliberately about the *shape* of the answer: a real row quotes
/// throughput in `tok/s` and marks the chosen plan with `->`, and when no
/// frontier can be drawn the tab must say so rather than rendering nothing.
/// No model name appears here on purpose: the catalogue is versioned apart from
/// this layout and the test must survive its next revision.
#[test]
fn the_frontier_tab_shows_a_recommendation() {
    let out = frame(100, 30, "frontier");

    let has_row = out.contains("tok/s");
    // The empty states the core can produce: no frontier at all, or a frontier
    // in which nothing clears the usable floor.
    let explained = out.contains("no feasible")
        || out.contains("no frontier")
        || out.contains("nothing above any floor");

    assert!(
        has_row || explained,
        "the frontier tab neither shows a throughput row nor explains why there is none:\n{out}"
    );

    if has_row {
        assert!(
            out.contains("->") || explained,
            "the frontier tab shows rows but no recommendation marker and no reason for its absence:\n{out}"
        );
    }
}

/// Ratatui is expected to clip rather than panic at sizes this small, and the
/// dashboard should degrade to an unreadable frame, not to a non-zero exit. Only
/// the process outcome is asserted: at 20x8 there is no honest content to check,
/// and demanding any would pin a layout nobody should rely on.
#[test]
fn no_tab_panics_at_a_degenerate_size() {
    for (w, h) in [(30u16, 10u16), (20, 8)] {
        for tab in TABS {
            let out = run(w, h, tab);
            assert!(
                out.status.success(),
                "`--tab {tab}` at {w}x{h} exited with {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim_end()
            );
        }
    }
}

/// The header is the one region that must survive any size: "which machine am I
/// on, and which view am I looking at" is the whole orientation function of the
/// dashboard.
///
/// This was not theoretical. The body constraint was `Min(10)`, so at 46x12
/// ratatui satisfied the minimum by starving the header and the machine name and
/// tab bar vanished entirely — while the degenerate-size test above still passed,
/// because it asserted only that the process exited zero. A frame with no header
/// is a rendering bug that exits successfully.
#[test]
fn the_header_survives_a_short_terminal() {
    for (w, h) in [(46u16, 12u16), (30, 10), (20, 8)] {
        let out = frame(w, h, "setup");
        let rows: Vec<&str> = out.lines().collect();
        assert!(rows.len() > 1, "a frame with no room for a header at {w}x{h}:\n{out}");
        assert!(
            rows[1].contains("Q-Infer"),
            "the machine identity row vanished at {w}x{h}:\n{out}"
        );
        // The bar degrades from names to abbreviations to digits, so accept any
        // of the three rather than pinning which tier a given width gets.
        let tabs = rows.get(2).copied().unwrap_or("");
        let has_bar = tabs.contains("Overview")
            || tabs.contains("Over")
            || tabs.chars().any(|c| c.is_ascii_digit());
        assert!(
            has_bar,
            "the tab bar vanished at {w}x{h} (row 2 is {tabs:?}):\n{out}"
        );
    }
}

/// A frame that changes between two identical runs cannot be reviewed, diffed in
/// CI, or checked into a golden file, and it usually means a timestamp or an
/// iteration over a hash map leaked into the drawing code. This is the property
/// the whole snapshot mechanism rests on, so it is asserted rather than assumed.
#[test]
fn the_snapshot_is_stable_across_two_runs() {
    for tab in TABS {
        let first = frame(100, 30, tab);
        let second = frame(100, 30, tab);
        if first == second {
            continue;
        }
        let a = frame_rows(&first);
        let b = frame_rows(&second);
        let at = (0..a.len().min(b.len()))
            .find(|&i| a[i] != b[i])
            .unwrap_or(a.len().min(b.len()));
        panic!(
            "`{tab}` rendered differently on two identical runs, first difference at row {at}:\n  run 1: {:?}\n  run 2: {:?}",
            a.get(at),
            b.get(at)
        );
    }
}

// ---------------------------------------------------------------------------
// A defect this suite caught, now fixed
// ---------------------------------------------------------------------------

/// `render_plans` computed `inner.width as usize - 8` for its rationale, risk
/// and fallback lines, so any terminal narrower than ten columns underflowed
/// and panicked before drawing anything. Every other width computation in the
/// file used `saturating_sub`; these three did not.
///
/// The test was written first and left `#[ignore]`d rather than weakened, which
/// is why the defect was fixed instead of absorbed: a test that asserts nothing
/// because the code cannot run is not a test.
#[test]
fn the_plans_tab_survives_a_ten_column_terminal() {
    for width in [10u16, 9, 8] {
        let out = run(width, 20, "plans");
        assert!(
            out.status.success(),
            "`--tab plans` at {width}x20 exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim_end()
        );
    }
}
