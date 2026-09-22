use std::{num::NonZeroU32, path::PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yog_core::ffmpeg::{
    decoding::DecodingBackend,
    encoding::VideoEncoding,
    plan::{Container, TranscodeRequest, VideoAction as CoreVideoAction},
    vmaf::VmafOptions,
};
use yog_runtime::{Command, Config, Operation, Options, Validate};

use crate::file_picker::PathSelection;

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
const DECODERS: &[DecoderChoice] = &[
    DecoderChoice::Software,
    DecoderChoice::Vaapi,
    DecoderChoice::Cuda,
    DecoderChoice::Qsv,
];
const X26X_PRESETS: &[&str] = &[
    "ultrafast",
    "superfast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
    "veryslow",
];
const QSV_PRESETS: &[&str] = &[
    "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow",
];
const NVENC_PRESETS: &[&str] = &["p1", "p2", "p3", "p4", "p5", "p6", "p7"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Input,
    Output,
    Container,
    Decoder,
    Action,
    Encoder,
    Preset,
    Quality,
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
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Transcode => "Transcode",
            Self::Predict => "Predict",
        }
    }
}

pub enum FormAction {
    None,
    PickInput,
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

#[derive(Debug, Clone, Copy)]
enum DecoderChoice {
    Software,
    Vaapi,
    Cuda,
    Qsv,
}

impl DecoderChoice {
    fn label(self) -> &'static str {
        match self {
            Self::Software => "Software",
            Self::Vaapi => "VAAPI",
            Self::Cuda => "CUDA",
            Self::Qsv => "QSV",
        }
    }

    fn backend(self) -> DecodingBackend {
        match self {
            Self::Software => DecodingBackend::Software,
            Self::Vaapi => DecodingBackend::Vaapi(None),
            Self::Cuda => DecodingBackend::Cuda(None),
            Self::Qsv => DecodingBackend::Qsv(None),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Presets {
    Named(&'static [&'static str]),
    Numbered { first: u8, last: u8 },
    None,
}

impl Presets {
    fn len(self) -> usize {
        match self {
            Self::Named(values) => values.len(),
            Self::Numbered { first, last } => usize::from(last - first) + 1,
            Self::None => 0,
        }
    }

    fn label(self, index: usize) -> String {
        match self {
            Self::Named(values) => values[index].to_owned(),
            Self::Numbered { first, .. } => (first + index as u8).to_string(),
            Self::None => "—".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Encoder {
    name: &'static str,
    presets: Presets,
}

impl Encoder {
    fn encoding(self) -> VideoEncoding {
        self.name
            .parse()
            .expect("TUI encoder names must be accepted by yog-core")
    }
}

const ENCODERS: &[Encoder] = &[
    Encoder {
        name: "libx264",
        presets: Presets::Named(X26X_PRESETS),
    },
    Encoder {
        name: "libx265",
        presets: Presets::Named(X26X_PRESETS),
    },
    Encoder {
        name: "libsvtav1",
        presets: Presets::Numbered { first: 0, last: 13 },
    },
    Encoder {
        name: "libaom-av1",
        presets: Presets::Numbered { first: 0, last: 8 },
    },
    Encoder {
        name: "librav1e",
        presets: Presets::Numbered { first: 0, last: 10 },
    },
    Encoder {
        name: "h264_nvenc",
        presets: Presets::Named(NVENC_PRESETS),
    },
    Encoder {
        name: "hevc_nvenc",
        presets: Presets::Named(NVENC_PRESETS),
    },
    Encoder {
        name: "av1_nvenc",
        presets: Presets::Named(NVENC_PRESETS),
    },
    Encoder {
        name: "h264_qsv",
        presets: Presets::Named(QSV_PRESETS),
    },
    Encoder {
        name: "hevc_qsv",
        presets: Presets::Named(QSV_PRESETS),
    },
    Encoder {
        name: "av1_qsv",
        presets: Presets::Named(QSV_PRESETS),
    },
    Encoder {
        name: "h264_vaapi",
        presets: Presets::None,
    },
    Encoder {
        name: "hevc_vaapi",
        presets: Presets::None,
    },
    Encoder {
        name: "av1_vaapi",
        presets: Presets::None,
    },
];

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
    container: usize,
    decoder: usize,
    action: VideoAction,
    encoder: usize,
    preset: usize,
    quality: u8,
    overwrite: bool,
    verify: bool,
    vmaf: VmafMode,
    vmaf_interval: u32,
    focus: usize,
    editing: Option<Field>,
}

impl Default for CommandForm {
    fn default() -> Self {
        Self {
            mode: Mode::Transcode,
            input: None,
            output: TextInput::default(),
            container: 0,
            decoder: 0,
            action: VideoAction::Encode,
            encoder: 1,
            preset: 5,
            quality: 23,
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
            KeyCode::Tab | KeyCode::BackTab => self.switch_mode(),
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

    pub fn focused(&self) -> Field {
        self.fields()[self.focus]
    }

    pub fn source_fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Input];
        if self.mode == Mode::Transcode {
            fields.push(Field::Output);
        }
        fields.extend([Field::Container, Field::Decoder]);
        fields
    }

    pub fn video_fields(&self) -> Vec<Field> {
        let mut fields = Vec::new();
        if self.mode == Mode::Transcode {
            fields.push(Field::Action);
        }
        if self.mode == Mode::Predict || self.action == VideoAction::Encode {
            fields.push(Field::Encoder);
            if self.encoder().presets.len() > 0 {
                fields.push(Field::Preset);
            }
            fields.push(Field::Quality);
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
        }
        fields.push(Field::Start);
        fields
    }

    pub fn label(field: Field) -> &'static str {
        match field {
            Field::Input => "Input",
            Field::Output => "Output",
            Field::Container => "Container",
            Field::Decoder => "Decoder",
            Field::Action => "Video",
            Field::Encoder => "Encoder",
            Field::Preset => "Preset",
            Field::Quality => "Quality",
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
            Field::Container => container_label(CONTAINERS[self.container]),
            Field::Decoder => DECODERS[self.decoder].label().to_owned(),
            Field::Action => match self.action {
                VideoAction::Copy => "Copy".to_owned(),
                VideoAction::Encode => "Encode".to_owned(),
            },
            Field::Encoder => self.encoder().encoding().to_string(),
            Field::Preset => self.encoder().presets.label(self.preset),
            Field::Quality => format!(
                "{} {}",
                self.encoder().encoding().quality_parameter(),
                self.quality
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
        self.focus = step(self.focus, self.fields().len(), direction);
    }

    fn switch_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Transcode => Mode::Predict,
            Mode::Predict => Mode::Transcode,
        };
        self.clamp_focus();
    }

    fn activate(&mut self) -> FormAction {
        match self.focused() {
            Field::Input => return FormAction::PickInput,
            Field::Output => {
                self.output.begin();
                self.editing = Some(Field::Output);
            }
            Field::Overwrite => self.overwrite = !self.overwrite,
            Field::Verify => self.verify = !self.verify,
            Field::Start => return FormAction::Submit,
            _ => self.adjust(1),
        }
        FormAction::None
    }

    fn adjust(&mut self, direction: isize) {
        match self.focused() {
            Field::Input | Field::Output => {}
            Field::Container => {
                self.container = step(self.container, CONTAINERS.len(), direction);
            }
            Field::Decoder => {
                self.decoder = step(self.decoder, DECODERS.len(), direction);
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
                self.encoder = step(self.encoder, ENCODERS.len(), direction);
                let encoder = self.encoder();
                self.preset = self.preset.min(encoder.presets.len().saturating_sub(1));
                let quality = encoder.encoding().quality_range();
                self.quality = self.quality.clamp(*quality.start(), *quality.end());
            }
            Field::Preset => {
                self.preset = step(self.preset, self.encoder().presets.len(), direction);
            }
            Field::Quality => {
                let quality = self.encoder().encoding().quality_range();
                self.quality = if direction < 0 {
                    self.quality.saturating_sub(1).max(*quality.start())
                } else {
                    self.quality.saturating_add(1).min(*quality.end())
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
                self.vmaf = match step(index, 3, direction) {
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

        let video = if self.mode == Mode::Transcode && self.action == VideoAction::Copy {
            CoreVideoAction::Copy
        } else {
            CoreVideoAction::Encode(self.selected_encoding())
        };
        let decoding = DECODERS[self.decoder].backend();
        let output = match self.mode {
            Mode::Transcode => PathBuf::from(&self.output.value),
            Mode::Predict => PathBuf::new(),
        };
        let mut request = TranscodeRequest::new(input.path(), output)
            .with_decoding(decoding)
            .with_video(video)
            .with_overwrite(self.mode == Mode::Transcode && self.overwrite);
        if let Some(container) = CONTAINERS[self.container] {
            request = request.with_container(container);
        }

        let command = Command {
            request,
            operation: match self.mode {
                Mode::Transcode => Operation::Transcode,
                Mode::Predict => Operation::Predict,
            },
            recursive: input.recursive(),
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
                (Mode::Transcode, VmafMode::Off) | (Mode::Predict, _) => None,
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
        let Field::Output = field else {
            unreachable!("only the output path supports text editing")
        };

        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing = None,
            KeyCode::Left => self.output.move_left(),
            KeyCode::Right => self.output.move_right(),
            KeyCode::Home => self.output.cursor = 0,
            KeyCode::End => self.output.cursor = self.output.value.len(),
            KeyCode::Backspace => self.output.backspace(),
            KeyCode::Delete => self.output.delete(),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.output.value.clear();
                self.output.cursor = 0;
            }
            KeyCode::Char(value)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.output.insert(value);
            }
            _ => {}
        }
    }

    fn clamp_focus(&mut self) {
        self.focus = self.focus.min(self.fields().len() - 1);
    }

    fn selected_encoding(&self) -> VideoEncoding {
        let encoder = self.encoder();
        let mut encoding = encoder.encoding();
        if encoder.presets.len() > 0 {
            encoding
                .try_set_preset(&encoder.presets.label(self.preset))
                .expect("TUI preset choices must be accepted by yog-core");
        }
        encoding.set_quality(self.quality);
        encoding
    }

    fn encoder(&self) -> Encoder {
        ENCODERS[self.encoder]
    }
}

fn step(current: usize, len: usize, direction: isize) -> usize {
    debug_assert!(len > 0);
    if direction < 0 {
        current.checked_sub(1).unwrap_or(len - 1)
    } else {
        (current + 1) % len
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
    fn predict_mode_removes_transcode_only_fields() {
        let mut form = CommandForm::default();
        form.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(form.mode(), Mode::Predict);
        assert_eq!(
            form.source_fields(),
            vec![Field::Input, Field::Container, Field::Decoder]
        );
        assert!(!form.video_fields().contains(&Field::Action));
        assert_eq!(form.option_fields(), vec![Field::Start]);

        form.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(form.mode(), Mode::Transcode);
    }
}
