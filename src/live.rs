//! Pure logic for the Live editing mode: splits a note into segments (one per block), works out
//! which ones show as rendered blocks and which as raw text, maps edits and cursor positions, and
//! keeps undo history. All offsets are UTF-8 byte offsets into the note body.

use std::ops::Range;

use crate::markdown::Block;

/// A run of whole source lines shown as one unit: from one block's first line up to the next block's.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: usize,
    /// End of the last line with visible text (before its `\n`). The blank gap runs from here to `end`.
    pub content_end: usize,
    pub end: usize,
    /// Indices into the `blocks` slice that start on a line of this segment.
    pub blocks: Vec<usize>,
}

/// Byte offset of every line start. Lines split at `\n` only, so a `\r` stays inside its line.
pub fn line_starts(body: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, b) in body.bytes().enumerate() {
        if b == b'\n' && i + 1 < body.len() {
            starts.push(i + 1);
        }
    }
    starts
}

/// Splits the body into segments. Each block line opens a segment; lines without a block (HTML
/// comments, link definitions) stay with the segment before them. Blocks on lines past the end of
/// the body are ignored.
pub fn segments(body: &str, blocks: &[Block]) -> Vec<Segment> {
    let starts = line_starts(body);
    // Line 0 always opens a segment, so leading blank lines have one of their own.
    let mut lines: Vec<usize> = blocks.iter().map(|b| b.line).filter(|&l| l < starts.len()).collect();
    lines.push(0);
    lines.sort_unstable();
    lines.dedup();
    let mut owned: Vec<Vec<usize>> = vec![Vec::new(); lines.len()];
    for (i, block) in blocks.iter().enumerate() {
        if let Ok(k) = lines.binary_search(&block.line) {
            owned[k].push(i);
        }
    }
    lines
        .iter()
        .enumerate()
        .map(|(k, &line)| {
            let start = starts[line];
            let end = lines.get(k + 1).map_or(body.len(), |&next| starts[next]);
            Segment { start, content_end: content_end(body, start, end), end, blocks: std::mem::take(&mut owned[k]) }
        })
        .collect()
}

/// End of the last line in `start..end` that has visible text, or `start` if the range is all blank.
fn content_end(body: &str, start: usize, end: usize) -> usize {
    let mut result = start;
    let mut line = start;
    while line < end {
        let (text_end, next) = match body[line..end].find('\n') {
            Some(i) => (line + i, line + i + 1),
            None => (end, end),
        };
        if !body[line..text_end].trim().is_empty() {
            result = text_end;
        }
        line = next;
    }
    result
}

/// Index of the segment that holds `offset` (`start <= offset < end`). The end of the body maps to
/// the last segment, so a cursor at the very end still has a home.
pub fn segment_at(segs: &[Segment], offset: usize) -> usize {
    segs.partition_point(|s| s.start <= offset).saturating_sub(1)
}

/// The union of all segments touching the range between `a` and `b`.
pub fn cover(segs: &[Segment], a: usize, b: usize) -> Range<usize> {
    let i = segment_at(segs, a.min(b));
    let j = segment_at(segs, a.max(b));
    segs[i].start..segs[j].end
}

/// One replacement: `removed` at byte `at` became `inserted`.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub at: usize,
    pub removed: String,
    pub inserted: String,
}

/// The single edit that turns `old` into `new`: keeps the common start and end, on char boundaries.
pub fn diff(old: &str, new: &str) -> Edit {
    let mut prefix = 0;
    for (a, b) in old.chars().zip(new.chars()) {
        if a != b {
            break;
        }
        prefix += a.len_utf8();
    }
    // The common end must not overlap the common start.
    let limit = old.len().min(new.len()) - prefix;
    let mut suffix = 0;
    for (a, b) in old[prefix..].chars().rev().zip(new[prefix..].chars().rev()) {
        if a != b || suffix + a.len_utf8() > limit {
            break;
        }
        suffix += a.len_utf8();
    }
    Edit {
        at: prefix,
        removed: old[prefix..old.len() - suffix].to_string(),
        inserted: new[prefix..new.len() - suffix].to_string(),
    }
}

/// Where an offset moves after `edit`. Offsets inside the removed text go to the end of the insert.
pub fn map_offset(off: usize, edit: &Edit) -> usize {
    let removed_end = edit.at + edit.removed.len();
    if off <= edit.at {
        off
    } else if off < removed_end {
        edit.at + edit.inserted.len()
    } else {
        off - edit.removed.len() + edit.inserted.len()
    }
}

/// Where a range moves after `edit`. The result always has start <= end.
pub fn map_range(r: Range<usize>, edit: &Edit) -> Range<usize> {
    let start = map_offset(r.start, edit);
    let end = map_offset(r.end, edit).max(start);
    start..end
}

/// Replaces `body[active]` with `text`. None if the range is backwards, past the end, or not on char boundaries.
pub fn splice(body: &str, active: Range<usize>, text: &str) -> Option<String> {
    if active.start > active.end || !body.is_char_boundary(active.start) || !body.is_char_boundary(active.end) {
        return None;
    }
    Some(format!("{}{}{}", &body[..active.start], text, &body[active.end..]))
}

/// What typing in the raw part does: the new body, the raw part's new byte range, and the cursor's
/// byte offset in the new body. `local_cursor` is the cursor inside `text`. None if the range or the
/// cursor doesn't fit the body or text, so a bad edit is never applied.
pub fn apply_typed(body: &str, active: Range<usize>, text: &str, local_cursor: usize) -> Option<(String, Range<usize>, usize)> {
    if local_cursor > text.len() || !text.is_char_boundary(local_cursor) {
        return None;
    }
    let new_body = splice(body, active.clone(), text)?;
    let start = active.start;
    Some((new_body, start..start + text.len(), start + local_cursor))
}

/// One thing to show in the Live view: a block from the parsed note, or raw text.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Block(usize),
    Raw(Range<usize>),
}

/// Rows above and below the `active` range (the part being edited as raw text).
/// A segment that crosses the edge of `active` contributes its outside part as raw text, so no text disappears.
/// Preconditions: `active` lies within the body and on char boundaries.
pub fn layout(body: &str, blocks: &[Block], active: Range<usize>) -> (Vec<Row>, Vec<Row>) {
    let mut before = Vec::new();
    let mut after = Vec::new();
    for seg in segments(body, blocks) {
        if seg.end <= active.start {
            push_rows(body, &seg, &mut before);
        } else if seg.start >= active.end {
            push_rows(body, &seg, &mut after);
        } else {
            if seg.start < active.start {
                push_raw(body, seg.start..active.start, &mut before);
            }
            if seg.end > active.end {
                push_raw(body, active.end..seg.end, &mut after);
            }
        }
    }
    (before, after)
}

fn push_rows(body: &str, seg: &Segment, out: &mut Vec<Row>) {
    if seg.blocks.is_empty() {
        push_raw(body, seg.start..seg.end, out);
    } else {
        out.extend(seg.blocks.iter().map(|&i| Row::Block(i)));
    }
}

/// Adds a raw row unless the text is only whitespace, which would show as an empty gap.
fn push_raw(body: &str, r: Range<usize>, out: &mut Vec<Row>) {
    if !body[r.clone()].trim().is_empty() {
        out.push(Row::Raw(r));
    }
}

/// Replaces `expect` at `at` with `with`, but only if `expect` is really there.
fn replace_checked(body: &str, at: usize, expect: &str, with: &str) -> Option<String> {
    let end = at.checked_add(expect.len())?;
    if body.get(at..end)? != expect {
        return None;
    }
    Some(format!("{}{}{}", &body[..at], with, &body[end..]))
}

/// Where the cursor goes when moving up or down one segment, keeping its column.
/// Moving up lands on the last line with text of the previous segment; moving down on the first line
/// of the next one. The column counts chars and is clamped to the line length (without `\r`).
/// Returns None at the first or last segment.
pub fn vertical_target(body: &str, segs: &[Segment], cursor: usize, up: bool) -> Option<usize> {
    let current = segment_at(segs, cursor);
    let target = if up { current.checked_sub(1)? } else { current + 1 };
    let seg = segs.get(target)?;
    let line = if up { last_text_line(body, seg) } else { seg.start };
    Some(offset_at_column(body, line, column(body, cursor)))
}

/// Start of the last line of `seg` with visible text, or the first line if the segment is all blank.
fn last_text_line(body: &str, seg: &Segment) -> usize {
    if seg.content_end == seg.start {
        seg.start
    } else {
        body[..seg.content_end].rfind('\n').map_or(0, |i| i + 1)
    }
}

/// Number of chars between the start of the cursor's line and the cursor.
fn column(body: &str, cursor: usize) -> usize {
    let line = body[..cursor].rfind('\n').map_or(0, |i| i + 1);
    body[line..cursor].chars().count()
}

/// Byte offset of char `col` on the line starting at `line`, or the line's end if it is shorter.
fn offset_at_column(body: &str, line: usize, col: usize) -> usize {
    let line_end = body[line..].find('\n').map_or(body.len(), |i| line + i);
    let text = &body[line..line_end];
    let text = text.strip_suffix('\r').unwrap_or(text);
    text.char_indices().nth(col).map_or(line + text.len(), |(i, _)| line + i)
}

/// Longest gap between two typed keys that still merge into one undo step.
const MERGE_MS: i64 = 1000;
/// Most undo steps kept.
const MAX_STEPS: usize = 500;
/// Most text (removed plus inserted bytes) kept across undo steps.
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// One undoable change, with the cursor position before and after it.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub edit: Edit,
    pub cursor_before: usize,
    pub cursor_after: usize,
    /// True for plain typing and backspace, which can merge with the step before.
    pub typed: bool,
    pub at_ms: i64,
}

/// Undo and redo stacks. Live mode keeps its own history because the editor's undo can't see the
/// segments being rebuilt.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Step>,
    /// Undone steps, each with the body length it was undone to. Inserts have no removed text to
    /// check, so the length is the only way to notice that the note changed before a redo.
    redo: Vec<(Step, usize)>,
}

/// The note text changed outside the editor, so the stored steps no longer match it and were dropped.
#[derive(Debug, PartialEq, Eq)]
pub struct Unchanged;

impl History {
    /// Adds a new change and clears redo. Consecutive typing (within a second of each other, one line)
    /// merges into one step, so undo removes a word rather than a letter. Backspace over the text just
    /// typed also merges.
    pub fn record(&mut self, step: Step) {
        self.redo.clear();
        if let Some(top) = self.undo.last_mut() {
            if can_merge(top, &step) {
                merge(top, step);
                self.trim();
                return;
            }
        }
        self.undo.push(step);
        self.trim();
    }

    /// Undoes the last change. Returns the new body and the cursor position from before that change.
    /// `Err(Unchanged)` if the body no longer has the text the change left behind.
    pub fn undo(&mut self, body: &str) -> Result<Option<(String, usize)>, Unchanged> {
        let Some(step) = self.undo.pop() else { return Ok(None) };
        let Some(text) = replace_checked(body, step.edit.at, &step.edit.inserted, &step.edit.removed) else {
            self.clear();
            return Err(Unchanged);
        };
        let cursor = step.cursor_before;
        self.redo.push((step, text.len()));
        Ok(Some((text, cursor)))
    }

    /// Redoes the last undone change. Returns the new body and the cursor position from after that change.
    /// `Err(Unchanged)` if the body no longer matches what undo left.
    pub fn redo(&mut self, body: &str) -> Result<Option<(String, usize)>, Unchanged> {
        let Some((step, len)) = self.redo.pop() else { return Ok(None) };
        let checked = (body.len() == len)
            .then(|| replace_checked(body, step.edit.at, &step.edit.removed, &step.edit.inserted))
            .flatten();
        let Some(text) = checked else {
            self.clear();
            return Err(Unchanged);
        };
        let cursor = step.cursor_after;
        self.undo.push(step);
        Ok(Some((text, cursor)))
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Drops the oldest steps until both limits hold.
    fn trim(&mut self) {
        if self.undo.len() > MAX_STEPS {
            self.undo.drain(..self.undo.len() - MAX_STEPS);
        }
        let mut bytes: usize = self.undo.iter().map(step_bytes).sum();
        while bytes > MAX_BYTES && !self.undo.is_empty() {
            bytes -= step_bytes(&self.undo.remove(0));
        }
    }
}

fn step_bytes(step: &Step) -> usize {
    step.edit.removed.len() + step.edit.inserted.len()
}

/// Whether `new` continues the typing in `top`: more text typed right after it, or backspace over
/// the last character typed. Newlines never merge, so each line is its own undo step.
fn can_merge(top: &Step, new: &Step) -> bool {
    if !top.typed || !new.typed || !(0..=MERGE_MS).contains(&(new.at_ms - top.at_ms)) {
        return false;
    }
    let has_newline = [&top.edit.removed, &top.edit.inserted, &new.edit.removed, &new.edit.inserted]
        .iter()
        .any(|text| text.contains('\n'));
    if has_newline || top.edit.inserted.is_empty() {
        return false;
    }
    let typed_end = top.edit.at + top.edit.inserted.len();
    let typing = new.edit.removed.is_empty() && !new.edit.inserted.is_empty() && new.edit.at == typed_end;
    let backspace = new.edit.inserted.is_empty()
        && new.edit.removed.chars().count() == 1
        && top.edit.inserted.ends_with(&new.edit.removed)
        && new.edit.at + new.edit.removed.len() == typed_end;
    typing || backspace
}

/// Folds `new` into `top`. The merged step keeps the first cursor position and the latest time,
/// so a long burst of typing stays one step while each key is within a second of the last.
fn merge(top: &mut Step, new: Step) {
    if new.edit.inserted.is_empty() {
        let keep = top.edit.inserted.len() - new.edit.removed.len();
        top.edit.inserted.truncate(keep);
    } else {
        top.edit.inserted.push_str(&new.edit.inserted);
    }
    top.cursor_after = new.cursor_after;
    top.at_ms = new.at_ms;
}

/// What a key does at the edge of the raw part. The caller only asks at those edges.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyAction {
    /// The caret goes here, with no selection.
    Move(usize),
    /// The selection goes from `anchor` to `cursor`.
    Select { anchor: usize, cursor: usize },
    /// These bytes of the body are removed.
    Delete(Range<usize>),
}

/// What `key` does when the raw part is `active` and the selection is `anchor..cursor`.
/// `None` means the text box should handle the key itself. Keys are the names Slint sends
/// (see `live-key` in `ui/live.slint`).
pub fn key_target(body: &str, segs: &[Segment], active: Range<usize>, cursor: usize, anchor: usize, key: &str) -> Option<KeyAction> {
    let plain = anchor == cursor;
    match key {
        // Backspace at the top of the raw part joins it with the block before (the `\n` between them).
        "backspace" => {
            if !plain || cursor != active.start {
                return None;
            }
            let ch = body[..cursor].chars().next_back()?;
            Some(KeyAction::Delete(cursor - ch.len_utf8()..cursor))
        }
        "delete" => {
            if !plain || cursor != active.end {
                return None;
            }
            let ch = body[cursor..].chars().next()?;
            Some(KeyAction::Delete(cursor..cursor + ch.len_utf8()))
        }
        "left" => {
            if !plain || cursor != active.start || active.start == 0 {
                return None;
            }
            let prev = &segs[segment_at(segs, active.start - 1)];
            Some(KeyAction::Move(prev.content_end))
        }
        "right" => {
            if !plain || cursor != active.end || active.end >= body.len() {
                return None;
            }
            Some(KeyAction::Move(segs[segment_at(segs, active.end)].start))
        }
        "up" => vertical_target(body, segs, cursor, true).map(KeyAction::Move),
        "down" => vertical_target(body, segs, cursor, false).map(KeyAction::Move),
        // The selection grows by one block at the raw part's edge, so it always covers whole blocks.
        // The end lands on the last visible character, which keeps the cursor inside that block.
        "select-up" => {
            let first = segment_at(segs, active.start);
            (first > 0).then(|| KeyAction::Select { anchor, cursor: segs[first - 1].start })
        }
        "select-down" => {
            if active.end >= body.len() {
                return None;
            }
            let next = &segs[segment_at(segs, active.end)];
            Some(KeyAction::Select { anchor, cursor: next.content_end })
        }
        "doc-start" => Some(KeyAction::Move(0)),
        "doc-end" => Some(KeyAction::Move(body.len())),
        "select-all" => Some(KeyAction::Select { anchor: 0, cursor: body.len() }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{parse, Kind};

    /// Bodies that exercise every kind of Markdown the Live view has to split.
    const FIXTURES: &[&str] = &[
        "",
        "just text with no newline",
        "first line\nsecond line",
        "# Title\r\n\r\nPara one\r\nstill one\r\n\r\nPara two\r\n",
        "héllo 🎉 wörld\n\n日本語のテキスト\n\n€100 ok\n",
        "See [[Book ideas]] and [[Other]].\n\nAnother [[x]] line\n",
        "Intro\n\n```rust\nfn a() {}\n\n\nfn b() {}\n```\n\nAfter\n",
        "Start\n\n```\ncode line\n\nmore\n",
        "Para\n\n    indented code\n    more\n\nEnd\n",
        "| a | b |\n| - | - |\n| 1 | 2 |\n\nafter\n",
        "- one\n  - nested\n    - deeper\n- two\n\n1. first\n2. second\n",
        "- ```\ncode\n```\n",
        "- # h\n- next\n",
        "> [!tip] x\n> more\n>\n> para\n",
        "Text[^1] here.\n\n[^1]: The note.\n",
        "Title\n=====\n\nBody text\n",
        "---\ntitle: Hello\n---\n\nBody\n",
        "First para\n\n<!-- hidden note -->\n\nSecond para\n",
        "See [a].\n\n[a]: https://x\n",
        "\n\n   \n\n",
        "\n\n\nHello\n\nWorld",
        "Title\n\n- a\n- [ ] b\n\n```\nx\n```\n\n| h |\n|---|\n| v |\n",
    ];

    /// Short strings with multi-byte chars and shared UTF-8 continuation bytes, for diff tests.
    const EXTRA: &[&str] = &[
        "", "a", "héllo", "hello", "€", "¬", "a€b", "a¬b", "日本語", "日本", "x\ny", "x\r\ny", "🎉🎉", "🎉", "ab", "abab",
    ];

    fn boundaries(body: &str) -> Vec<usize> {
        (0..=body.len()).filter(|&i| body.is_char_boundary(i)).collect()
    }

    /// Checks the rules every segmentation must follow and returns the segments.
    fn check_partition(body: &str, blocks: &[Block]) -> Vec<Segment> {
        let segs = segments(body, blocks);
        assert!(!segs.is_empty(), "{body:?}");
        assert_eq!(segs[0].start, 0, "{body:?}");
        assert_eq!(segs.last().unwrap().end, body.len(), "{body:?}");
        let starts = line_starts(body);
        for (i, s) in segs.iter().enumerate() {
            assert!(body.is_char_boundary(s.start) && body.is_char_boundary(s.end), "{body:?}");
            assert!(starts.contains(&s.start), "segment start is not a line start: {body:?}");
            assert!(s.end == body.len() || body.as_bytes()[s.end - 1] == b'\n', "{body:?}");
            assert!(s.start <= s.content_end && s.content_end <= s.end, "{body:?}");
            assert!(body[s.content_end..s.end].trim().is_empty(), "blank gap has text: {body:?}");
            if let Some(next) = segs.get(i + 1) {
                assert_eq!(s.end, next.start, "{body:?}");
            }
        }
        segs
    }

    fn line_index(body: &str, offset: usize) -> usize {
        body[..offset].matches('\n').count()
    }

    #[test]
    fn segments_partition_every_fixture() {
        for body in FIXTURES {
            check_partition(body, &parse(body));
        }
    }

    #[test]
    fn every_block_lands_in_one_segment_on_its_line() {
        for body in FIXTURES {
            let blocks = parse(body);
            let segs = check_partition(body, &blocks);
            let mut seen = vec![0; blocks.len()];
            for seg in &segs {
                for &i in &seg.blocks {
                    seen[i] += 1;
                    let line = blocks[i].line;
                    let first = line_index(body, seg.start);
                    let past = if seg.end == body.len() { usize::MAX } else { line_index(body, seg.end) };
                    assert!(first <= line && line < past, "block {i} on line {line} outside its segment: {body:?}");
                }
            }
            assert!(seen.iter().all(|&n| n == 1), "every block must be in exactly one segment: {body:?}");
        }
    }

    #[test]
    fn splicing_any_segment_range_gives_the_same_body() {
        for body in FIXTURES {
            let segs = check_partition(body, &parse(body));
            for i in 0..segs.len() {
                for j in i..segs.len() {
                    let r = segs[i].start..segs[j].end;
                    let text = body[r.clone()].to_string();
                    assert_eq!(splice(body, r, &text).as_deref(), Some(*body), "{body:?}");
                }
            }
        }
    }

    #[test]
    fn every_char_boundary_maps_to_a_segment() {
        for body in FIXTURES {
            let blocks = parse(body);
            let segs = check_partition(body, &blocks);
            for c in boundaries(body) {
                let cov = cover(&segs, c, c);
                assert!(cov.start <= c && c <= cov.end, "cover of {c} misses it: {body:?}");
                let i = segment_at(&segs, c);
                assert!(segs[i].start <= c && (c < segs[i].end || c == body.len()), "{body:?}");
            }
        }
    }

    #[test]
    fn layout_shows_every_segment_once() {
        for body in FIXTURES {
            let blocks = parse(body);
            let segs = check_partition(body, &blocks);
            let points = boundaries(body);
            // Single-point actives (whole segments) and ranges that cut through segments.
            let mut actives: Vec<Range<usize>> = points.iter().map(|&c| cover(&segs, c, c)).collect();
            for pair in points.windows(6) {
                actives.push(pair[0]..pair[5]);
            }
            for active in actives {
                let (before, after) = layout(body, &blocks, active.clone());
                let rows: Vec<&Row> = before.iter().chain(after.iter()).collect();
                for seg in &segs {
                    if seg.start >= active.start && seg.end <= active.end {
                        continue;
                    }
                    let outside: Vec<Range<usize>> = if seg.end <= active.start || seg.start >= active.end {
                        vec![seg.start..seg.end]
                    } else {
                        let mut parts = Vec::new();
                        if seg.start < active.start {
                            parts.push(seg.start..active.start);
                        }
                        if seg.end > active.end {
                            parts.push(active.end..seg.end);
                        }
                        parts
                    };
                    let whole_outside = seg.end <= active.start || seg.start >= active.end;
                    if whole_outside && !seg.blocks.is_empty() {
                        for &i in &seg.blocks {
                            assert!(rows.contains(&&Row::Block(i)), "block {i} missing: {body:?} {active:?}");
                        }
                        continue;
                    }
                    for part in outside {
                        if body[part.clone()].trim().is_empty() {
                            continue;
                        }
                        assert!(
                            rows.iter().any(|r| matches!(r, Row::Raw(r) if r.start <= part.start && part.end <= r.end)),
                            "text {part:?} not shown: {body:?} {active:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn diff_and_splice_round_trip() {
        let corpus: Vec<&str> = FIXTURES.iter().chain(EXTRA.iter()).copied().collect();
        for a in &corpus {
            for b in &corpus {
                let edit = diff(a, b);
                let at = edit.at..edit.at + edit.removed.len();
                assert_eq!(splice(a, at, &edit.inserted).as_deref(), Some(*b), "{a:?} -> {b:?}");
            }
        }
    }

    #[test]
    fn map_range_stays_in_bounds_and_on_boundaries() {
        for a in EXTRA {
            for b in EXTRA {
                let edit = diff(a, b);
                for &s in &boundaries(a) {
                    for &e in &boundaries(a) {
                        if s > e {
                            continue;
                        }
                        let r = map_range(s..e, &edit);
                        assert!(r.start <= r.end && r.end <= b.len(), "{a:?} {b:?} {s}..{e}");
                        assert!(b.is_char_boundary(r.start) && b.is_char_boundary(r.end), "{a:?} {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn diff_keeps_common_text_and_respects_chars() {
        assert_eq!(diff("abc", "abXc"), Edit { at: 2, removed: String::new(), inserted: "X".into() });
        assert_eq!(diff("héllo", "hello"), Edit { at: 1, removed: "é".into(), inserted: "e".into() });
        // Both chars end in the same byte (0xAC), but they are different chars.
        assert_eq!(diff("€", "¬"), Edit { at: 0, removed: "€".into(), inserted: "¬".into() });
        assert_eq!(diff("same", "same"), Edit { at: 4, removed: String::new(), inserted: String::new() });
        assert_eq!(diff("", "x"), Edit { at: 0, removed: String::new(), inserted: "x".into() });
    }

    #[test]
    fn splice_rejects_bad_ranges() {
        assert_eq!(splice("é", 1..2, "x"), None);
        assert_eq!(splice("abc", 0..10, "x"), None);
        assert_eq!(splice("abc", 2..1, "x"), None);
        assert_eq!(splice("abc", 1..2, "X"), Some("aXc".into()));
    }

    #[test]
    fn three_paragraphs_make_three_segments() {
        let body = "one\n\ntwo\n\nthree";
        let segs = check_partition(body, &parse(body));
        assert_eq!(
            segs,
            vec![
                Segment { start: 0, content_end: 3, end: 5, blocks: vec![0] },
                Segment { start: 5, content_end: 8, end: 10, blocks: vec![1] },
                Segment { start: 10, content_end: 15, end: 15, blocks: vec![2] },
            ]
        );
    }

    #[test]
    fn leading_blank_lines_belong_to_first_segment() {
        let body = "\n\nHello";
        let segs = check_partition(body, &parse(body));
        assert_eq!(
            segs,
            vec![
                Segment { start: 0, content_end: 0, end: 2, blocks: vec![] },
                Segment { start: 2, content_end: 7, end: 7, blocks: vec![0] },
            ]
        );
    }

    #[test]
    fn empty_and_blank_bodies_are_one_segment() {
        assert_eq!(segments("", &[]), vec![Segment { start: 0, content_end: 0, end: 0, blocks: vec![] }]);
        let blank = "\n  \n\n";
        assert_eq!(segments(blank, &parse(blank)), vec![Segment { start: 0, content_end: 0, end: 5, blocks: vec![] }]);
    }

    #[test]
    fn list_item_with_fence_splits_into_item_and_code() {
        let body = "- ```\ncode\n```\n";
        let blocks = parse(body);
        let segs = check_partition(body, &blocks);
        // The parser also reports the fence's text as a paragraph on line 1, so there are three segments.
        let kinds: Vec<Vec<Kind>> = segs.iter().map(|s| s.blocks.iter().map(|&i| blocks[i].kind).collect()).collect();
        assert_eq!(kinds, vec![vec![Kind::Code, Kind::Item], vec![Kind::Paragraph], vec![Kind::Code]]);
    }

    #[test]
    fn two_blocks_on_one_line_share_a_segment() {
        let body = "- # h";
        let blocks = parse(body);
        let segs = check_partition(body, &blocks);
        assert_eq!(blocks.len(), 2);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].blocks, vec![0, 1]);
    }

    #[test]
    fn html_comment_belongs_to_the_segment_before_it() {
        let body = "Intro para\n\n<!-- hidden -->\n\nSecond\n";
        let blocks = parse(body);
        let segs = check_partition(body, &blocks);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], Segment { start: 0, content_end: 27, end: 29, blocks: vec![0] });
    }

    #[test]
    fn layout_splits_rows_around_the_active_segment() {
        let body = "one\n\ntwo\n\nthree";
        let blocks = parse(body);
        let segs = check_partition(body, &blocks);
        let (before, after) = layout(body, &blocks, cover(&segs, 6, 6));
        assert_eq!(before, vec![Row::Block(0)]);
        assert_eq!(after, vec![Row::Block(2)]);
    }

    #[test]
    fn text_without_blocks_stays_as_raw_rows() {
        let body = "<!-- only -->\n\nText";
        let blocks = parse(body);
        let (before, after) = layout(body, &blocks, 15..19);
        assert_eq!(before, vec![Row::Raw(0..15)]);
        assert!(after.is_empty());
    }

    #[test]
    fn whitespace_only_rows_are_skipped() {
        let body = "\n\n  \nHello";
        let blocks = parse(body);
        let (before, after) = layout(body, &blocks, 5..10);
        assert!(before.is_empty());
        assert!(after.is_empty());
    }

    #[test]
    fn crossing_segment_keeps_its_outside_text_raw() {
        let body = "one\n\ntwo\n\nthree";
        let blocks = parse(body);
        // Active covers the middle of "two" only, so "t" and "wo" stay visible as raw text.
        let (before, after) = layout(body, &blocks, 6..7);
        assert_eq!(before, vec![Row::Block(0), Row::Raw(5..6)]);
        assert_eq!(after, vec![Row::Raw(7..10), Row::Block(2)]);
    }

    #[test]
    fn typing_in_the_raw_part_updates_the_body_and_range() {
        // "a\n\n# T\n\nb": the raw part is "# T" (bytes 3..6).
        let (body, active, cursor) = apply_typed("a\n\n# T\n\nb", 3..6, "## Tx", 5).unwrap();
        assert_eq!(body, "a\n\n## Tx\n\nb");
        assert_eq!(active, 3..8);
        assert_eq!(cursor, 8);
    }

    #[test]
    fn typing_outside_the_range_is_refused() {
        // The cursor must sit inside the text just typed, on a character boundary.
        assert_eq!(apply_typed("abc", 0..3, "abc", 4), None);
        assert_eq!(apply_typed("é", 0..2, "é", 1), None);
        // The range must lie inside the body and on character boundaries.
        assert_eq!(apply_typed("abc", 1..9, "x", 1), None);
        assert_eq!(apply_typed("é", 0..1, "x", 1), None);
        // A backwards range is refused.
        assert_eq!(apply_typed("abc", 2..1, "x", 1), None);
    }

    #[test]
    fn typing_into_an_empty_note() {
        assert_eq!(apply_typed("", 0..0, "hi", 2), Some(("hi".to_string(), 0..2, 2)));
    }

    #[test]
    fn typing_at_the_very_end_keeps_the_rest() {
        assert_eq!(apply_typed("abc", 3..3, "d", 1), Some(("abcd".to_string(), 3..4, 4)));
    }

    #[test]
    fn deleting_the_whole_raw_part_leaves_an_empty_range() {
        assert_eq!(apply_typed("a\n\nb", 3..4, "", 0), Some(("a\n\n".to_string(), 3..3, 3)));
    }

    #[test]
    fn crlf_line_endings_are_kept() {
        // "a\r\nb\r\nc": the raw part is "b\r\n" (bytes 3..6).
        assert_eq!(apply_typed("a\r\nb\r\nc", 3..6, "x\r\n", 3), Some(("a\r\nx\r\nc".to_string(), 3..6, 6)));
    }

    #[test]
    fn multi_byte_text_counts_bytes() {
        assert_eq!(apply_typed("é\n\nb", 0..2, "éé", 4), Some(("éé\n\nb".to_string(), 0..4, 4)));
    }
    #[test]
    fn vertical_target_keeps_the_column() {
        let body = "one\n\ntwo\n\nthree";
        let segs = check_partition(body, &parse(body));
        assert_eq!(vertical_target(body, &segs, 6, true), Some(1));
        assert_eq!(vertical_target(body, &segs, 6, false), Some(11));
        assert_eq!(vertical_target(body, &segs, 0, true), None);
        assert_eq!(vertical_target(body, &segs, 12, false), None);
    }

    #[test]
    fn vertical_target_clamps_to_the_line_and_skips_cr() {
        let body = "ab\n\nxyz";
        let segs = check_partition(body, &parse(body));
        // Column 3 on "xyz" is past the end of "ab", so it lands on the end of "ab".
        assert_eq!(vertical_target(body, &segs, 7, true), Some(2));
        let crlf = "ab\r\n\r\nxyz";
        let segs = check_partition(crlf, &parse(crlf));
        assert_eq!(vertical_target(crlf, &segs, 9, true), Some(2));
    }

    #[test]
    fn vertical_target_none_for_single_segment() {
        let body = "hello";
        let segs = check_partition(body, &parse(body));
        assert_eq!(vertical_target(body, &segs, 2, true), None);
        assert_eq!(vertical_target(body, &segs, 2, false), None);
    }

    fn ins(at: usize, text: &str, before: usize, ms: i64) -> Step {
        Step {
            edit: Edit { at, removed: String::new(), inserted: text.to_string() },
            cursor_before: before,
            cursor_after: at + text.len(),
            typed: true,
            at_ms: ms,
        }
    }

    #[test]
    fn history_merges_typed_text_and_undoes_it() {
        let mut h = History::default();
        h.record(ins(3, "d", 3, 0));
        h.record(ins(4, "e", 4, 100));
        assert_eq!(h.undo.len(), 1);
        let (body, cursor) = h.undo("abcde").unwrap().unwrap();
        assert_eq!((body.as_str(), cursor), ("abc", 3));
    }

    #[test]
    fn undo_then_redo_restores_the_same_body() {
        let mut h = History::default();
        h.record(ins(1, "y", 1, 0));
        h.record(ins(2, "z", 2, 100));
        let (body, cursor) = h.undo("xyz").unwrap().unwrap();
        assert_eq!((body.as_str(), cursor), ("x", 1));
        let (body, cursor) = h.redo(&body).unwrap().unwrap();
        assert_eq!((body.as_str(), cursor), ("xyz", 3));
        assert_eq!(h.redo("xyz"), Ok(None));
    }

    #[test]
    fn backspace_over_typed_text_merges() {
        let mut h = History::default();
        h.record(ins(3, "d", 3, 0));
        let back = Step {
            edit: Edit { at: 3, removed: "d".into(), inserted: String::new() },
            cursor_before: 4,
            cursor_after: 3,
            typed: true,
            at_ms: 50,
        };
        h.record(back);
        assert_eq!(h.undo.len(), 1);
        assert_eq!(h.undo("abc").unwrap(), Some(("abc".into(), 3)));
    }

    #[test]
    fn merging_stops_at_newlines_and_unrelated_edits() {
        let mut h = History::default();
        h.record(ins(3, "d", 3, 0));
        h.record(ins(4, "\n", 4, 10));
        h.record(ins(5, "e", 5, 20));
        assert_eq!(h.undo.len(), 3);

        let mut h = History::default();
        h.record(ins(3, "d", 3, 0));
        h.record(ins(0, "x", 0, 10));
        assert_eq!(h.undo.len(), 2);
    }

    #[test]
    fn merging_stops_after_one_second() {
        let mut h = History::default();
        h.record(ins(3, "d", 3, 0));
        h.record(ins(4, "e", 4, 1000));
        assert_eq!(h.undo.len(), 1);
        h.record(ins(5, "f", 5, 2001));
        assert_eq!(h.undo.len(), 2);
    }

    #[test]
    fn untyped_steps_never_merge() {
        let mut h = History::default();
        let mut paste = ins(3, "d", 3, 0);
        paste.typed = false;
        h.record(paste);
        h.record(ins(4, "e", 4, 10));
        assert_eq!(h.undo.len(), 2);
    }

    #[test]
    fn undo_refuses_when_the_note_changed_outside() {
        let mut h = History::default();
        h.record(ins(3, "d", 3, 0));
        assert_eq!(h.undo("abX"), Err(Unchanged));
        assert_eq!(h.undo("abcd"), Ok(None));

        let mut h = History::default();
        h.record(ins(3, "d", 3, 0));
        h.undo("abcd").unwrap();
        // Same spot, different length: an insert can't be checked by its text, so the length catches it.
        assert_eq!(h.redo("abXYZ"), Err(Unchanged));
        assert_eq!(h.redo("abc"), Ok(None));
    }

    #[test]
    fn clear_drops_everything() {
        let mut h = History::default();
        h.record(ins(3, "d", 3, 0));
        h.clear();
        assert_eq!(h.undo("abcd"), Ok(None));
    }

    #[test]
    fn history_limits_steps_and_bytes() {
        let mut h = History::default();
        let mut body = String::new();
        for _ in 0..600 {
            let step = Step {
                edit: Edit { at: body.len(), removed: String::new(), inserted: "x".into() },
                cursor_before: body.len(),
                cursor_after: body.len() + 1,
                typed: false,
                at_ms: 0,
            };
            h.record(step);
            body.push('x');
        }
        assert_eq!(h.undo.len(), MAX_STEPS);

        let mut h = History::default();
        let big = "a".repeat(1_000_000);
        for i in 0..10 {
            h.record(Step {
                edit: Edit { at: 0, removed: String::new(), inserted: big.clone() },
                cursor_before: 0,
                cursor_after: 0,
                typed: false,
                at_ms: i,
            });
        }
        let stored: usize = h.undo.iter().map(step_bytes).sum();
        assert!(stored <= MAX_BYTES);
        assert_eq!(h.undo.len(), 8);
    }


    /// "one", "two" and "three" as three blocks, each with its blank line after it.
    const THREE: &str = "one\n\ntwo\n\nthree";

    fn key_at(body: &str, active: Range<usize>, cursor: usize, anchor: usize, key: &str) -> Option<KeyAction> {
        let segs = check_partition(body, &parse(body));
        key_target(body, &segs, active, cursor, anchor, key)
    }

    #[test]
    fn backspace_at_the_top_joins_the_block_before() {
        // The raw part is "two" (bytes 5..10). Backspace removes the "\n" before it.
        assert_eq!(key_at(THREE, 5..10, 5, 5, "backspace"), Some(KeyAction::Delete(4..5)));
    }

    #[test]
    fn backspace_inside_or_at_the_very_start_is_not_taken() {
        assert_eq!(key_at(THREE, 5..10, 6, 6, "backspace"), None);
        assert_eq!(key_at(THREE, 0..3, 0, 0, "backspace"), None);
        assert_eq!(key_at(THREE, 5..10, 5, 3, "backspace"), None);
        assert_eq!(key_at("", 0..0, 0, 0, "backspace"), None);
    }

    #[test]
    fn backspace_after_multi_byte_text_joins_the_block() {
        // "é" and "ü" are two bytes each. The "\n" before the block "ü" is removed.
        let body = "é\n\nü\n\nx";
        assert_eq!(key_at(body, 4..8, 4, 4, "backspace"), Some(KeyAction::Delete(3..4)));
    }

    #[test]
    fn delete_at_the_end_removes_the_char_after() {
        assert_eq!(key_at(THREE, 5..10, 10, 10, "delete"), Some(KeyAction::Delete(10..11)));
    }

    #[test]
    fn delete_removes_a_whole_multi_byte_char() {
        let body = "é\n\nü\n\nx";
        assert_eq!(key_at(body, 0..4, 4, 4, "delete"), Some(KeyAction::Delete(4..6)));
    }

    #[test]
    fn delete_not_at_the_end_or_at_the_end_of_the_note_is_not_taken() {
        assert_eq!(key_at(THREE, 5..10, 6, 6, "delete"), None);
        assert_eq!(key_at(THREE, 10..15, 15, 15, "delete"), None);
        assert_eq!(key_at("abc", 0..3, 3, 3, "delete"), None);
        assert_eq!(key_at("", 0..0, 0, 0, "delete"), None);
    }

    #[test]
    fn left_at_the_top_moves_to_the_end_of_the_block_before() {
        // The end of "one" (before its "\n"), not the blank line after it.
        assert_eq!(key_at(THREE, 5..10, 5, 5, "left"), Some(KeyAction::Move(3)));
    }

    #[test]
    fn left_in_the_first_block_or_inside_the_raw_part_is_not_taken() {
        assert_eq!(key_at(THREE, 0..3, 0, 0, "left"), None);
        assert_eq!(key_at(THREE, 5..10, 6, 6, "left"), None);
        assert_eq!(key_at(THREE, 5..10, 5, 3, "left"), None);
    }

    #[test]
    fn right_at_the_end_moves_to_the_start_of_the_next_block() {
        assert_eq!(key_at(THREE, 5..10, 10, 10, "right"), Some(KeyAction::Move(10)));
    }

    #[test]
    fn right_in_the_last_block_or_inside_the_raw_part_is_not_taken() {
        assert_eq!(key_at(THREE, 10..15, 15, 15, "right"), None);
        assert_eq!(key_at(THREE, 5..10, 9, 9, "right"), None);
        assert_eq!(key_at("", 0..0, 0, 0, "right"), None);
    }

    #[test]
    fn up_and_down_keep_the_column_across_blocks() {
        // Column 1 of "two" is byte 6. Up lands on column 1 of "one" (byte 1), down on column 1 of "three" (byte 11).
        assert_eq!(key_at(THREE, 5..10, 6, 6, "up"), Some(KeyAction::Move(1)));
        assert_eq!(key_at(THREE, 5..10, 6, 6, "down"), Some(KeyAction::Move(11)));
    }

    #[test]
    fn up_in_the_first_block_and_down_in_the_last_are_not_taken() {
        assert_eq!(key_at(THREE, 0..3, 1, 1, "up"), None);
        assert_eq!(key_at(THREE, 10..15, 11, 11, "down"), None);
        assert_eq!(key_at("", 0..0, 0, 0, "up"), None);
        assert_eq!(key_at("", 0..0, 0, 0, "down"), None);
    }

    #[test]
    fn select_up_takes_the_block_before_and_keeps_the_anchor() {
        assert_eq!(key_at(THREE, 5..10, 6, 6, "select-up"), Some(KeyAction::Select { anchor: 6, cursor: 0 }));
        assert_eq!(key_at(THREE, 0..3, 0, 0, "select-up"), None);
    }

    #[test]
    fn select_down_takes_the_block_after_and_keeps_the_anchor() {
        assert_eq!(key_at(THREE, 5..10, 6, 6, "select-down"), Some(KeyAction::Select { anchor: 6, cursor: 15 }));
        assert_eq!(key_at(THREE, 10..15, 11, 11, "select-down"), None);
        assert_eq!(key_at("", 0..0, 0, 0, "select-down"), None);
    }

    #[test]
    fn select_down_adds_one_block_to_an_existing_selection() {
        // "a\n\nb\n\nc\n\nd": "b" is raw and selected from 3. The selection grows to the end of "c".
        let body = "a\n\nb\n\nc\n\nd";
        assert_eq!(key_at(body, 3..6, 4, 3, "select-down"), Some(KeyAction::Select { anchor: 3, cursor: 7 }));
    }

    #[test]
    fn doc_start_doc_end_and_select_all() {
        assert_eq!(key_at(THREE, 5..10, 6, 6, "doc-start"), Some(KeyAction::Move(0)));
        assert_eq!(key_at(THREE, 5..10, 6, 6, "doc-end"), Some(KeyAction::Move(THREE.len())));
        assert_eq!(key_at(THREE, 5..10, 6, 6, "select-all"), Some(KeyAction::Select { anchor: 0, cursor: THREE.len() }));
        assert_eq!(key_at("", 0..0, 0, 0, "select-all"), Some(KeyAction::Select { anchor: 0, cursor: 0 }));
        assert_eq!(key_at("", 0..0, 0, 0, "doc-end"), Some(KeyAction::Move(0)));
    }

    #[test]
    fn unknown_keys_are_not_taken() {
        assert_eq!(key_at(THREE, 5..10, 6, 6, "enter"), None);
        assert_eq!(key_at(THREE, 5..10, 6, 6, ""), None);
    }

    #[test]
    fn crlf_blocks_join_and_skip_the_cr_on_the_way_up() {
        // "a\r\n\r\nb\r\n": "b" starts at byte 5. Backspace removes the "\n" before it.
        let body = "a\r\n\r\nb\r\n";
        assert_eq!(key_at(body, 5..8, 5, 5, "backspace"), Some(KeyAction::Delete(4..5)));
        // Up from the "b" line lands on column 0 of "a", without the "\r".
        assert_eq!(key_at(body, 5..8, 5, 5, "up"), Some(KeyAction::Move(0)));
    }

    #[test]
    fn list_items_are_separate_blocks_for_the_keys() {
        let body = "- one\n- two\n";
        // "- two" (bytes 6..12) is its own segment, and nothing comes after it.
        assert_eq!(key_at(body, 6..12, 6, 6, "select-down"), None);
        // Shift+Up from it takes "- one".
        assert_eq!(key_at(body, 6..12, 6, 6, "select-up"), Some(KeyAction::Select { anchor: 6, cursor: 0 }));
    }

    #[test]
    fn a_code_block_is_one_unit_for_the_keys() {
        let body = "Intro\n\n```\nx\n\ny\n```\n\nAfter\n";
        let blocks = parse(body);
        let segs = check_partition(body, &blocks);
        let code = segs.iter().position(|s| s.blocks.iter().any(|&i| blocks[i].kind == Kind::Code)).unwrap();
        let active = segs[code].start..segs[code].end;
        // Backspace at the top of the code block joins the text before it.
        assert_eq!(key_target(body, &segs, active.clone(), active.start, active.start, "backspace"), Some(KeyAction::Delete(active.start - 1..active.start)));
        // Right at its end moves to "After". The blank line inside the code block stays in it.
        assert_eq!(key_target(body, &segs, active.clone(), active.end, active.end, "right"), Some(KeyAction::Move(segs[code + 1].start)));
    }

    #[test]
    fn typed_edits_in_different_blocks_undo_one_at_a_time() {
        let bodies = ["one\n\ntwo\n\nthree", "oneX\n\ntwo\n\nthree", "oneX\n\ntwo\n\nthreeY", "oneX\n\ntwoZ\n\nthreeY"];
        let mut h = History::default();
        for (i, pair) in bodies.windows(2).enumerate() {
            // Five seconds apart, so none of them merge into one step.
            let edit = diff(pair[0], pair[1]);
            let cursor = edit.at + edit.inserted.len();
            h.record(Step { edit, cursor_before: cursor - 1, cursor_after: cursor, typed: true, at_ms: i as i64 * 5000 });
        }
        let mut body = bodies[3].to_string();
        for expected in bodies[..3].iter().rev() {
            let (text, _) = h.undo(&body).unwrap().unwrap();
            assert_eq!(&text, expected);
            body = text;
        }
        assert_eq!(h.undo(&body), Ok(None));
        let (text, _) = h.redo(&body).unwrap().unwrap();
        assert_eq!(text, bodies[1]);
    }
}
