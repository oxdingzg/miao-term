//! One writing surface: rendered Markdown blocks, with the active block edited
//! in place. The rope remains the only document; UI edits use its transactions
//! so saving, external reloads and undo share the source editor's history.

use std::ops::Range;

use mtty_editor::{history::EditKind, Change, Document, Selection, Transaction};
use pulldown_cmark::{Event, Options, Parser};

#[derive(Default)]
pub struct LiveMarkdown {
    source_mode: bool,
    revision: Option<u64>,
    text: String,
    blocks: Vec<Range<usize>>,
    active: Option<Range<usize>>,
    focus_next: bool,
}

impl LiveMarkdown {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        mut render: impl FnMut(&mut egui::Ui, &str),
    ) {
        if self.revision != Some(doc.revision()) {
            self.text = doc.rope().to_string();
            self.blocks = blocks(&self.text);
            self.active = None;
            self.revision = Some(doc.revision());
        }
        ui.horizontal(|ui| {
            if ui.selectable_label(!self.source_mode, "Markdown").clicked() {
                self.source_mode = false;
                self.active = None;
            }
            if ui
                .selectable_label(self.source_mode, "Source / 源码")
                .clicked()
            {
                self.source_mode = true;
                self.active = None;
                self.focus_next = true;
            }
            if ui.button("Undo / 撤销").clicked() && doc.undo() {
                self.revision = None;
            }
            if ui.button("Redo / 重做").clicked() && doc.redo() {
                self.revision = None;
            }
        });
        ui.separator();
        if self.revision.is_none() {
            ui.ctx().request_repaint();
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("live-markdown-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                if self.source_mode {
                    self.edit(ui, doc, 0..self.text.len());
                    return;
                }
                // Keep a block whole while Enter creates new paragraphs inside
                // it. Reparse it when the writer leaves it, not on each key.
                let ranges = self.blocks.clone();
                let mut activate = None;
                let mut leave = false;
                for (index, range) in ranges.iter().enumerate() {
                    ui.push_id(index, |ui| {
                        if self.active.as_ref() == Some(range) {
                            leave |= self.edit(ui, doc, range.clone());
                        } else {
                            let inner = ui.scope(|ui| {
                                if self.text[range.clone()].trim().is_empty() {
                                    ui.label("Click to write / 点击开始写作");
                                } else {
                                    render(ui, &self.text[range.clone()]);
                                }
                            });
                            let response = ui.interact(
                                inner.response.rect,
                                ui.id().with("markdown-block"),
                                egui::Sense::click(),
                            );
                            if response.clicked() {
                                activate = Some(range.clone());
                            }
                        }
                        ui.add_space(8.0);
                    });
                    // An edit changes byte offsets for all later blocks. Draw
                    // them next frame from the updated source, never slice the
                    // new text with offsets from this frame's old snapshot.
                    if self.text.len() != ranges.last().map_or(0, |r| r.end) {
                        ui.ctx().request_repaint();
                        break;
                    }
                }
                if let Some(range) = activate {
                    self.blocks = blocks(&self.text);
                    self.active = Some(range);
                    self.focus_next = true;
                    ui.ctx().request_repaint();
                } else if leave {
                    self.active = None;
                    self.blocks = blocks(&self.text);
                    ui.ctx().request_repaint();
                }
                if ui.button("+ Paragraph / 新段落").clicked() {
                    let at = self.text.len();
                    let end = doc.rope().len_chars();
                    let added = if self.text.ends_with("\n\n") || self.text.is_empty() {
                        "\n"
                    } else {
                        "\n\n"
                    };
                    doc.apply(
                        Transaction::new(vec![Change::insert(end, added)]),
                        Selection::cursor(end + added.chars().count()),
                        EditKind::Other,
                    );
                    self.text.push_str(added);
                    self.blocks.push(at..self.text.len());
                    self.active = Some(at..self.text.len());
                    self.revision = Some(doc.revision());
                    self.focus_next = true;
                    ui.ctx().request_repaint();
                }
            });
    }

    /// Returns true when the writer leaves this block. Changes reach the rope
    /// immediately, including while the IME/text field retains keyboard focus.
    fn edit(&mut self, ui: &mut egui::Ui, doc: &mut Document, range: Range<usize>) -> bool {
        let old = self.text[range.clone()].to_string();
        let mut draft = old.clone();
        let output = egui::TextEdit::multiline(&mut draft)
            .id(ui.id().with("markdown-input"))
            .desired_width(f32::INFINITY)
            .desired_rows(old.lines().count().max(1))
            .font(egui::TextStyle::Monospace)
            .show(ui);
        if self.focus_next {
            output.response.request_focus();
            self.focus_next = false;
        }
        let start = self.text[..range.start].chars().count();
        if output.response.changed() {
            replace(doc, start, &old, &draft);
            self.text.replace_range(range.clone(), &draft);
            let new_range = range.start..range.start + draft.len();
            if !self.source_mode {
                if let Some(block) = self.blocks.iter_mut().find(|r| **r == range) {
                    *block = new_range.clone();
                }
                // Shift later block ranges by the changed byte length.
                for block in &mut self.blocks {
                    if block.start >= range.end && block.start != range.start {
                        block.start = block.start + draft.len() - old.len();
                        block.end = block.end + draft.len() - old.len();
                    }
                }
                self.active = Some(new_range);
            }
            self.revision = Some(doc.revision());
        }
        if output.response.has_focus() {
            if let Some(cursor) = output.cursor_range {
                doc.set_selection(mtty_editor::Selection::single(mtty_editor::Range::new(
                    start + cursor.secondary.ccursor.index,
                    start + cursor.primary.ccursor.index,
                )));
            }
        }
        output.response.lost_focus()
    }
}

/// Top-level parser offsets keep fences, tables, quotes and nested lists whole.
/// Each span owns the whitespace up to the next block, so editing never loses
/// separators, CRLF or trailing newlines. All offsets are UTF-8 byte offsets.
fn blocks(text: &str) -> Vec<Range<usize>> {
    let mut depth = 0;
    let mut starts = Vec::new();
    for (event, range) in Parser::new_ext(text, Options::all()).into_offset_iter() {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    starts.push(range.start);
                }
                depth += 1;
            }
            Event::End(_) => depth -= 1,
            _ if depth == 0 => starts.push(range.start),
            _ => {}
        }
    }
    starts.sort_unstable();
    starts.dedup();
    if starts.is_empty() {
        return std::iter::once(0..text.len()).collect();
    }
    starts[0] = 0;
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| start..starts.get(i + 1).copied().unwrap_or(text.len()))
        .collect()
}

/// Change only differing characters. This preserves unaffected selections and
/// tree-sitter offsets, and handles CJK/emoji without splitting UTF-8 bytes.
fn replace(doc: &mut Document, start: usize, old: &str, new: &str) {
    let before: Vec<char> = old.chars().collect();
    let after: Vec<char> = new.chars().collect();
    let prefix = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let inserted: String = after[prefix..after.len() - suffix].iter().collect();
    doc.apply(
        Transaction::new(vec![Change::replace(
            start + prefix,
            start + before.len() - suffix,
            inserted,
        )]),
        Selection::cursor(start + after.len() - suffix),
        EditKind::Typing,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complex_blocks_keep_their_source_and_nested_structure() {
        let text = "# 标题\r\n\r\n- a\r\n  - nested\r\n- b\r\n\r\n```rs\r\n\r\nlet x = 1;\r\n```\r\n\r\n最后😀\r\n";
        let ranges = blocks(text);
        assert_eq!(ranges.len(), 4);
        assert_eq!(
            ranges.iter().map(|r| &text[r.clone()]).collect::<String>(),
            text
        );
        assert!(text[ranges[1].clone()].contains("nested"));
        assert!(text[ranges[2].clone()].contains("let x = 1;"));
    }

    #[test]
    fn empty_document_is_an_editable_block() {
        assert_eq!(blocks(""), [0..0]);
        assert_eq!(blocks("\n\n"), [0..2]);
    }

    #[test]
    fn live_text_field_edits_the_same_document_and_reconciles_external_changes() {
        let ctx = egui::Context::default();
        let mut live = LiveMarkdown::default();
        let mut doc = Document::from_text("# 标题\n\n正文\n\n尾部\n");
        let frame = |live: &mut LiveMarkdown, doc: &mut Document, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        live.show(ui, doc, |ui, text| {
                            ui.label(text);
                        });
                    });
                },
            );
        };
        frame(&mut live, &mut doc, Vec::new());
        live.active = Some(live.blocks[1].clone());
        live.focus_next = true;
        frame(&mut live, &mut doc, Vec::new());
        assert!(ctx.wants_keyboard_input());
        frame(
            &mut live,
            &mut doc,
            vec![egui::Event::Text("中文😀".into())],
        );
        assert!(doc.rope().to_string().starts_with("# 标题\n\n"));
        assert!(doc.rope().to_string().contains("中文😀"));
        assert_eq!(live.text, doc.rope().to_string());
        assert!(live.text.ends_with("尾部\n"));
        frame(&mut live, &mut doc, Vec::new());
        assert_eq!(
            live.blocks
                .iter()
                .map(|r| &live.text[r.clone()])
                .collect::<String>(),
            live.text
        );
        // An agent edit/reload uses the rope too: invalidate the field before
        // it can write its older draft over the newly changed document.
        doc.apply_external(Transaction::new(vec![Change::insert(0, "新内容\n\n")]));
        frame(&mut live, &mut doc, Vec::new());
        assert_eq!(live.text, doc.rope().to_string());
        assert!(live.active.is_none());
        assert!(live.text.starts_with("新内容\n\n"));
    }

    #[test]
    fn inline_edits_keep_other_blocks_unicode_and_undo() {
        let mut doc = Document::from_text("# 标题\n\n**旧😀**\n\n尾部\n");
        replace(&mut doc, 6, "**旧😀**\n\n", "**新字😀**\n\n");
        assert_eq!(doc.rope().to_string(), "# 标题\n\n**新字😀**\n\n尾部\n");
        assert!(doc.is_modified());
        assert!(doc.undo());
        assert_eq!(doc.rope().to_string(), "# 标题\n\n**旧😀**\n\n尾部\n");
        assert!(doc.redo());
        assert_eq!(doc.rope().to_string(), "# 标题\n\n**新字😀**\n\n尾部\n");
    }
}
