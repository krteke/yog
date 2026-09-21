use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use yog_core::ffmpeg::{encoding::VideoEncoding, plan::Container};

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
const DECODERS: &[&str] = &["Software", "VAAPI", "CUDA", "QSV"];
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
pub(crate) enum Field {
    Input,
    Output,
    Container,
    Decoder,
    Action,
    Encoder,
    Preset,
    Quality,
    Recursive,
    Overwrite,
    Verify,
    Vmaf,
    VmafInterval,
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

pub(crate) struct App {
    input: TextInput,
    output: TextInput,
    container: usize,
    decoder: usize,
    action: VideoAction,
    encoder: usize,
    preset: usize,
    quality: u8,
    recursive: bool,
    overwrite: bool,
    verify: bool,
    vmaf: VmafMode,
    vmaf_interval: u32,
    focus: usize,
    editing: Option<Field>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            input: TextInput::default(),
            output: TextInput::default(),
            container: 0,
            decoder: 0,
            action: VideoAction::Encode,
            encoder: 1,
            preset: 5,
            quality: 23,
            recursive: false,
            overwrite: false,
            verify: false,
            vmaf: VmafMode::Off,
            vmaf_interval: 5,
            focus: 0,
            editing: None,
        }
    }
}

impl App {
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> bool {
        if let Some(field) = self.editing {
            self.handle_text_key(field, key);
            return false;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => self.move_focus(1),
            KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => self.move_focus(-1),
            KeyCode::Char('h') | KeyCode::Left => self.adjust(-1),
            KeyCode::Char('l') | KeyCode::Right => self.adjust(1),
            KeyCode::Char('g') => self.focus = 0,
            KeyCode::Char('G') => self.focus = self.fields().len() - 1,
            KeyCode::Char(' ') => self.activate(),
            KeyCode::Enter => self.activate(),
            _ => {}
        }
        false
    }

    pub(crate) fn focused(&self) -> Field {
        self.fields()[self.focus]
    }

    pub(crate) fn source_fields(&self) -> &'static [Field] {
        &[
            Field::Input,
            Field::Output,
            Field::Container,
            Field::Decoder,
        ]
    }

    pub(crate) fn video_fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Action];
        if self.action == VideoAction::Encode {
            fields.push(Field::Encoder);
            if self.encoder().presets.len() > 0 {
                fields.push(Field::Preset);
            }
            fields.push(Field::Quality);
        }
        fields
    }

    pub(crate) fn option_fields(&self) -> Vec<Field> {
        let mut fields = vec![
            Field::Recursive,
            Field::Overwrite,
            Field::Verify,
            Field::Vmaf,
        ];
        if self.vmaf == VmafMode::Subsample {
            fields.push(Field::VmafInterval);
        }
        fields
    }

    pub(crate) fn label(field: Field) -> &'static str {
        match field {
            Field::Input => "Input",
            Field::Output => "Output",
            Field::Container => "Container",
            Field::Decoder => "Decoder",
            Field::Action => "Video",
            Field::Encoder => "Encoder",
            Field::Preset => "Preset",
            Field::Quality => "Quality",
            Field::Recursive => "Recursive",
            Field::Overwrite => "Overwrite",
            Field::Verify => "Verify",
            Field::Vmaf => "VMAF",
            Field::VmafInterval => "N subsample",
        }
    }

    pub(crate) fn value(&self, field: Field) -> String {
        match field {
            Field::Input => self.input.display(self.editing == Some(field)),
            Field::Output => self.output.display(self.editing == Some(field)),
            Field::Container => container_label(CONTAINERS[self.container]),
            Field::Decoder => DECODERS[self.decoder].to_owned(),
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
            Field::Recursive => state(self.recursive).to_owned(),
            Field::Overwrite => state(self.overwrite).to_owned(),
            Field::Verify => state(self.verify).to_owned(),
            Field::Vmaf => match self.vmaf {
                VmafMode::Off => "Off".to_owned(),
                VmafMode::Full => "Full".to_owned(),
                VmafMode::Subsample => "Subsample".to_owned(),
            },
            Field::VmafInterval => self.vmaf_interval.to_string(),
        }
    }

    pub(crate) fn is_editing(&self, field: Field) -> bool {
        self.editing == Some(field)
    }

    fn fields(&self) -> Vec<Field> {
        let mut fields = self.source_fields().to_vec();
        fields.extend(self.video_fields());
        fields.extend(self.option_fields());
        fields
    }

    fn move_focus(&mut self, direction: isize) {
        let len = self.fields().len();
        self.focus = step(self.focus, len, direction);
    }

    fn activate(&mut self) {
        match self.focused() {
            Field::Input => {
                self.input.begin();
                self.editing = Some(Field::Input);
            }
            Field::Output => {
                self.output.begin();
                self.editing = Some(Field::Output);
            }
            Field::Recursive => self.recursive = !self.recursive,
            Field::Overwrite => self.overwrite = !self.overwrite,
            Field::Verify => self.verify = !self.verify,
            _ => self.adjust(1),
        }
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
            Field::Recursive => self.recursive = direction > 0,
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
        }
    }

    fn handle_text_key(&mut self, field: Field, key: KeyEvent) {
        let input = match field {
            Field::Input => &mut self.input,
            Field::Output => &mut self.output,
            _ => unreachable!("only path fields enter text editing"),
        };

        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing = None,
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
        let mut app = App::default();
        assert_eq!(app.focused(), Field::Input);

        app.handle_key(key('j'));
        assert_eq!(app.focused(), Field::Output);

        app.handle_key(key('j'));
        assert_eq!(app.focused(), Field::Container);
        assert_eq!(app.value(Field::Container), "Auto");

        app.handle_key(key('l'));
        assert_eq!(app.value(Field::Container), "MKV");

        app.handle_key(key('k'));
        assert_eq!(app.focused(), Field::Output);
    }

    #[test]
    fn path_editing_keeps_vim_keys_as_text_until_editing_ends() {
        let mut app = App::default();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(key('h'));
        app.handle_key(key('j'));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.value(Field::Input), "hj");
        assert!(app.handle_key(key('q')));
    }
}
