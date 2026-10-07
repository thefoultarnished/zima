//! Presentation mode: the note as full-screen slides.

use slint::{ComponentHandle, ModelRc, VecModel};

use super::{App, md_block};
use crate::markdown::{self, Block, Kind};
use crate::{MdBlock, Theme};

/// Slides are this much bigger than the note's own text.
const SLIDE_SCALE: f32 = 2.1;

/// Split on `---` lines; a note without any starts a new slide at each top-level heading.
pub fn split_slides(blocks: Vec<Block>) -> Vec<Vec<Block>> {
    let has_rules = blocks.iter().any(|b| b.kind == Kind::Rule);
    let mut slides: Vec<Vec<Block>> = vec![Vec::new()];
    for block in blocks {
        let new_slide = if has_rules { block.kind == Kind::Rule } else { block.kind == Kind::Heading && block.level <= 2 };
        if new_slide && !slides.last().is_some_and(|s| s.is_empty()) {
            slides.push(Vec::new());
        }
        if block.kind != Kind::Rule {
            slides.last_mut().unwrap().push(block);
        }
    }
    slides.retain(|s| !s.is_empty());
    for slide in &mut slides {
        if let Some(first) = slide.first_mut() {
            first.space = 0;
        }
    }
    slides
}

impl App {
    pub fn start_presenting(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(note) = self.current() else {
            self.toast("Open a note to present it", false);
            return;
        };
        let dark = ui.global::<Theme>().get_dark();
        let mut slides: Vec<Vec<MdBlock>> = split_slides(markdown::parse(&note.body))
            .into_iter()
            .map(|slide| slide.into_iter().map(|b| md_block(b, dark)).collect())
            .collect();
        // The title opens the show.
        let title = super::display_title(note);
        let starts_with_heading = slides.first().and_then(|s| s.first()).is_some_and(|b| b.kind == "heading");
        if !note.title.is_empty() && !starts_with_heading {
            let mut heading = Block::new(Kind::Heading, format!("**{}**", markdown::escape(&title)), 0);
            heading.level = 1;
            slides.insert(0, vec![md_block(heading, dark)]);
        }
        if slides.is_empty() {
            self.toast("Nothing to present yet", false);
            return;
        }
        self.slides = slides;
        self.slide = 0;
        ui.global::<Theme>().set_text_scale(self.state.zoom.clamp(0.7, 2.0) * SLIDE_SCALE);
        if self.restore_to.is_none() {
            self.restore_to = crate::window::cover_screen(&ui);
        }
        ui.set_presenting(true);
        self.show_slide();
    }

    pub fn step_slide(&mut self, delta: i32) {
        if self.slides.is_empty() {
            return;
        }
        self.slide = (self.slide as i32 + delta).clamp(0, self.slides.len() as i32 - 1) as usize;
        self.show_slide();
    }

    fn show_slide(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let blocks = self.slides.get(self.slide).cloned().unwrap_or_default();
        ui.set_slide_blocks(ModelRc::new(VecModel::from(blocks)));
        ui.set_slide_number(self.slide as i32 + 1);
        ui.set_slide_count(self.slides.len() as i32);
    }

    pub fn stop_presenting(&mut self) {
        self.slides.clear();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_presenting(false);
            if let Some(placement) = self.restore_to.take() {
                crate::window::restore_placement(&ui, placement);
            }
            ui.global::<Theme>().set_text_scale(self.state.zoom.clamp(0.7, 2.0));
            ui.invoke_focus_default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(body: &str) -> Vec<usize> {
        split_slides(markdown::parse(body)).iter().map(|s| s.len()).collect()
    }

    #[test]
    fn slides() {
        // Split on rules, which aren't shown.
        assert_eq!(kinds("# One\n\ntext\n\n---\n\n# Two\n\n---\n\nthree"), vec![2, 1, 1]);
        // No rules: each top-level heading starts a slide.
        assert_eq!(kinds("intro\n\n# A\n\na\n\n## B\n\nb\n\n### c"), vec![1, 2, 3]);
        // A leading rule doesn't make an empty slide.
        assert_eq!(kinds("---\n\nonly"), vec![1]);
    }
}
