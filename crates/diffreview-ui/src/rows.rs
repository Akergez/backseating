//! What the diff pane draws for one file, one row each: the file-level
//! note, the lines, and each line's note under it. Kept apart from drawing
//! so the moves around it can be tested.

use diffreview::annotation::{Anchor, Store};
use diffreview::{FileDiff, LineKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// The annotation on the whole file.
    FileNote,
    /// A line of the diff, by index into [`FileDiff::lines`].
    Line(usize),
    /// The annotation on the line with this index.
    Note(usize),
}

pub fn rows(file: &FileDiff, store: &Store) -> Vec<Row> {
    let mut rows = Vec::with_capacity(file.lines.len() + 1);
    if store.get(&file.path, Anchor::File).is_some() {
        rows.push(Row::FileNote);
    }
    for (index, line) in file.lines.iter().enumerate() {
        rows.push(Row::Line(index));
        if Anchor::of(line).is_some_and(|anchor| store.get(&file.path, anchor).is_some()) {
            rows.push(Row::Note(index));
        }
    }
    rows
}

/// The text a note row shows: its first line, and a mark when there is more.
pub fn note_text(row: Row, file: &FileDiff, store: &Store) -> Option<String> {
    let (prefix, annotation) = match row {
        Row::FileNote => ("› file: ", store.get(&file.path, Anchor::File)),
        Row::Note(line) => (
            "› ",
            Anchor::of(&file.lines[line]).and_then(|anchor| store.get(&file.path, anchor)),
        ),
        Row::Line(_) => return None,
    };
    let text = annotation
        .map(|annotation| annotation.comment.as_str())
        .unwrap_or_default();
    let first = text.lines().next().unwrap_or_default();
    let more = if text.lines().count() > 1 { " …" } else { "" };
    Some(format!("{prefix}{first}{more}"))
}

/// The row with the most characters, tabs counted as drawn: the diff pane
/// measures it for how far it scrolls sideways.
pub fn widest(file: &FileDiff, store: &Store, rows: &[Row]) -> usize {
    let width = |row: Row| match row {
        Row::Line(index) => {
            let text = &file.lines[index].text;
            text.chars().count() + 3 * text.matches('\t').count()
        }
        _ => note_text(row, file, store).map_or(0, |text| text.chars().count()),
    };
    rows.iter()
        .enumerate()
        .max_by_key(|&(at, row)| (width(*row), std::cmp::Reverse(at)))
        .map_or(0, |(at, _)| at)
}

/// The row a hunk move lands on: the first changed line of the next (or
/// previous) run of changes after `from`.
pub fn hunk_row(file: &FileDiff, rows: &[Row], from: usize, forward: bool) -> Option<usize> {
    let starts = hunk_starts(file, rows);
    if forward {
        starts.into_iter().find(|&at| at > from)
    } else {
        starts.into_iter().rev().find(|&at| at < from)
    }
}

/// Every row that starts a run of changes: a changed line whose line before
/// it (notes aside) is unchanged, or that has none before it.
fn hunk_starts(file: &FileDiff, rows: &[Row]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut previous_changed = false;
    for (at, row) in rows.iter().enumerate() {
        let Row::Line(index) = row else { continue };
        let changed = matches!(file.lines[*index].kind, LineKind::Add | LineKind::Remove);
        if changed && !previous_changed {
            starts.push(at);
        }
        previous_changed = changed;
    }
    starts
}

/// The first row worth putting the cursor on: the first change.
pub fn first_change(file: &FileDiff, rows: &[Row]) -> usize {
    hunk_starts(file, rows).first().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diffreview::annotation::{Annotation, ChangeType};

    fn file() -> FileDiff {
        diffreview::diff::parse(
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,6 +1,6 @@\n a\n-b\n+B\n c\n d\n-e\n+E\n",
        )
        .remove(0)
    }

    #[test]
    fn notes_follow_their_lines() {
        let file = file();
        let mut store = Store::default();
        store.add(Annotation {
            file: "x".into(),
            anchor: Anchor::Line {
                line: 2,
                change: ChangeType::Add,
            },
            end_line: None,
            comment: "c".into(),
        });
        store.add(Annotation {
            file: "x".into(),
            anchor: Anchor::File,
            end_line: None,
            comment: "f".into(),
        });
        let rows = rows(&file, &store);
        assert_eq!(
            &rows[..5],
            &[
                Row::FileNote,
                Row::Line(0),
                Row::Line(1),
                Row::Line(2),
                Row::Line(3)
            ]
        );
        assert_eq!(rows[5], Row::Note(3));
    }

    #[test]
    fn widest_row_counts_tabs_and_notes() {
        let file = diffreview::diff::parse(
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n abcdef\n-\t\tx\n+y\n",
        )
        .remove(0);
        let mut store = Store::default();
        let rows_without = rows(&file, &store);
        // 0 hunk header, 1 abcdef, 2 -\t\tx (nine columns), 3 +y
        assert_eq!(widest(&file, &store, &rows_without), 0);
        store.add(Annotation {
            file: "x".into(),
            anchor: Anchor::Line {
                line: 2,
                change: ChangeType::Add,
            },
            end_line: None,
            comment: "a note longer than any line of this little diff".into(),
        });
        let rows = rows(&file, &store);
        assert_eq!(rows[widest(&file, &store, &rows)], Row::Note(3));
    }

    #[test]
    fn hunk_moves_land_on_the_start_of_each_run() {
        let file = file();
        let rows = rows(&file, &Store::default());
        // 0 hunk header, 1 a, 2 -b, 3 +B, 4 c, 5 d, 6 -e, 7 +E
        assert_eq!(hunk_row(&file, &rows, 0, true), Some(2));
        assert_eq!(hunk_row(&file, &rows, 2, true), Some(6));
        assert_eq!(hunk_row(&file, &rows, 6, true), None);
        assert_eq!(hunk_row(&file, &rows, 7, false), Some(6));
        assert_eq!(first_change(&file, &rows), 2);
    }
}
