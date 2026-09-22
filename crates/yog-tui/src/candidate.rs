use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yog_runtime::Candidate;

use crate::{
    encoding::{DecoderDraft, EncodingDraft, cycle},
    text_input::TextInput,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateField {
    Decoder,
    DecodeDevice,
    Encoder,
    Preset,
    Multipass,
    EncodeDevice,
    QualityPoints,
}

impl CandidateField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Decoder => "Decoder",
            Self::DecodeDevice => "Decode device",
            Self::Encoder => "Encoder",
            Self::Preset => "Preset",
            Self::Multipass => "Multipass",
            Self::EncodeDevice => "Encode device",
            Self::QualityPoints => "Quality points",
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
    decoder: DecoderDraft,
    encoding: EncodingDraft,
}

impl CandidateDraft {
    pub fn new(decoder: DecoderDraft, encoding: EncodingDraft) -> Self {
        Self { decoder, encoding }
    }

    pub fn build(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            decoding: self.decoder.backend(),
            encoding: self.encoding.build(),
            qualities: self.encoding.qualities()?,
        })
    }

    pub fn summary(&self) -> String {
        let mut details = vec![self.encoding.encoder_label(), self.decoder.backend().name()];
        if self.encoding.preset_count() > 0 {
            details.push(format!("Preset {}", self.encoding.preset_label()));
        }
        if self.encoding.is_nvenc() && self.encoding.multipass_label() != "Default" {
            details.push(format!("Multipass {}", self.encoding.multipass_label()));
        }
        if self.encoding.is_vaapi() {
            details.push(format!("Device {}", self.encoding.device(false)));
        }
        details.push(self.encoding.quality_points_label(false));
        details.join(" / ")
    }

    fn fields(&self) -> Vec<CandidateField> {
        let mut fields = vec![CandidateField::Decoder, CandidateField::Encoder];
        if self.decoder.has_device() {
            fields.insert(1, CandidateField::DecodeDevice);
        }
        if self.encoding.preset_count() > 0 {
            fields.push(CandidateField::Preset);
        }
        if self.encoding.is_nvenc() {
            fields.push(CandidateField::Multipass);
        }
        if self.encoding.is_vaapi() {
            fields.push(CandidateField::EncodeDevice);
        }
        fields.push(CandidateField::QualityPoints);
        fields
    }

    fn value(&self, field: CandidateField, editing: bool) -> String {
        match field {
            CandidateField::Decoder => self.decoder.label().to_owned(),
            CandidateField::DecodeDevice => self.decoder.device(editing),
            CandidateField::Encoder => self.encoding.encoder_label(),
            CandidateField::Preset => self.encoding.preset_label(),
            CandidateField::Multipass => self.encoding.multipass_label().to_owned(),
            CandidateField::EncodeDevice => self.encoding.device(editing),
            CandidateField::QualityPoints => self.encoding.quality_points_label(editing),
        }
    }

    fn adjust(&mut self, field: CandidateField, direction: isize) {
        match field {
            CandidateField::Decoder => {
                self.decoder.adjust(direction);
            }
            CandidateField::DecodeDevice
            | CandidateField::EncodeDevice
            | CandidateField::QualityPoints => {}
            CandidateField::Encoder => {
                self.encoding.adjust_encoder(direction);
            }
            CandidateField::Preset => {
                self.encoding.adjust_preset(direction);
            }
            CandidateField::Multipass => {
                self.encoding.adjust_multipass(direction);
            }
        }
    }

    fn text_input(&mut self, field: CandidateField) -> &mut TextInput {
        match field {
            CandidateField::DecodeDevice => self.decoder.device_input(),
            CandidateField::EncodeDevice => self.encoding.device_input(),
            CandidateField::QualityPoints => self.encoding.quality_points_input(),
            _ => unreachable!("candidate field does not support text editing"),
        }
    }
}

struct CandidateEdit {
    index: Option<usize>,
    draft: CandidateDraft,
    focus: usize,
    text_editing: Option<CandidateField>,
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
        if let Some(field) = self
            .editing
            .as_ref()
            .and_then(|editing| editing.text_editing)
        {
            self.handle_text_key(field, key);
            return CandidateAction::None;
        }

        if self.editing.is_some() {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => self.editing = None,
                KeyCode::Enter => {
                    if matches!(
                        self.focused(),
                        Some(
                            CandidateField::DecodeDevice
                                | CandidateField::EncodeDevice
                                | CandidateField::QualityPoints
                        )
                    ) {
                        let field = self.focused().unwrap();
                        let editing = self.editing.as_mut().unwrap();
                        editing.draft.text_input(field).begin();
                        editing.text_editing = Some(field);
                    } else {
                        self.save();
                    }
                }
                KeyCode::Char('s') => self.save(),
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

    pub fn is_text_editing(&self, field: CandidateField) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|editing| editing.text_editing == Some(field))
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
            .value(field, self.is_text_editing(field))
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
            text_editing: None,
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
            text_editing: None,
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

    fn handle_text_key(&mut self, field: CandidateField, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            self.editing.as_mut().unwrap().text_editing = None;
            return;
        }

        let input = self.editing.as_mut().unwrap().draft.text_input(field);
        match key.code {
            KeyCode::Left => input.move_left(),
            KeyCode::Right => input.move_right(),
            KeyCode::Home => input.move_home(),
            KeyCode::End => input.move_end(),
            KeyCode::Backspace => input.backspace(),
            KeyCode::Delete => input.delete(),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => input.clear(),
            KeyCode::Char(value)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                input.insert(value);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(value: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(value), KeyModifiers::NONE)
    }

    fn draft(
        decoder: usize,
        encoder: usize,
        preset: usize,
        quality_points: &str,
    ) -> CandidateDraft {
        CandidateDraft::new(
            DecoderDraft::new(decoder),
            EncodingDraft::new(encoder, preset, 23, quality_points),
        )
    }

    #[test]
    fn candidates_are_saved_before_they_enter_the_list() {
        let seed = draft(0, 1, 5, "18-30");
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
        let seed = draft(0, 1, 5, "18-30");
        let mut editor = CandidateEditor::new(vec![seed], draft(0, 1, 5, "0-51"));

        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle_key(key('l'));
        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(
            editor.candidates()[0].build().unwrap().decoding,
            yog_core::ffmpeg::decoding::DecodingBackend::Software
        ));

        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle_key(key('l'));
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            editor.candidates()[0].build().unwrap().decoding,
            yog_core::ffmpeg::decoding::DecodingBackend::Vaapi(None)
        ));
    }

    #[test]
    fn nvenc_candidate_carries_its_independent_multipass_and_quality_points() {
        let mut draft = draft(2, 5, 3, "10,20,25-27");
        draft.encoding.adjust_multipass(-1);

        let candidate = draft.build().unwrap();
        assert!(matches!(
            candidate.decoding,
            yog_core::ffmpeg::decoding::DecodingBackend::Cuda(None)
        ));
        assert_eq!(candidate.encoding.preset().as_deref(), Some("p4"));
        assert_eq!(candidate.encoding.multipass(), Some("fullres"));
        assert_eq!(candidate.qualities, vec![10, 20, 25, 26, 27]);
    }

    #[test]
    fn hardware_device_text_is_saved_with_the_candidate() {
        let seed = draft(0, 1, 5, "18-30");
        let mut editor = CandidateEditor::new(vec![seed], draft(0, 1, 5, "0-51"));

        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle_key(key('l'));
        editor.handle_key(key('j'));
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle_key(key('0'));
        editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        editor.handle_key(key('s'));

        assert!(matches!(
            editor.candidates()[0].build().unwrap().decoding,
            yog_core::ffmpeg::decoding::DecodingBackend::Vaapi(Some(device)) if device == "0"
        ));
    }
}
