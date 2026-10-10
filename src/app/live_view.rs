//! Live mode (view mode 3): the note is drawn like the preview, except the block(s) under the
//! cursor, which show as raw Markdown in one input. This file keeps the split between the two in
//! sync with the note text.

use std::ops::Range;
use std::rc::Rc;

use slint::{Model, ModelRc, StyledText, VecModel};

use super::{App, Ink, md_block};
use crate::markdown::{self, Block};
use crate::model::now_ms;
use crate::{AppWindow, MdBlock, live};

/// One row of the rendered part. Kept, so a re-layout can tell which rows changed.
#[derive(Clone, PartialEq)]
enum Shown {
    Block(Block),
    /// Raw Markdown that no block draws (HTML comments, link definitions), or part of a block cut by the raw part.
    Raw { text: String, line: usize },
}

impl Shown {
    fn to_md(&self, ink: &Ink) -> MdBlock {
        match self {
            Shown::Block(block) => md_block(block.clone(), ink),
            Shown::Raw { text, line } => MdBlock {
                kind: "raw".into(),
                text: StyledText::default(),
                plain: text.as_str().into(),
                level: 0,
                marker: "".into(),
                indent: 0,
                checked: false,
                cells: ModelRc::default(),
                line: *line as i32,
                space: 0,
                note_links: ModelRc::default(),
            },
        }
    }
}

/// What Live mode shows while it's on: which part of the note is raw, and the rows around it.
pub struct LiveState {
    /// Byte range of the raw part in the note body.
    pub active: Range<usize>,
    /// Counts the raw text's changes made by Rust. Typing sent before a change is then ignored.
    pub rev: i32,
    before: Rc<VecModel<MdBlock>>,
    after: Rc<VecModel<MdBlock>>,
    shown_before: Vec<Shown>,
    shown_after: Vec<Shown>,
}

impl App {
    /// Shows the open note in Live mode, with the raw part covering `anchor..cursor`.
    /// `focus` moves keyboard focus to the raw text; false keeps it where it is (e.g. in the find box).
    pub fn live_snap_at(&mut self, anchor: usize, cursor: usize, focus: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(body) = self.current().map(|n| n.body.clone()) else {
            self.live = None;
            return;
        };
        let blocks = markdown::parse(&body);
        let (anchor, cursor) = (clamp_offset(&body, anchor), clamp_offset(&body, cursor));
        let active = live::cover(&live::segments(&body, &blocks), anchor, cursor);
        let (before, after) = live::layout(&body, &blocks, active.clone());
        let ink = Ink::of(&ui);
        let shown_before = shown(&body, &blocks, before);
        let shown_after = shown(&body, &blocks, after);
        let rev = self.live.as_ref().map_or(0, |live| live.rev) + 1;
        let before = Rc::new(VecModel::from(to_md(&shown_before, &ink)));
        let after = Rc::new(VecModel::from(to_md(&shown_after, &ink)));
        ui.set_live_before(ModelRc::from(before.clone()));
        ui.set_live_after(ModelRc::from(after.clone()));
        self.live = Some(LiveState { active: active.clone(), rev, before, after, shown_before, shown_after });
        ui.set_live_text(body[active.clone()].into());
        ui.set_live_start(active.start as i32);
        ui.set_live_len(active.len() as i32);
        ui.set_live_rev(rev);
        self.cursor = cursor;
        select_in(&ui, anchor, cursor, focus);
    }

    /// Redraws the rows around the raw part after typing pauses. Only rows that changed are redrawn,
    /// and the raw part itself stays as it is.
    pub fn live_refresh(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let Some(active) = self.live.as_ref().map(|live| live.active.clone()) else { return };
        if body.get(active.clone()).is_none() {
            let cursor = self.cursor;
            self.live_snap_at(cursor, cursor, false);
            return;
        }
        let blocks = markdown::parse(&body);
        let (before, after) = live::layout(&body, &blocks, active);
        let (before, after) = (shown(&body, &blocks, before), shown(&body, &blocks, after));
        let ink = Ink::of(&ui);
        if let Some(live) = self.live.as_mut() {
            update_rows(&live.before, &mut live.shown_before, before, &ink);
            update_rows(&live.after, &mut live.shown_after, after, &ink);
        }
    }

    /// Typing in the raw part. Text from before a Rust change (an old `rev`) is dropped, and the
    /// current text is put back. Otherwise the typed text goes into the note as an ordinary edit.
    pub fn live_edited(&mut self, text: String, local_cursor: usize, rev: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let Some(live) = self.live.as_ref() else { return };
        let active = live.active.clone();
        if rev != live.rev {
            if let Some(current) = body.get(active) {
                ui.set_live_text(current.into());
            }
            return;
        }
        let Some((new_body, new_active, cursor)) = live::apply_typed(&body, active, &text, local_cursor) else {
            // Should not happen. The note is left as it was, and shown in Edit mode instead.
            self.toast("Live mode hit a problem, so this note is shown in Edit mode.", true);
            ui.set_view_mode(0);
            self.set_view_mode(0);
            return;
        };
        if let Some(live) = self.live.as_mut() {
            live.active = new_active;
        }
        ui.set_note_body(new_body.as_str().into());
        let edit = live::diff(&body, &new_body);
        self.live_history.record(live::Step { edit, cursor_before: self.cursor, cursor_after: cursor, typed: true, at_ms: now_ms() });
        self.set_body(new_body, cursor);
    }

    /// Ctrl+Z in Live mode. Rust does the undo, because the text box can't see the blocks being rebuilt.
    pub fn live_undo(&mut self) {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        match self.live_history.undo(&body) {
            Ok(Some((text, cursor))) => {
                let cursor = clamp_offset(&text, cursor);
                self.set_body_from_rust(text, Some((cursor, cursor)), false);
            }
            Ok(None) => {}
            Err(live::Unchanged) => self.toast("Can't undo past a change made outside this note.", false),
        }
    }

    /// Ctrl+Y in Live mode: the counterpart of `live_undo`.
    pub fn live_redo(&mut self) {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        match self.live_history.redo(&body) {
            Ok(Some((text, cursor))) => {
                let cursor = clamp_offset(&text, cursor);
                self.set_body_from_rust(text, Some((cursor, cursor)), false);
            }
            Ok(None) => {}
            Err(live::Unchanged) => self.toast("Can't redo past a change made outside this note.", false),
        }
    }

    /// A key pressed at the edge of the raw part (the `live-key` callback). Returns false when the
    /// text box should handle the key itself. Rust decides the move, so the text stays intact.
    pub fn live_key(&mut self, key: &str) -> bool {
        let Some(ui) = self.ui.upgrade() else { return false };
        let Some(body) = self.current().map(|n| n.body.clone()) else { return false };
        let Some(active) = self.live.as_ref().map(|live| live.active.clone()) else { return false };
        let (anchor, cursor) = self.selection();
        let segs = live::segments(&body, &markdown::parse(&body));
        let (anchor, cursor) = match live::key_target(&body, &segs, active.clone(), cursor, anchor, key) {
            Some(live::KeyAction::Delete(range)) => {
                // Backspace at the top or Delete at the bottom always joins the raw part with a neighbour,
                // so the rows around it are rebuilt as well.
                let Some(text) = live::splice(&body, range.clone(), "") else { return false };
                ui.invoke_live_anchor();
                self.replace_body(text, Some((range.start, range.start)));
                return true;
            }
            Some(live::KeyAction::Move(at)) => (at, at),
            Some(live::KeyAction::Select { anchor, cursor }) => (anchor, cursor),
            None => return false,
        };
        // Keep the caret at the same height on screen when the raw part moves to other blocks.
        if live::cover(&segs, anchor, cursor) != active {
            ui.invoke_live_anchor();
        }
        self.live_snap_at(anchor, cursor, true);
        true
    }

    /// The cursor moved inside the raw part (`local_cursor` is its byte offset there).
    pub fn live_cursor_moved(&mut self, local_cursor: usize) {
        let Some(start) = self.live.as_ref().map(|live| live.active.start) else { return };
        self.cursor_moved(start + local_cursor);
    }

    /// A rendered block was clicked: its part of the note becomes raw, with the cursor at the end of its text.
    pub fn live_activate(&mut self, line: usize) {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let Some(&start) = live::line_starts(&body).get(line) else { return };
        let segs = live::segments(&body, &markdown::parse(&body));
        let end = segs[live::segment_at(&segs, start)].content_end;
        self.live_snap_at(end, end, true);
    }

    /// Ctrl+clicked a rendered block: opens the first link in its text, if any.
    pub fn live_follow(&mut self, line: usize) {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let Some(&start) = live::line_starts(&body).get(line) else { return };
        let segs = live::segments(&body, &markdown::parse(&body));
        let seg = &segs[live::segment_at(&segs, start)];
        if let Some(url) = markdown::first_link(&body[seg.start..seg.end]) {
            self.follow_link(&url);
        }
    }

    /// Clicked below the note: the cursor goes to the end of it.
    pub fn live_end(&mut self) {
        let Some(len) = self.current().map(|n| n.body.len()) else { return };
        self.live_snap_at(len, len, true);
    }

    /// Selects `anchor..cursor` (whole-note offsets) in the editor. In Live mode the raw part
    /// moves first if the selection lies outside it.
    pub fn show_selection(&mut self, anchor: usize, cursor: usize, focus: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        let inside = match &self.live {
            Some(live) => live.active.start <= anchor.min(cursor) && anchor.max(cursor) <= live.active.end,
            None => true,
        };
        if inside {
            select_in(&ui, anchor, cursor, focus);
        } else {
            self.live_snap_at(anchor, cursor, focus);
        }
    }

    /// Shows the open note after a change made from Rust (formatting, commands, undo, sync).
    /// `old_body` is the text before the change. `selection` is where the cursor goes, if it moves.
    pub fn show_body_and_selection(&mut self, old_body: &str, selection: Option<(usize, usize)>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let Some(active) = self.live.as_ref().map(|live| live.active.clone()) else {
            ui.set_note_body(body.as_str().into());
            if let Some((anchor, cursor)) = selection {
                ui.invoke_set_body_selection(anchor as i32, cursor as i32);
                self.cursor = cursor;
            }
            return;
        };
        ui.set_note_body(body.as_str().into());
        let edit = live::diff(old_body, &body);
        let mapped = live::map_range(active.clone(), &edit);
        let (anchor, cursor) = selection.unwrap_or_else(|| {
            let at = live::map_offset(self.cursor, &edit);
            (at, at)
        });
        let edit_inside = active.start <= edit.at && edit.at + edit.removed.len() <= active.end;
        let selection_inside = mapped.start <= anchor.min(cursor) && anchor.max(cursor) <= mapped.end;
        self.cursor = cursor;
        match body.get(mapped.clone()) {
            Some(raw) if edit_inside && selection_inside => {
                // The change is in the raw part: show the new text there and keep the raw part.
                let rev = self.live.as_ref().map_or(0, |live| live.rev) + 1;
                if let Some(live) = self.live.as_mut() {
                    live.active = mapped.clone();
                    live.rev = rev;
                }
                ui.set_live_text(raw.into());
                ui.set_live_start(mapped.start as i32);
                ui.set_live_len(mapped.len() as i32);
                ui.set_live_rev(rev);
                select_in(&ui, anchor, cursor, selection.is_some());
            }
            _ => self.live_snap_at(anchor, cursor, selection.is_some()),
        }
    }
}

/// `offset` pulled back into the body and onto a character boundary.
fn clamp_offset(body: &str, offset: usize) -> usize {
    let mut at = offset.min(body.len());
    while !body.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Selects in the editor; `focus` also moves keyboard focus there.
fn select_in(ui: &AppWindow, anchor: usize, cursor: usize, focus: bool) {
    if focus {
        ui.invoke_set_body_selection(anchor as i32, cursor as i32);
    } else {
        ui.invoke_select_in_body(anchor as i32, cursor as i32);
    }
}

fn shown(body: &str, blocks: &[Block], rows: Vec<live::Row>) -> Vec<Shown> {
    rows.into_iter()
        .map(|row| match row {
            live::Row::Block(i) => Shown::Block(blocks[i].clone()),
            live::Row::Raw(range) => Shown::Raw { text: body[range.clone()].to_string(), line: body[..range.start].matches('\n').count() },
        })
        .collect()
}

fn to_md(shown: &[Shown], ink: &Ink) -> Vec<MdBlock> {
    shown.iter().map(|s| s.to_md(ink)).collect()
}

/// Makes `model` show `new`, changing only the rows that differ from `old`.
fn update_rows(model: &VecModel<MdBlock>, old: &mut Vec<Shown>, new: Vec<Shown>, ink: &Ink) {
    if old.len() == new.len() {
        for (i, (before, after)) in old.iter().zip(&new).enumerate() {
            if before != after {
                model.set_row_data(i, after.to_md(ink));
            }
        }
    } else {
        model.set_vec(to_md(&new, ink));
    }
    *old = new;
}
