use std::{ffi::OsString, path::PathBuf};

use gpui_kit::component::{
    IndexPath,
    form::{Field, Form},
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window,
};
use yog_core::ffmpeg::{
    decoding::DecodingBackend,
    encoding::{DEFAULT_VAAPI_DEVICE, NvencMultipass, VideoEncoding},
    plan::Container,
};

pub type Choice = Entity<SelectState<SearchableVec<&'static str>>>;

pub const DECODERS: &[&str] = &["Software", "VAAPI", "CUDA", "QSV"];
pub const ENCODERS: &[&str] = &[
    "libx264",
    "libx265",
    "libsvtav1",
    "libaom-av1",
    "librav1e",
    "h264_nvenc",
    "hevc_nvenc",
    "av1_nvenc",
    "h264_qsv",
    "hevc_qsv",
    "av1_qsv",
    "h264_vaapi",
    "hevc_vaapi",
    "av1_vaapi",
];
pub const CONTAINERS: &[(&str, Option<Container>)] = &[
    ("Auto", None),
    ("Matroska", Some(Container::Matroska)),
    ("MP4", Some(Container::Mp4)),
    ("MOV", Some(Container::Mov)),
    ("M4A", Some(Container::M4a)),
    ("3GP", Some(Container::ThreeGp)),
    ("3G2", Some(Container::ThreeG2)),
    ("F4V", Some(Container::F4v)),
    ("ISMV", Some(Container::Ismv)),
    ("PSP", Some(Container::Psp)),
    ("WebM", Some(Container::Webm)),
    ("MPEG-TS", Some(Container::MpegTs)),
    ("M2TS", Some(Container::M2ts)),
    ("AVI", Some(Container::Avi)),
    ("FLV", Some(Container::Flv)),
    ("ASF", Some(Container::Asf)),
    ("WMV", Some(Container::Wmv)),
    ("MPEG-PS", Some(Container::MpegPs)),
    ("VOB", Some(Container::Vob)),
    ("Ogg", Some(Container::Ogg)),
    ("OGV", Some(Container::Ogv)),
];

pub const X26X_PRESETS: &[&str] = &[
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
const SVT_PRESETS: &[&str] = &[
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13",
];
const AOM_PRESETS: &[&str] = &["0", "1", "2", "3", "4", "5", "6", "7", "8"];
const RAV1E_PRESETS: &[&str] = &["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"];
const NVENC_PRESETS: &[&str] = &["p1", "p2", "p3", "p4", "p5", "p6", "p7"];
const QSV_PRESETS: &[&str] = &[
    "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow",
];
pub const MULTIPASS: &[&str] = &[
    "Default",
    "Disabled",
    "Quarter resolution",
    "Full resolution",
];

pub fn presets(encoder: &str) -> &'static [&'static str] {
    match encoder {
        "libx264" | "libx265" => X26X_PRESETS,
        "libsvtav1" => SVT_PRESETS,
        "libaom-av1" => AOM_PRESETS,
        "librav1e" => RAV1E_PRESETS,
        "h264_nvenc" | "hevc_nvenc" | "av1_nvenc" => NVENC_PRESETS,
        "h264_qsv" | "hevc_qsv" | "av1_qsv" => QSV_PRESETS,
        "h264_vaapi" | "hevc_vaapi" | "av1_vaapi" => &[],
        _ => unreachable!("encoder choice is internal"),
    }
}

pub fn default_preset(encoder: &str) -> usize {
    match encoder {
        "libx264" | "libx265" => 5,
        "libsvtav1" => 6,
        "libaom-av1" => 4,
        "librav1e" => 6,
        "h264_nvenc" | "hevc_nvenc" | "av1_nvenc" => 3,
        _ => 3,
    }
}

pub fn selected(choice: &Choice, cx: &App) -> &'static str {
    choice
        .read(cx)
        .selected_value()
        .expect("select choices always have a selection")
}

pub fn container(label: &str) -> Option<Container> {
    CONTAINERS
        .iter()
        .find(|(name, _)| *name == label)
        .expect("container choice is internal")
        .1
}

pub fn decoding(label: &str, device: &str) -> DecodingBackend {
    let device = (!device.trim().is_empty()).then(|| OsString::from(device.trim()));
    match label {
        "Software" => DecodingBackend::Software,
        "VAAPI" => DecodingBackend::Vaapi(device),
        "CUDA" => DecodingBackend::Cuda(device),
        "QSV" => DecodingBackend::Qsv(device),
        _ => unreachable!("decoder choice is internal"),
    }
}

pub fn encoding(name: &str, preset: &str, multipass: &str, device: &str) -> VideoEncoding {
    let mut encoding = name
        .parse::<VideoEncoding>()
        .expect("encoder choices are accepted by yog-core");
    if !presets(name).is_empty() {
        encoding
            .try_set_preset(preset)
            .expect("preset choices are accepted by yog-core");
    }
    if let VideoEncoding::Nvenc {
        multipass: selected,
        ..
    } = &mut encoding
    {
        *selected = match multipass {
            "Default" => None,
            "Disabled" => Some(NvencMultipass::Disabled),
            "Quarter resolution" => Some(NvencMultipass::QuarterResolution),
            "Full resolution" => Some(NvencMultipass::FullResolution),
            _ => unreachable!("multipass choice is internal"),
        };
    }
    if let VideoEncoding::Vaapi { device: target, .. } = &mut encoding {
        *target = PathBuf::from(device.trim());
    }
    encoding
}

pub struct VideoInputs {
    decoder: Choice,
    decode_device: Entity<InputState>,
    encoder: Choice,
    preset: Choice,
    multipass: Choice,
    encode_device: Entity<InputState>,
    _encoder_subscription: Subscription,
    _decoder_subscription: Subscription,
}

impl VideoInputs {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut select = |items: &[&'static str], index, cx: &mut Context<Self>| {
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(items.to_vec()),
                    Some(IndexPath::new(index)),
                    window,
                    cx,
                )
            })
        };
        let decoder = select(DECODERS, 0, cx);
        let encoder = select(ENCODERS, 1, cx);
        let preset = select(X26X_PRESETS, 5, cx);
        let multipass = select(MULTIPASS, 0, cx);
        let encoder_subscription = cx.subscribe_in(
            &encoder,
            window,
            |this, _, _: &SelectEvent<SearchableVec<&'static str>>, window, cx| {
                let values = presets(selected(&this.encoder, cx));
                if !values.is_empty() {
                    let index = default_preset(selected(&this.encoder, cx));
                    this.preset.update(cx, |preset, cx| {
                        preset.set_items(SearchableVec::new(values.to_vec()), window, cx);
                        preset.set_selected_index(Some(IndexPath::new(index)), window, cx);
                    });
                }
                cx.notify();
            },
        );
        let decoder_subscription = cx.subscribe_in(
            &decoder,
            window,
            |_, _, _: &SelectEvent<SearchableVec<&'static str>>, _, cx| cx.notify(),
        );
        Self {
            decoder,
            decode_device: cx
                .new(|cx| InputState::new(window, cx).placeholder("Device (optional)")),
            encoder,
            preset,
            multipass,
            encode_device: cx
                .new(|cx| InputState::new(window, cx).default_value(DEFAULT_VAAPI_DEVICE)),
            _encoder_subscription: encoder_subscription,
            _decoder_subscription: decoder_subscription,
        }
    }

    pub fn build(&self, cx: &App) -> (DecodingBackend, VideoEncoding) {
        (
            decoding(
                selected(&self.decoder, cx),
                &self.decode_device.read(cx).value(),
            ),
            encoding(
                selected(&self.encoder, cx),
                selected(&self.preset, cx),
                selected(&self.multipass, cx),
                &self.encode_device.read(cx).value(),
            ),
        )
    }

    pub fn quality_parameter(&self, cx: &App) -> &'static str {
        selected(&self.encoder, cx)
            .parse::<VideoEncoding>()
            .expect("encoder choices are accepted by yog-core")
            .quality_parameter()
    }
}

impl Render for VideoInputs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let encoder = selected(&self.encoder, cx);
        let mut form = Form::new()
            .child(
                Field::new()
                    .label("Decoder")
                    .child(Select::new(&self.decoder).w_full()),
            )
            .child(
                Field::new()
                    .label("Encoder")
                    .child(Select::new(&self.encoder).w_full()),
            );
        if selected(&self.decoder, cx) != "Software" {
            form = form.child(
                Field::new()
                    .label("Decode device")
                    .child(Input::new(&self.decode_device)),
            );
        }
        if !presets(encoder).is_empty() {
            form = form.child(
                Field::new()
                    .label("Preset")
                    .child(Select::new(&self.preset).w_full()),
            );
        }
        if encoder.ends_with("_nvenc") {
            form = form.child(
                Field::new()
                    .label("Multipass")
                    .child(Select::new(&self.multipass).w_full()),
            );
        }
        if encoder.ends_with("_vaapi") {
            form = form.child(
                Field::new()
                    .label("Encode device")
                    .child(Input::new(&self.encode_device)),
            );
        }
        form
    }
}
