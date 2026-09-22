use crossterm::event::{KeyCode, KeyEvent};
use yog_core::ffmpeg::encoding::{NvencMultipass, VideoEncoding};
use yog_runtime::Candidate;

use crate::encoding::{DECODERS, ENCODERS, cycle};

const MULTIPASS: &[(&str, Option<NvencMultipass>)] = &[
    ("Default", None),
    ("Disabled", Some(NvencMultipass::Disabled)),
    (
        "Quarter resolution",
        Some(NvencMultipass::QuarterResolution),
    ),
    ("Full resolution", Some(NvencMultipass::FullResolution)),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateField {
    Decoder,
    Encoder,
    Preset,
    Multipass,
    RangeStart,
    RangeEnd,
}

impl CandidateField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Decoder => "Decoder",
            Self::Encoder => "Encoder",
            Self::Preset => "Preset",
            Self::Multipass => "Multipass",
            Self::RangeStart => "Range start",
            Self::RangeEnd => "Range end",
        }
    }
}

pub enum CandidateAction {
    None,
    Cancel,
    Confirm(Vec<CandidateDraft>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDraft {
    decoder: usize,
    encoder: usize,
    preset: usize,
    multipass: usize,
    range_start: u8,
    range_end: u8,
}

impl CandidateDraft {
    pub fn new(
        decoder: usize,
        encoder: usize,
        preset: usize,
        range_start: u8,
        range_end: u8,
    ) -> Self {
        Self {
            decoder,
            encoder,
            preset,
            multipass: 0,
            range_start,
            range_end,
        }
    }

    pub fn build(&self) -> Candidate {
        let mut encoding = ENCODERS[self.encoder].configured(self.preset);
        if let VideoEncoding::Nvenc { multipass, .. } = &mut encoding {
            *multipass = MULTIPASS[self.multipass].1;
        }
        Candidate {
            decoding: DECODERS[self.decoder].backend(),
            encoding,
            qualities: (self.range_start..=self.range_end).collect(),
        }
    }

    pub fn summary(&self) -> String {
        let encoder = ENCODERS[self.encoder];
        let encoding = encoder.encoding();
        let mut details = vec![encoder.label(), DECODERS[self.decoder].label().to_owned()];
        if encoder.preset_count() > 0 {
            details.push(format!("Preset {}", encoder.preset_label(self.preset)));
        }
        if encoder.is_nvenc() && self.multipass > 0 {
            details.push(format!("Multipass {}", MULTIPASS[self.multipass].0));
        }
        details.push(format!(
            "{} {}..={}",
            encoding.quality_parameter(),
            self.range_start,
            self.range_end
        ));
        details.join(" / ")
    }

    fn fields(&self) -> Vec<CandidateField> {
        let encoder = self.encoder();
        let mut fields = vec![CandidateField::Decoder, CandidateField::Encoder];
        if encoder.preset_count() > 0 {
            fields.push(CandidateField::Preset);
        }
        if encoder.is_nvenc() {
            fields.push(CandidateField::Multipass);
        }
        fields.extend([CandidateField::RangeStart, CandidateField::RangeEnd]);
        fields
    }

    fn value(&self, field: CandidateField) -> String {
        let encoder = self.encoder();
        match field {
            CandidateField::Decoder => DECODERS[self.decoder].label().to_owned(),
            CandidateField::Encoder => encoder.label(),
            CandidateField::Preset => encoder.preset_label(self.preset),
            CandidateField::Multipass => MULTIPASS[self.multipass].0.to_owned(),
            CandidateField::RangeStart => format!(
                "{} {}",
                encoder.encoding().quality_parameter(),
                self.range_start
            ),
            CandidateField::RangeEnd => format!(
                "{} {}",
                encoder.encoding().quality_parameter(),
                self.range_end
            ),
        }
    }

    fn adjust(&mut self, field: CandidateField, direction: isize) {
        match field {
            CandidateField::Decoder => {
                self.decoder = cycle(self.decoder, DECODERS.len(), direction);
            }
            CandidateField::Encoder => {
                self.encoder = cycle(self.encoder, ENCODERS.len(), direction);
                let encoder = self.encoder();
                self.preset = self.preset.min(encoder.preset_count().saturating_sub(1));
                self.multipass = 0;
                let range = encoder.encoding().quality_range();
                self.range_start = *range.start();
                self.range_end = *range.end();
            }
            CandidateField::Preset => {
                self.preset = cycle(self.preset, self.encoder().preset_count(), direction);
            }
            CandidateField::Multipass => {
                self.multipass = cycle(self.multipass, MULTIPASS.len(), direction);
            }
            CandidateField::RangeStart => {
                let minimum = *self.encoder().encoding().quality_range().start();
                self.range_start = if direction < 0 {
                    self.range_start.saturating_sub(1).max(minimum)
                } else {
                    self.range_start.saturating_add(1).min(self.range_end)
                };
            }
            CandidateField::RangeEnd => {
                let maximum = *self.encoder().encoding().quality_range().end();
                self.range_end = if direction < 0 {
                    self.range_end.saturating_sub(1).max(self.range_start)
                } else {
                    self.range_end.saturating_add(1).min(maximum)
                };
            }
        }
    }

    fn encoder(&self) -> crate::encoding::EncoderChoice {
        ENCODERS[self.encoder]
    }
}

struct CandidateEdit {
    index: Option<usize>,
    draft: CandidateDraft,
    focus: usize,
}

pub struct CandidateEditor {
    candidates: Vec<CandidateDraft>,
    seed: CandidateDraft,
    selected: usize,
    editing: Option<CandidateEdit>,
}

impl CandidateEditor {
    pub fn new(candidates: Vec<CandidateDraft>, seed: CandidateDraft) -> Self {
        Self {
            candidates,
            seed,
            selected: 0,
            editing: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> CandidateAction {
        if self.editing.is_some() {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => self.editing = None,
                KeyCode::Enter => self.save(),
                KeyCode::Char('j') | KeyCode::Down => self.move_focus(1),
                KeyCode::Char('k') | KeyCode::Up => self.move_focus(-1),
                KeyCode::Char('h') | KeyCode::Left => self.adjust(-1),
                KeyCode::Char('l') | KeyCode::Right => self.adjust(1),
                KeyCode::Char('g') | KeyCode::Home => {
                    self.editing.as_mut().unwrap().focus = 0;
                }
                KeyCode::Char('G') | KeyCode::End => {
                    let last = self.fields().len().saturating_sub(1);
                    self.editing.as_mut().unwrap().focus = last;
                }
                _ => {}
            }
            return CandidateAction::None;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return CandidateAction::Cancel,
            KeyCode::Char('s') => return CandidateAction::Confirm(self.candidates.clone()),
            KeyCode::Enter | KeyCode::Char('e') => self.edit(),
            KeyCode::Char('a') => self.add(),
            KeyCode::Char('d') => self.remove(),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => self.selected = 0,
            KeyCode::Char('G') | KeyCode::End => {
                self.selected = self.candidates.len().saturating_sub(1);
            }
            _ => {}
        }
        CandidateAction::None
    }

    pub fn candidates(&self) -> &[CandidateDraft] {
        &self.candidates
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    pub fn is_new_candidate(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|editing| editing.index.is_none())
    }

    pub fn fields(&self) -> Vec<CandidateField> {
        self.settings()
            .map_or_else(Vec::new, CandidateDraft::fields)
    }

    pub fn focused(&self) -> Option<CandidateField> {
        let editing = self.editing.as_ref()?;
        editing.draft.fields().get(editing.focus).copied()
    }

    pub fn value(&self, field: CandidateField) -> String {
        self.settings()
            .expect("candidate fields require a selected or edited candidate")
            .value(field)
    }

    fn settings(&self) -> Option<&CandidateDraft> {
        self.editing
            .as_ref()
            .map(|editing| &editing.draft)
            .or_else(|| self.candidates.get(self.selected))
    }

    fn move_selection(&mut self, direction: isize) {
        if !self.candidates.is_empty() {
            self.selected = cycle(self.selected, self.candidates.len(), direction);
        }
    }

    fn move_focus(&mut self, direction: isize) {
        let editing = self.editing.as_mut().unwrap();
        editing.focus = cycle(editing.focus, editing.draft.fields().len(), direction);
    }

    fn adjust(&mut self, direction: isize) {
        let Some(field) = self.focused() else {
            return;
        };
        let editing = self.editing.as_mut().unwrap();
        editing.draft.adjust(field, direction);
        editing.focus = editing.focus.min(editing.draft.fields().len() - 1);
    }

    fn add(&mut self) {
        let draft = self
            .candidates
            .get(self.selected)
            .cloned()
            .unwrap_or_else(|| self.seed.clone());
        self.editing = Some(CandidateEdit {
            index: None,
            draft,
            focus: 0,
        });
    }

    fn edit(&mut self) {
        let Some(draft) = self.candidates.get(self.selected).cloned() else {
            return;
        };
        self.editing = Some(CandidateEdit {
            index: Some(self.selected),
            draft,
            focus: 0,
        });
    }

    fn save(&mut self) {
        let editing = self.editing.take().unwrap();
        if let Some(index) = editing.index {
            self.candidates[index] = editing.draft;
            self.selected = index;
        } else {
            self.candidates.push(editing.draft);
            self.selected = self.candidates.len() - 1;
        }
    }

    fn remove(&mut self) {
        if self.candidates.is_empty() {
            return;
        }
        self.candidates.remove(self.selected);
        self.selected = self.selected.min(self.candidates.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(value: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(value), KeyModifiers::NONE)
    }

    #[test]
    fn candidates_are_saved_before_they_enter_the_list() {
        let seed = CandidateDraft::new(0, 1, 5, 18, 30);
        let mut editor = CandidateEditor::new(Vec::new(), seed.clone());
        assert!(editor.candidates().is_empty());

        editor.handle_key(key('a'));
        assert!(editor.is_editing());
        assert!(editor.candidates().is_empty());
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(editor.candidates(), std::slice::from_ref(&seed));

        editor.handle_key(key('a'));
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(editor.candidates(), &[seed.clone(), seed.clone()]);
        assert_eq!(editor.selected(), 1);

        editor.handle_key(key('d'));
        assert_eq!(editor.candidates(), &[seed]);
        assert_eq!(editor.selected(), 0);

        editor.handle_key(key('d'));
        assert!(editor.candidates().is_empty());
        assert!(editor.focused().is_none());
    }

    #[test]
    fn editing_an_existing_candidate_can_be_cancelled_or_saved() {
        let seed = CandidateDraft::new(0, 1, 5, 18, 30);
        let mut editor = CandidateEditor::new(vec![seed], CandidateDraft::new(0, 1, 5, 0, 51));

        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle_key(key('l'));
        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            editor.candidates()[0].build().decoding,
            yog_core::ffmpeg::decoding::DecodingBackend::Software
        ));

        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle_key(key('l'));
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            editor.candidates()[0].build().decoding,
            yog_core::ffmpeg::decoding::DecodingBackend::Vaapi(None)
        ));
    }

    #[test]
    fn nvenc_candidate_carries_its_independent_multipass_and_range() {
        let mut draft = CandidateDraft::new(2, 5, 3, 10, 30);
        draft.multipass = 3;

        let candidate = draft.build();
        assert!(matches!(
            candidate.decoding,
            yog_core::ffmpeg::decoding::DecodingBackend::Cuda(None)
        ));
        assert_eq!(candidate.encoding.preset().as_deref(), Some("p4"));
        assert_eq!(candidate.encoding.multipass(), Some("fullres"));
        assert_eq!(candidate.qualities, (10..=30).collect::<Vec<_>>());
    }
}
