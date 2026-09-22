use std::{num::NonZeroU32, path::PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yog_core::ffmpeg::{
    encoding::VideoEncoding,
    plan::{Container, TranscodeRequest, VideoAction as CoreVideoAction},
    vmaf::VmafOptions,
};
use yog_runtime::{Command, Config, EmulationOptions, Operation, Options, Validate};

use crate::{
    candidate::{CandidateDraft, CandidateEditor},
    encoding::{DECODERS, ENCODERS, EncoderChoice, cycle},
    file_picker::PathSelection,
};

const CONTAINERS: &[Option<Container>] = &[
    None,
    Some(Container::Matroska),
    Some(Container::Mp4),
    Some(Container::Mov),
    Some(Container::M4a),
    Some(Container::ThreeGp),
    Some(Container::ThreeG2),
    Some(Container::F4v),
    Some(Container::Ismv),
    Some(Container::Psp),
    Some(Container::Webm),
    Some(Container::MpegTs),
    Some(Container::M2ts),
    Some(Container::Avi),
    Some(Container::Flv),
    Some(Container::Asf),
    Some(Container::Wmv),
    Some(Container::MpegPs),
    Some(Container::Vob),
    Some(Container::Ogg),
    Some(Container::Ogv),
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Input,
    Output,
    Png,
    Svg,
    Container,
    Decoder,
    Action,
    Candidates,
    Encoder,
    Preset,
    Quality,
    RangeStart,
    RangeEnd,
    Overwrite,
    Verify,
    Vmaf,
    VmafInterval,
    Start,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Transcode,
    Predict,
    Emulate,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Transcode => "Transcode",
            Self::Predict => "Predict",
            Self::Emulate => "Emulate",
        }
    }
}

pub enum FormAction {
    None,
    PickInput,
    EditCandidates,
    Submit,
    Quit,
}

pub struct RunRequest {
    pub command: Command,
    pub options: Options,
    pub config: Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VideoAction {
    Copy,
    Encode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VmafMode {
    Off,
    Full,
    Subsample,
}

#[derive(Debug, Default)]
struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    fn begin(&mut self) {
        self.cursor = self.value.len();
    }

    fn insert(&mut self, value: char) {
        self.value.insert(self.cursor, value);
        self.cursor += value.len_utf8();
    }

    fn backspace(&mut self) {
        let Some((index, _)) = self.value[..self.cursor].char_indices().next_back() else {
            return;
        };
        self.value.remove(index);
        self.cursor = index;
    }

    fn delete(&mut self) {
        if self.cursor < self.value.len() {
            self.value.remove(self.cursor);
        }
    }

    fn move_left(&mut self) {
        if let Some((index, _)) = self.value[..self.cursor].char_indices().next_back() {
            self.cursor = index;
        }
    }

    fn move_right(&mut self) {
        if let Some(value) = self.value[self.cursor..].chars().next() {
            self.cursor += value.len_utf8();
        }
    }

    fn display(&self, editing: bool) -> String {
        if !editing {
            return if self.value.is_empty() {
                "—".to_owned()
            } else {
                self.value.clone()
            };
        }

        let mut value = self.value.clone();
        value.insert(self.cursor, '│');
        value
    }
}

pub struct CommandForm {
    mode: Mode,
    input: Option<PathSelection>,
    output: TextInput,
    png: TextInput,
    svg: TextInput,
    container: usize,
    decoder: usize,
    action: VideoAction,
    encoder: usize,
    preset: usize,
    quality: u8,
    range_start: u8,
    range_end: u8,
    candidates: Vec<CandidateDraft>,
    overwrite: bool,
    verify: bool,
    vmaf: VmafMode,
    vmaf_interval: u32,
    focus: usize,
    editing: Option<Field>,
}

impl Default for CommandForm {
    fn default() -> Self {
        let range = ENCODERS[1].encoding().quality_range();
        Self {
            mode: Mode::Transcode,
            input: None,
            output: TextInput::default(),
            png: TextInput::default(),
            svg: TextInput::default(),
            container: 0,
            decoder: 0,
            action: VideoAction::Encode,
            encoder: 1,
            preset: 5,
            quality: 23,
            range_start: *range.start(),
            range_end: *range.end(),
            candidates: Vec::new(),
            overwrite: false,
            verify: false,
            vmaf: VmafMode::Off,
            vmaf_interval: 5,
            focus: 0,
            editing: None,
        }
    }
}

impl CommandForm {
    pub fn handle_key(&mut self, key: KeyEvent) -> FormAction {
        if let Some(field) = self.editing {
            self.handle_text_key(field, key);
            return FormAction::None;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return FormAction::Quit,
            KeyCode::Tab => self.change_mode(true),
            KeyCode::BackTab => self.change_mode(false),
            KeyCode::Char('j') | KeyCode::Down => self.move_focus(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_focus(-1),
            KeyCode::Char('h') | KeyCode::Left => self.adjust(-1),
            KeyCode::Char('l') | KeyCode::Right => self.adjust(1),
            KeyCode::Char('g') => self.focus = 0,
            KeyCode::Char('G') => self.focus = self.fields().len() - 1,
            KeyCode::Char('s') => return FormAction::Submit,
            KeyCode::Char(' ') | KeyCode::Enter => return self.activate(),
            _ => {}
        }
        FormAction::None
    }

    pub fn input(&self) -> Option<&PathSelection> {
        self.input.as_ref()
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_input(&mut self, input: PathSelection) {
        self.input = Some(input);
    }

    pub fn candidate_editor(&self) -> CandidateEditor {
        CandidateEditor::new(
            self.candidates.clone(),
            CandidateDraft::new(
                self.decoder,
                self.encoder,
                self.preset,
                self.range_start,
                self.range_end,
            ),
        )
    }

    pub fn set_candidates(&mut self, candidates: Vec<CandidateDraft>) {
        self.candidates = candidates;
        self.focus = self
            .fields()
            .iter()
            .position(|field| *field == Field::Candidates)
            .expect("candidate editing is only available in Emulate mode");
    }

    pub fn focused(&self) -> Field {
        self.fields()[self.focus]
    }

    pub fn source_fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Input];
        if self.mode == Mode::Transcode {
            fields.push(Field::Output);
        }
        fields.push(Field::Container);
        if self.mode != Mode::Emulate || self.candidates.is_empty() {
            fields.push(Field::Decoder);
        }
        fields
    }

    pub fn video_fields(&self) -> Vec<Field> {
        let mut fields = Vec::new();
        if self.mode == Mode::Emulate {
            fields.push(Field::Candidates);
            if !self.candidates.is_empty() {
                return fields;
            }
        }
        if self.mode == Mode::Transcode {
            fields.push(Field::Action);
        }
        if self.mode != Mode::Transcode || self.action == VideoAction::Encode {
            fields.push(Field::Encoder);
            if self.encoder().preset_count() > 0 {
                fields.push(Field::Preset);
            }
            if self.mode == Mode::Emulate {
                fields.extend([Field::RangeStart, Field::RangeEnd]);
            } else {
                fields.push(Field::Quality);
            }
        }
        fields
    }

    pub fn option_fields(&self) -> Vec<Field> {
        let mut fields = Vec::new();
        if self.mode == Mode::Transcode {
            fields.extend([Field::Overwrite, Field::Verify, Field::Vmaf]);
            if self.vmaf == VmafMode::Subsample {
                fields.push(Field::VmafInterval);
            }
        } else if self.mode == Mode::Emulate {
            fields.extend([Field::Png, Field::Svg, Field::Overwrite]);
        }
        fields.push(Field::Start);
        fields
    }

    pub fn label(field: Field) -> &'static str {
        match field {
            Field::Input => "Input",
            Field::Output => "Output",
            Field::Png => "PNG",
            Field::Svg => "SVG",
            Field::Container => "Container",
            Field::Decoder => "Decoder",
            Field::Action => "Video",
            Field::Candidates => "Candidates",
            Field::Encoder => "Encoder",
            Field::Preset => "Preset",
            Field::Quality => "Quality",
            Field::RangeStart => "Range start",
            Field::RangeEnd => "Range end",
            Field::Overwrite => "Overwrite",
            Field::Verify => "Verify",
            Field::Vmaf => "VMAF",
            Field::VmafInterval => "N subsample",
            Field::Start => "",
        }
    }

    pub fn value(&self, field: Field) -> String {
        match field {
            Field::Input => self
                .input
                .as_ref()
                .map_or_else(|| "—".to_owned(), PathSelection::display),
            Field::Output => self.output.display(self.editing == Some(field)),
            Field::Png => self.png.display(self.editing == Some(field)),
            Field::Svg => self.svg.display(self.editing == Some(field)),
            Field::Container => container_label(CONTAINERS[self.container]),
            Field::Decoder => DECODERS[self.decoder].label().to_owned(),
            Field::Action => match self.action {
                VideoAction::Copy => "Copy".to_owned(),
                VideoAction::Encode => "Encode".to_owned(),
            },
            Field::Candidates => {
                if self.candidates.is_empty() {
                    "Single encoder".to_owned()
                } else {
                    format!("{} configured", self.candidates.len())
                }
            }
            Field::Encoder => self.encoder().label(),
            Field::Preset => self.encoder().preset_label(self.preset),
            Field::Quality => format!(
                "{} {}",
                self.encoder().encoding().quality_parameter(),
                self.quality
            ),
            Field::RangeStart => format!(
                "{} {}",
                self.encoder().encoding().quality_parameter(),
                self.range_start
            ),
            Field::RangeEnd => format!(
                "{} {}",
                self.encoder().encoding().quality_parameter(),
                self.range_end
            ),
            Field::Overwrite => state(self.overwrite).to_owned(),
            Field::Verify => state(self.verify).to_owned(),
            Field::Vmaf => match self.vmaf {
                VmafMode::Off => "Off".to_owned(),
                VmafMode::Full => "Full".to_owned(),
                VmafMode::Subsample => "Subsample".to_owned(),
            },
            Field::VmafInterval => self.vmaf_interval.to_string(),
            Field::Start => "Start".to_owned(),
        }
    }

    pub fn is_editing(&self, field: Field) -> bool {
        self.editing == Some(field)
    }

    fn fields(&self) -> Vec<Field> {
        let mut fields = self.source_fields();
        fields.extend(self.video_fields());
        fields.extend(self.option_fields());
        fields
    }

    fn move_focus(&mut self, direction: isize) {
        self.focus = cycle(self.focus, self.fields().len(), direction);
    }

    fn change_mode(&mut self, forward: bool) {
        self.mode = match (self.mode, forward) {
            (Mode::Transcode, true) | (Mode::Emulate, false) => Mode::Predict,
            (Mode::Predict, true) | (Mode::Transcode, false) => Mode::Emulate,
            (Mode::Emulate, true) | (Mode::Predict, false) => Mode::Transcode,
        };
        self.clamp_focus();
    }

    fn activate(&mut self) -> FormAction {
        match self.focused() {
            Field::Input => return FormAction::PickInput,
            Field::Output | Field::Png | Field::Svg => {
                let field = self.focused();
                self.text_input_mut(field).begin();
                self.editing = Some(field);
            }
            Field::Candidates => return FormAction::EditCandidates,
            Field::Overwrite => self.overwrite = !self.overwrite,
            Field::Verify => self.verify = !self.verify,
            Field::Start => return FormAction::Submit,
            _ => self.adjust(1),
        }
        FormAction::None
    }

    fn adjust(&mut self, direction: isize) {
        match self.focused() {
            Field::Input | Field::Output | Field::Png | Field::Svg | Field::Candidates => {}
            Field::Container => {
                self.container = cycle(self.container, CONTAINERS.len(), direction);
            }
            Field::Decoder => {
                self.decoder = cycle(self.decoder, DECODERS.len(), direction);
            }
            Field::Action => {
                self.action = if direction < 0 {
                    VideoAction::Copy
                } else {
                    VideoAction::Encode
                };
                self.clamp_focus();
            }
            Field::Encoder => {
                self.encoder = cycle(self.encoder, ENCODERS.len(), direction);
                let encoder = self.encoder();
                self.preset = self.preset.min(encoder.preset_count().saturating_sub(1));
                let quality = encoder.encoding().quality_range();
                self.quality = self.quality.clamp(*quality.start(), *quality.end());
                if self.mode == Mode::Emulate {
                    self.range_start = *quality.start();
                    self.range_end = *quality.end();
                }
            }
            Field::Preset => {
                self.preset = cycle(self.preset, self.encoder().preset_count(), direction);
            }
            Field::Quality => {
                let quality = self.encoder().encoding().quality_range();
                self.quality = if direction < 0 {
                    self.quality.saturating_sub(1).max(*quality.start())
                } else {
                    self.quality.saturating_add(1).min(*quality.end())
                };
            }
            Field::RangeStart => {
                let minimum = *self.encoder().encoding().quality_range().start();
                self.range_start = if direction < 0 {
                    self.range_start.saturating_sub(1).max(minimum)
                } else {
                    self.range_start.saturating_add(1).min(self.range_end)
                };
            }
            Field::RangeEnd => {
                let maximum = *self.encoder().encoding().quality_range().end();
                self.range_end = if direction < 0 {
                    self.range_end.saturating_sub(1).max(self.range_start)
                } else {
                    self.range_end.saturating_add(1).min(maximum)
                };
            }
            Field::Overwrite => self.overwrite = direction > 0,
            Field::Verify => self.verify = direction > 0,
            Field::Vmaf => {
                let index = match self.vmaf {
                    VmafMode::Off => 0,
                    VmafMode::Full => 1,
                    VmafMode::Subsample => 2,
                };
                self.vmaf = match cycle(index, 3, direction) {
                    0 => VmafMode::Off,
                    1 => VmafMode::Full,
                    _ => VmafMode::Subsample,
                };
                self.clamp_focus();
            }
            Field::VmafInterval => {
                self.vmaf_interval = if direction < 0 {
                    self.vmaf_interval.saturating_sub(1).max(1)
                } else {
                    self.vmaf_interval.saturating_add(1)
                };
            }
            Field::Start => {}
        }
    }

    pub fn build_request(&self) -> Result<RunRequest, String> {
        let input = self
            .input
            .as_ref()
            .ok_or_else(|| "Select an input file or directory".to_owned())?;
        if self.mode == Mode::Transcode && self.output.value.is_empty() {
            return Err("Enter an output path".to_owned());
        }
        if self.mode == Mode::Emulate && input.recursive() {
            return Err("Emulation requires an input file".to_owned());
        }

        let candidate_mode = self.mode == Mode::Emulate && !self.candidates.is_empty();
        let video =
            if candidate_mode || self.mode == Mode::Transcode && self.action == VideoAction::Copy {
                CoreVideoAction::Copy
            } else {
                CoreVideoAction::Encode(self.selected_encoding(self.mode != Mode::Emulate))
            };
        let decoding = if candidate_mode {
            DECODERS[0].backend()
        } else {
            DECODERS[self.decoder].backend()
        };
        let output = match self.mode {
            Mode::Transcode => PathBuf::from(&self.output.value),
            Mode::Predict | Mode::Emulate => PathBuf::new(),
        };
        let mut request = TranscodeRequest::new(input.path(), output)
            .with_decoding(decoding)
            .with_video(video)
            .with_overwrite(self.mode != Mode::Predict && self.overwrite);
        if let Some(container) = CONTAINERS[self.container] {
            request = request.with_container(container);
        }

        let command = Command {
            request,
            operation: match self.mode {
                Mode::Transcode => Operation::Transcode,
                Mode::Predict => Operation::Predict,
                Mode::Emulate => Operation::Emulate(EmulationOptions {
                    png: (!self.png.value.is_empty()).then(|| PathBuf::from(&self.png.value)),
                    svg: (!self.svg.value.is_empty()).then(|| PathBuf::from(&self.svg.value)),
                    qualities: if candidate_mode {
                        Vec::new()
                    } else {
                        (self.range_start..=self.range_end).collect()
                    },
                    candidates: self.candidates.iter().map(CandidateDraft::build).collect(),
                }),
            },
            recursive: self.mode != Mode::Emulate && input.recursive(),
        };
        command.validate().map_err(|error| format!("{error:#}"))?;
        let options = Options {
            verify: self.mode == Mode::Transcode && self.verify,
            vmaf: match (self.mode, self.vmaf) {
                (Mode::Transcode, VmafMode::Full) => Some(VmafOptions::default()),
                (Mode::Transcode, VmafMode::Subsample) => Some(VmafOptions {
                    n_subsample: Some(
                        NonZeroU32::new(self.vmaf_interval)
                            .expect("VMAF interval is always greater than zero"),
                    ),
                }),
                (Mode::Transcode, VmafMode::Off) | (Mode::Predict | Mode::Emulate, _) => None,
            },
            terminal_output: false,
            ..Options::default()
        };
        let config = Config::load(None).map_err(|error| format!("{error:#}"))?;

        Ok(RunRequest {
            command,
            options,
            config,
        })
    }

    fn handle_text_key(&mut self, field: Field, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            self.editing = None;
            return;
        }

        let input = self.text_input_mut(field);
        match key.code {
            KeyCode::Left => input.move_left(),
            KeyCode::Right => input.move_right(),
            KeyCode::Home => input.cursor = 0,
            KeyCode::End => input.cursor = input.value.len(),
            KeyCode::Backspace => input.backspace(),
            KeyCode::Delete => input.delete(),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                input.value.clear();
                input.cursor = 0;
            }
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

    fn clamp_focus(&mut self) {
        self.focus = self.focus.min(self.fields().len() - 1);
    }

    fn selected_encoding(&self, fixed_quality: bool) -> VideoEncoding {
        let encoder = self.encoder();
        let mut encoding = encoder.configured(self.preset);
        if fixed_quality {
            encoding.set_quality(self.quality);
        }
        encoding
    }

    fn text_input_mut(&mut self, field: Field) -> &mut TextInput {
        match field {
            Field::Output => &mut self.output,
            Field::Png => &mut self.png,
            Field::Svg => &mut self.svg,
            _ => unreachable!("only output paths support text editing"),
        }
    }

    fn encoder(&self) -> EncoderChoice {
        ENCODERS[self.encoder]
    }
}

fn state(value: bool) -> &'static str {
    if value { "On" } else { "Off" }
}

fn container_label(container: Option<Container>) -> String {
    container.map_or_else(
        || "Auto".to_owned(),
        |value| value.extension().to_uppercase(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(value: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(value), KeyModifiers::NONE)
    }

    #[test]
    fn vim_navigation_moves_focus_and_changes_choices() {
        let mut form = CommandForm::default();
        assert_eq!(form.focused(), Field::Input);

        form.handle_key(key('j'));
        assert_eq!(form.focused(), Field::Output);

        form.handle_key(key('j'));
        assert_eq!(form.focused(), Field::Container);
        assert_eq!(form.value(Field::Container), "Auto");

        form.handle_key(key('l'));
        assert_eq!(form.value(Field::Container), "MKV");

        form.handle_key(key('k'));
        assert_eq!(form.focused(), Field::Output);
    }

    #[test]
    fn output_editing_keeps_vim_keys_as_text_until_editing_ends() {
        let mut form = CommandForm::default();
        form.handle_key(key('j'));
        form.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        form.handle_key(key('h'));
        form.handle_key(key('j'));
        form.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(form.value(Field::Output), "hj");
        assert!(matches!(form.handle_key(key('q')), FormAction::Quit));
    }

    #[test]
    fn tabs_expose_only_the_fields_for_each_operation() {
        let mut form = CommandForm::default();
        form.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(form.mode(), Mode::Predict);
        assert_eq!(
            form.source_fields(),
            vec![Field::Input, Field::Container, Field::Decoder]
        );
        assert!(!form.video_fields().contains(&Field::Action));
        assert_eq!(form.option_fields(), vec![Field::Start]);

        form.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(form.mode(), Mode::Emulate);
        assert_eq!(
            form.video_fields(),
            vec![
                Field::Candidates,
                Field::Encoder,
                Field::Preset,
                Field::RangeStart,
                Field::RangeEnd
            ]
        );
        assert_eq!(
            form.option_fields(),
            vec![Field::Png, Field::Svg, Field::Overwrite, Field::Start]
        );

        form.set_candidates(vec![CandidateDraft::new(0, 1, 5, 18, 30)]);
        assert_eq!(form.source_fields(), vec![Field::Input, Field::Container]);
        assert_eq!(form.video_fields(), vec![Field::Candidates]);
        assert_eq!(form.value(Field::Candidates), "1 configured");

        form.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(form.mode(), Mode::Transcode);

        form.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(form.mode(), Mode::Emulate);
    }
}
