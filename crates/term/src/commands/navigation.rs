//! Location jumps, jump history, and labeled pattern navigation.

use super::context::Context;
use crate::{
    commands::picker::PathStyleConfig,
    ui::{self, overlay::overlaid, Picker},
};
use editor_core::{
    graphemes::{self, ensure_grapheme_boundary_next, next_grapheme_boundary},
    line_ending::line_end_char_index,
    movement::{Direction, Movement},
    Range, Selection, Tendril,
};
use std::{borrow::Cow, num::NonZeroUsize, path::Path};
use stdx::rope::RopeExt as _;
use text_layout::text_annotations::Overlay;
use ui_core::{input::KeyEvent, keyboard::KeyModifiers};
use view::{
    document::Mode,
    editor::Action,
    quicklist::{QuicklistEntry, QuicklistPosition, QuicklistTarget},
    Document, DocumentId, Editor, View,
};

pub(super) fn goto_next_buffer(cx: &mut Context) {
    goto_buffer(cx.editor, Direction::Forward, cx.count());
}

pub(super) fn goto_previous_buffer(cx: &mut Context) {
    goto_buffer(cx.editor, Direction::Backward, cx.count());
}

pub(super) fn goto_buffer(editor: &mut Editor, direction: Direction, count: usize) {
    let current = view!(editor).document_id();

    let id = match direction {
        Direction::Forward => {
            let iter = editor.documents.keys();
            // skip 'count' times past current buffer
            iter.cycle().skip_while(|id| *id != &current).nth(count)
        }
        Direction::Backward => {
            let iter = editor.documents.keys();
            // skip 'count' times past current buffer
            iter.rev()
                .cycle()
                .skip_while(|id| *id != &current)
                .nth(count)
        }
    }
    .unwrap();

    let id = *id;

    editor.switch(id, Action::Replace);
}

pub(super) fn goto_file_start(cx: &mut Context) {
    goto_file_start_impl(cx, Movement::Move);
}

pub(super) fn extend_to_file_start(cx: &mut Context) {
    goto_file_start_impl(cx, Movement::Extend);
}

fn goto_file_start_impl(cx: &mut Context, movement: Movement) {
    if cx.count.is_some() {
        goto_line_impl(cx, movement);
    } else {
        let (view, doc) = current!(cx.editor);
        let text = doc.text().slice(..);
        let selection = doc
            .selection(view.id)
            .clone()
            .transform(|range| range.put_cursor(text, 0, movement == Movement::Extend));
        push_jump(view, doc);
        doc.set_selection(view.id, selection);
    }
}

pub(super) fn goto_file_end(cx: &mut Context) {
    goto_file_end_impl(cx, Movement::Move);
}

pub(super) fn extend_to_file_end(cx: &mut Context) {
    goto_file_end_impl(cx, Movement::Extend)
}

fn goto_file_end_impl(cx: &mut Context, movement: Movement) {
    let (view, doc) = current!(cx.editor);
    let text = doc.text().slice(..);
    let pos = doc.text().len_chars();
    let selection = doc
        .selection(view.id)
        .clone()
        .transform(|range| range.put_cursor(text, pos, movement == Movement::Extend));
    push_jump(view, doc);
    doc.set_selection(view.id, selection);
}

// Store a jump on the jumplist.
pub(super) fn push_jump(view: &mut View, doc: &mut Document) {
    doc.commit_changes();
    view.sync_changes(doc);
    let jump = (doc.id(), doc.selection(view.id).clone());
    view.push_jump(doc, jump);
}

pub(super) fn goto_line(cx: &mut Context) {
    goto_line_impl(cx, Movement::Move);
}

fn goto_line_impl(cx: &mut Context, movement: Movement) {
    if cx.count.is_some() {
        let (view, doc) = current!(cx.editor);
        push_jump(view, doc);

        goto_line_without_jumplist(cx.editor, cx.count, movement);
    }
}

pub(super) fn goto_line_without_jumplist(
    editor: &mut Editor,
    count: Option<NonZeroUsize>,
    movement: Movement,
) {
    if let Some(count) = count {
        let (view, doc) = current!(editor);
        let text = doc.text().slice(..);
        let max_line = if text
            .line(
                text.len_lines(stdx::rope::LINE_TYPE) - 1,
                stdx::rope::LINE_TYPE,
            )
            .len_chars()
            == 0
        {
            // If the last line is blank, don't jump to it.
            text.len_lines(stdx::rope::LINE_TYPE).saturating_sub(2)
        } else {
            text.len_lines(stdx::rope::LINE_TYPE) - 1
        };
        let line_idx = std::cmp::min(count.get() - 1, max_line);
        let pos = text.line_to_char_idx(line_idx);
        let selection = doc
            .selection(view.id)
            .clone()
            .transform(|range| range.put_cursor(text, pos, movement == Movement::Extend));

        doc.set_selection(view.id, selection);
    }
}

pub(super) fn goto_last_line(cx: &mut Context) {
    goto_last_line_impl(cx, Movement::Move)
}

pub(super) fn extend_to_last_line(cx: &mut Context) {
    goto_last_line_impl(cx, Movement::Extend)
}

fn goto_last_line_impl(cx: &mut Context, movement: Movement) {
    let (view, doc) = current!(cx.editor);
    let text = doc.text().slice(..);
    let line_idx = if text
        .line(
            text.len_lines(stdx::rope::LINE_TYPE) - 1,
            stdx::rope::LINE_TYPE,
        )
        .len_chars()
        == 0
    {
        // If the last line is blank, don't jump to it.
        text.len_lines(stdx::rope::LINE_TYPE).saturating_sub(2)
    } else {
        text.len_lines(stdx::rope::LINE_TYPE) - 1
    };
    let pos = text.line_to_char_idx(line_idx);
    let selection = doc
        .selection(view.id)
        .clone()
        .transform(|range| range.put_cursor(text, pos, movement == Movement::Extend));

    push_jump(view, doc);
    doc.set_selection(view.id, selection);
}

pub(super) fn goto_column(cx: &mut Context) {
    goto_column_impl(cx, Movement::Move);
}

pub(super) fn extend_to_column(cx: &mut Context) {
    goto_column_impl(cx, Movement::Extend);
}

fn goto_column_impl(cx: &mut Context, movement: Movement) {
    let count = cx.count();
    let (view, doc) = current!(cx.editor);
    let text = doc.text().slice(..);
    let selection = doc.selection(view.id).clone().transform(|range| {
        let line = range.cursor_line(text);
        let line_start = text.line_to_char_idx(line);
        let line_end = line_end_char_index(&text, line);
        let pos = graphemes::nth_next_grapheme_boundary(text, line_start, count - 1).min(line_end);
        range.put_cursor(text, pos, movement == Movement::Extend)
    });
    push_jump(view, doc);
    doc.set_selection(view.id, selection);
}

pub(super) fn goto_last_accessed_file(cx: &mut Context) {
    let view = view_mut!(cx.editor);
    if let Some(alt) = view.docs_access_history.pop() {
        cx.editor.switch(alt, Action::Replace);
    } else {
        cx.editor.set_error(|| "no last accessed buffer")
    }
}

pub(super) fn goto_last_modification(cx: &mut Context) {
    let mode = cx.editor.mode();
    let (view, doc) = current!(cx.editor);
    let pos = view::editing::last_edit_pos(doc);
    let text = doc.text().slice(..);
    if let Some(pos) = pos {
        let selection = doc
            .selection(view.id)
            .clone()
            .transform(|range| range.put_cursor(text, pos, mode == Mode::Select));
        push_jump(view, doc);
        doc.set_selection(view.id, selection);
    }
}

pub(super) fn goto_last_modified_file(cx: &mut Context) {
    let view = view!(cx.editor);
    let alternate_file = view
        .last_modified_docs
        .into_iter()
        .flatten()
        .find(|&id| id != view.document_id());
    if let Some(alt) = alternate_file {
        cx.editor.switch(alt, Action::Replace);
    } else {
        cx.editor.set_error(|| "no last modified buffer")
    }
}

pub(super) fn jump_forward(cx: &mut Context) {
    cx.editor.jump_forward(cx.editor.tree.focus(), cx.count());
}

pub(super) fn jump_backward(cx: &mut Context) {
    cx.editor.jump_backward(cx.editor.tree.focus(), cx.count());
}

pub(super) fn save_selection(cx: &mut Context) {
    let (view, doc) = current!(cx.editor);
    push_jump(view, doc);
    cx.editor.set_status("Selection saved to jumplist");
}

pub(super) fn jumplist_picker(cx: &mut Context) {
    struct JumpMeta<'a> {
        id: DocumentId,
        path: Option<Cow<'a, Path>>,
        selection: Selection,
        line_start: usize,
        text: String,
        is_current: bool,
    }

    for (view, _) in cx.editor.tree.views_mut() {
        for doc_id in view.jumps.iter().map(|e| e.0).collect::<Vec<_>>().iter() {
            let doc = doc_mut!(cx.editor, doc_id);
            view.sync_changes(doc);
        }
    }

    let new_meta = |view: &View, doc_id: DocumentId, selection: Selection| {
        let doc = doc!(cx.editor, &doc_id);
        let text = doc.text().slice(..);
        let contents = selection
            .fragments(text)
            .map(Cow::into_owned)
            .collect::<Vec<_>>()
            .join(" ");
        let line_start = selection.primary().cursor_line(text);

        JumpMeta {
            id: doc_id,
            path: doc
                .path()
                .map(ToOwned::to_owned)
                .map(stdx::path::get_relative_path),
            selection,
            line_start,
            text: contents,
            is_current: view.document_id() == doc_id,
        }
    };

    let columns = [
        ui::PickerColumn::new("id", |item: &JumpMeta, _| item.id.to_string().into()),
        ui::PickerColumn::new("path", |item: &JumpMeta, config: &PathStyleConfig| {
            config.stylize(item.path.as_deref(), Some(item.line_start))
        })
        .with_prefix(|item, config| config.icon(item.path.as_deref())),
        ui::PickerColumn::new("flags", |item: &JumpMeta, _| {
            let mut flags = Vec::new();
            if item.is_current {
                flags.push("*");
            }

            if flags.is_empty() {
                "".into()
            } else {
                format!(" ({})", flags.join("")).into()
            }
        }),
        ui::PickerColumn::new("contents", |item: &JumpMeta, _| item.text.as_str().into()),
    ];

    let picker = Picker::new(
        cx.editor.redraw().clone(),
        columns,
        1, // path
        cx.editor.tree.views().flat_map(|(view, _)| {
            view.jumps
                .iter()
                .rev()
                .map(|(doc_id, selection)| new_meta(view, *doc_id, selection.clone()))
        }),
        PathStyleConfig::new(cx.editor),
        |cx, meta, action| {
            cx.editor.switch(meta.id, action);
            let config = cx.editor.config();
            let (view, doc) = (view_mut!(cx.editor), doc_mut!(cx.editor, &meta.id));
            doc.set_selection(view.id, meta.selection.clone());
            if action.align_view(view, doc.id()) {
                view.ensure_cursor_in_view_center(doc, config.scroll.padding);
            }
        },
    )
    .with_title("Jump List")
    .with_preview(|editor, meta| {
        let doc = &editor.documents.get(&meta.id)?;
        let line = meta.selection.primary().cursor_line(doc.text().slice(..));
        Some((meta.id.into(), Some((line, line))))
    })
    .with_quicklist(|_editor, meta| {
        Some(QuicklistEntry {
            target: QuicklistTarget::Document(meta.id),
            position: QuicklistPosition::Selection(meta.selection.clone()),
        })
    });
    cx.push_layer(Box::new(overlaid(picker)));
}

pub(super) fn goto_pattern(cx: &mut Context) {
    jump_to_pattern(cx, Movement::Move)
}

pub(super) fn extend_to_pattern(cx: &mut Context) {
    jump_to_pattern(cx, Movement::Extend)
}

/// Read a two-character pattern, then jump to a labeled occurrence of it in the
/// focused view, like Leap's `<Plug>(leap)`.
fn jump_to_pattern(cx: &mut Context, behaviour: Movement) {
    let alphabet = cx.editor.config().jump_label_alphabet.clone();
    if alphabet.is_empty() {
        return;
    }
    let labels = label_chars(&alphabet);
    let limit = labels.len() * labels.len();

    cx.on_next_key(move |cx, event| {
        let Some(first) = plain_char(event) else {
            return;
        };
        cx.on_next_key(move |cx, event| {
            let Some(second) = plain_char(event) else {
                return;
            };
            let targets = {
                let (view, doc) = current_ref!(cx.editor);
                find_pattern_targets(view, doc, first, second, limit)
            };
            jump_to_label(cx, targets, labels, behaviour);
        });
    });
}

/// Input characters for patterns and labels. Shortcuts are ignored, but Shift is
/// kept because terminals report it for uppercase characters.
fn plain_char(event: KeyEvent) -> Option<char> {
    event.char().filter(|_| {
        !event
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    })
}

/// Characters used for single-key labels: the configured alphabet followed by
/// its uppercase forms, so up to twice the alphabet size is reachable with one
/// key. Characters without a distinct single-character uppercase are skipped.
fn label_chars(alphabet: &[char]) -> Vec<char> {
    let mut chars = alphabet.to_vec();
    for &ch in alphabet {
        let mut upper = ch.to_uppercase();
        let (Some(upper), None) = (upper.next(), upper.next()) else {
            continue;
        };
        if upper != ch && !chars.contains(&upper) {
            chars.push(upper);
        }
    }
    chars
}

fn label_text(label: char) -> Tendril {
    let mut text = Tendril::new();
    text.push(label);
    text
}

/// Label every target and jump to the one the user picks. A lone target is
/// jumped to directly, like Leap does. Targets that fit in the single-key label
/// set are picked with one key; beyond that two-character labels are used.
fn jump_to_label(
    cx: &mut Context,
    mut targets: Vec<Range>,
    labels: Vec<char>,
    behaviour: Movement,
) {
    if targets.is_empty() || labels.is_empty() {
        return;
    }
    if targets.len() == 1 {
        jump_to_target(cx, targets[0], behaviour);
        return;
    }

    if targets.len() > labels.len() {
        // Two-character labels replace both matched characters, so a match that
        // ends on a line ending has no room for the second label.
        let text = doc!(cx.editor).text().slice(..);
        targets.retain(|target| {
            text.get_char_at(target.from() + 1)
                .is_none_or(|ch| !is_line_ending(ch))
        });
    }

    match targets.len() {
        0 => {}
        1 => jump_to_target(cx, targets[0], behaviour),
        len if len <= labels.len() => jump_to_single_label(cx, targets, labels, behaviour),
        _ => jump_to_double_label(cx, targets, labels, behaviour),
    }
}

/// Label each target with one character and jump when that key is pressed.
fn jump_to_single_label(
    cx: &mut Context,
    targets: Vec<Range>,
    labels: Vec<char>,
    behaviour: Movement,
) {
    let mut overlays: Vec<_> = targets
        .iter()
        .zip(&labels)
        .map(|(target, &label)| Overlay::new(target.from(), label_text(label)))
        .collect();
    overlays.sort_unstable_by_key(|overlay| overlay.char_idx);
    let (view, doc) = current!(cx.editor);
    doc.set_jump_labels(view.id, overlays);
    let view_id = view.id;
    let doc = doc.id();

    cx.on_next_key(move |cx, event| {
        doc_mut!(cx.editor, &doc).remove_jump_labels(view_id);
        let Some(label) = plain_char(event) else {
            return;
        };
        let Some(index) = labels.iter().position(|&candidate| candidate == label) else {
            return;
        };
        if let Some(target) = targets.get(index).copied() {
            jump_to_target(cx, target, behaviour);
        }
    });
}

/// Label each target with two characters and jump once both are pressed.
fn jump_to_double_label(
    cx: &mut Context,
    targets: Vec<Range>,
    labels: Vec<char>,
    behaviour: Movement,
) {
    let label_count = labels.len();
    let doc = doc!(cx.editor);
    let text = doc.text().slice(..);
    let mut overlays: Vec<_> = targets
        .iter()
        .enumerate()
        .flat_map(|(i, target)| {
            [
                Overlay::new(target.from(), label_text(labels[i / label_count])),
                Overlay::new(
                    next_grapheme_boundary(text, target.from()),
                    label_text(labels[i % label_count]),
                ),
            ]
        })
        .collect();
    overlays.sort_unstable_by_key(|overlay| overlay.char_idx);
    let (view, doc) = current!(cx.editor);
    doc.set_jump_labels(view.id, overlays);
    let view_id = view.id;
    let doc = doc.id();

    cx.on_next_key(move |cx, event| {
        let Some(first) = plain_char(event) else {
            doc_mut!(cx.editor, &doc).remove_jump_labels(view_id);
            return;
        };
        let Some(first_index) = labels.iter().position(|&label| label == first) else {
            doc_mut!(cx.editor, &doc).remove_jump_labels(view_id);
            return;
        };
        let outer = first_index * label_count;
        if outer >= targets.len() {
            doc_mut!(cx.editor, &doc).remove_jump_labels(view_id);
            return;
        }
        cx.on_next_key(move |cx, event| {
            doc_mut!(cx.editor, &doc).remove_jump_labels(view_id);
            let Some(second) = plain_char(event) else {
                return;
            };
            let Some(inner) = labels.iter().position(|&label| label == second) else {
                return;
            };
            if let Some(target) = targets.get(outer + inner).copied() {
                jump_to_target(cx, target, behaviour);
            }
        });
    });
}

/// Positions where `first` is followed by `second` in the focused view. Only
/// matches whose two characters are fully on screen are kept, nearest to the
/// cursor first so the easiest labels land on the closest matches.
fn find_pattern_targets(
    view: &View,
    doc: &Document,
    first: char,
    second: char,
    limit: usize,
) -> Vec<Range> {
    let text = doc.text().slice(..);
    // This is not necessarily exact if there is virtual text like soft wrap.
    // The layout check below trims the extra candidates.
    let start = text.line_to_char_idx(text.char_to_line_idx(doc.view_offset(view.id).anchor));
    let end = text.line_to_char_idx(view.estimate_last_doc_line(doc) + 1);
    let layout = view.layout(doc, None);
    let cursor = layout.display_position_at_pos(doc.selection(view.id).primary().cursor(text));

    let mut targets: Vec<(usize, Range)> = Vec::with_capacity(limit);
    let mut pos = start;
    while pos + 1 < end && targets.len() < limit {
        // Folded text is not drawn, so matches inside it cannot be labeled.
        if let Some(conceal) = layout.annotations.conceal_at(pos) {
            pos = conceal.range.end.max(pos + 1);
            continue;
        }
        let (Some(leading), Some(trailing)) = (text.get_char_at(pos), text.get_char_at(pos + 1))
        else {
            pos += 1;
            continue;
        };
        // A label replaces the matched character, so a match cannot start on a
        // line ending.
        if !chars_match(leading, first)
            || !chars_match(trailing, second)
            || is_line_ending(leading)
            || ensure_grapheme_boundary_next(text, pos) != pos
        {
            pos += 1;
            continue;
        }

        // Labels only make sense for matches that are fully visible. A match
        // ending on a line ending is visible with its start, even though the
        // line ending itself has no usable cell.
        let Some(start_pos) = layout.display_position_at_pos(pos) else {
            pos += 2;
            continue;
        };
        let end_visible = is_line_ending(trailing)
            || layout
                .display_position_at_pos(pos + 1)
                .is_some_and(|next| next.row == start_pos.row);
        if end_visible {
            let distance = cursor.map_or(0, |cursor| {
                let rows = cursor.row.abs_diff(start_pos.row);
                let cols = cursor.col.abs_diff(start_pos.col);
                rows * rows + cols * cols
            });
            targets.push((
                distance,
                Range::new(pos, next_grapheme_boundary(text, pos + 1)),
            ));
        }
        // Occurrences do not overlap, matching Leap's repeated search.
        pos += 2;
    }

    targets.sort_by_key(|(distance, _)| *distance);
    targets.into_iter().map(|(_, range)| range).collect()
}

/// A whitespace pattern character matches any whitespace, so a typed space can
/// also reach tabs and line endings; other characters compare case-insensitively.
fn chars_match(candidate: char, pattern: char) -> bool {
    if pattern.is_whitespace() {
        candidate.is_whitespace()
    } else {
        candidate == pattern || candidate.to_lowercase().eq(pattern.to_lowercase())
    }
}

fn is_line_ending(ch: char) -> bool {
    matches!(ch, '\n' | '\r')
}

fn jump_to_target(cx: &mut Context, target: Range, behaviour: Movement) {
    let (view, doc) = current!(cx.editor);
    let selection = if behaviour == Movement::Extend {
        let anchor = doc.selection(view.id).primary().anchor;
        let head = if anchor <= target.from() {
            target.to()
        } else {
            target.from()
        };
        Range::new(anchor, head)
    } else {
        Range::point(target.from())
    };
    push_jump(view, doc);
    doc.set_selection(view.id, selection.into());
}

/// A command prompt owns its preview until it is confirmed or cancelled.
#[derive(Default)]
pub(super) enum GotoLinePreview {
    #[default]
    Idle,
    Active(GotoLineOrigin),
    Invalidated,
}

pub(super) struct GotoLineOrigin {
    document: DocumentId,
    view: view::ViewId,
    binding_revision: u64,
    document_version: i32,
    selection: Selection,
}

impl GotoLineOrigin {
    fn is_valid(&self, editor: &Editor) -> bool {
        editor.tree.try_get(self.view).is_some_and(|view| {
            view.document_id() == self.document && view.binding_revision() == self.binding_revision
        }) && editor
            .document(self.document)
            .is_some_and(|doc| doc.version() == self.document_version)
    }
}

impl GotoLinePreview {
    pub(super) fn abort(&mut self, editor: &mut Editor) {
        if matches!(self, Self::Invalidated) {
            return;
        }
        let Self::Active(origin) = std::mem::take(self) else {
            return;
        };
        if !origin.is_valid(editor) {
            *self = Self::Invalidated;
            return;
        }
        if editor.tree.focus() != origin.view {
            *self = Self::Invalidated;
        }
        let scrolloff = editor.config().scroll.padding;
        let doc = editor.documents.get_mut(&origin.document).unwrap();
        doc.set_selection(origin.view, origin.selection);
        editor
            .tree
            .get_mut(origin.view)
            .ensure_cursor_in_view(doc, scrolloff);
    }

    pub(super) fn handle(
        &mut self,
        editor: &mut Editor,
        args: ::command_line::Args,
        event: ui::PromptEvent,
    ) -> anyhow::Result<()> {
        use ui::PromptEvent;

        if event == PromptEvent::Abort || (event == PromptEvent::Update && args.is_empty()) {
            self.abort(editor);
            return Ok(());
        }
        let line = args[0].parse::<usize>()?;
        if matches!(self, Self::Idle) {
            let (view, doc) = current_ref!(editor);
            *self = Self::Active(GotoLineOrigin {
                document: doc.id(),
                view: view.id,
                binding_revision: view.binding_revision(),
                document_version: doc.version(),
                selection: doc.selection(view.id).clone(),
            });
        }
        let Self::Active(origin) = self else {
            return Ok(());
        };
        if !origin.is_valid(editor) {
            // A stale session must not attach itself to a replacement document or split.
            *self = Self::Invalidated;
            return Ok(());
        }
        if editor.tree.focus() != origin.view {
            self.abort(editor);
            *self = Self::Invalidated;
            return Ok(());
        }

        goto_line_preview(editor, NonZeroUsize::new(line));

        if event == PromptEvent::Validate
            && let Self::Active(origin) = std::mem::take(self)
        {
            let (view, doc) = current!(editor);
            view.push_jump(doc, (origin.document, origin.selection));
        }
        Ok(())
    }
}

fn goto_line_preview(editor: &mut Editor, line: Option<NonZeroUsize>) {
    let scrolloff = editor.config().scroll.padding;
    goto_line_without_jumplist(
        editor,
        line,
        if editor.mode() == Mode::Select {
            Movement::Extend
        } else {
            Movement::Move
        },
    );
    let (view, doc) = current!(editor);
    view.ensure_cursor_in_view(doc, scrolloff);
}

pub(super) mod typed {
    //! Typable navigation commands.

    use crate::compositor;
    use ::command_line::Args;
    use std::num::NonZeroUsize;

    #[cold]
    pub(in crate::commands) fn goto_line_number(
        cx: &mut compositor::Context,
        args: Args,
    ) -> anyhow::Result<()> {
        let line = args[0].parse::<usize>()?;
        let (view, doc) = current!(cx.editor);
        let selection = doc.selection(view.id).clone();
        view.push_jump(doc, (doc.id(), selection));
        super::goto_line_preview(cx.editor, NonZeroUsize::new(line));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::label_chars;

    #[test]
    fn single_key_labels_extend_the_alphabet_with_uppercase() {
        let alphabet: Vec<char> = ('a'..='z').collect();
        let labels = label_chars(&alphabet);
        assert_eq!(52, labels.len());
        assert_eq!(('a'..='z').collect::<Vec<_>>(), labels[..26].to_vec());
        assert_eq!('A', labels[26]);
        assert_eq!('Z', labels[51]);
    }

    #[test]
    fn single_key_labels_skip_characters_without_a_distinct_uppercase() {
        assert_eq!(vec!['1', 'a', 'A'], label_chars(&['1', 'a']));
    }

    #[test]
    fn single_key_labels_stay_unique() {
        assert_eq!(vec!['a', 'A'], label_chars(&['a', 'A']));
    }
}
