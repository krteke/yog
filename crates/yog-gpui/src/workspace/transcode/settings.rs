use std::{
    num::{NonZeroU32, NonZeroU64},
    path::{Path, PathBuf},
};

use gpui_kit::component::{
    ActiveTheme, IndexPath, StyledExt as _,
    checkbox::Checkbox,
    form::{Field, Form},
    input::{Input, InputState},
    radio::RadioGroup,
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use yog_core::ffmpeg::{
    encoding::{DEFAULT_VAAPI_DEVICE, RateControl, VideoEncoding},
    plan::{Container, TranscodeRequest, VideoAction},
    vmaf::VmafOptions,
};
use yog_runtime::{Command, Config, Operation, Options, Validate};

use super::super::{
    components,
    execution::{self, RunRequest},
    source::SourcePicker,
    video::{
        CONTAINERS, Choice, DECODERS, ENCODERS, MULTIPASS, X26X_PRESETS, container, decoding,
        default_preset, encoding, presets, selected,
    },
};

#[derive(Clone, Copy)]
enum Destination {
    Output,
    Report,
}

pub struct TranscodeSettings {
    source: Entity<SourcePicker>,
    output: Entity<InputState>,
    report: Entity<InputState>,
    decoder: Choice,
    decode_device: Entity<InputState>,
    container: Choice,
    encoder: Choice,
    preset: Choice,
    rate: Choice,
    quality: Entity<InputState>,
    bitrate: Entity<InputState>,
    multipass: Choice,
    encode_device: Entity<InputState>,
    vmaf: Choice,
    n_subsample: Entity<InputState>,
    encode: bool,
    overwrite: bool,
    verify: bool,
    destination_error: Option<String>,
    _output_observation: Subscription,
    _encoder_subscription: Subscription,
    _decoder_subscription: Subscription,
    _rate_subscription: Subscription,
    _vmaf_subscription: Subscription,
}

impl TranscodeSettings {
    pub fn new(source: Entity<SourcePicker>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut select = |items: &[&'static str], selected, cx: &mut Context<Self>| {
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(items.to_vec()),
                    Some(IndexPath::new(selected)),
                    window,
                    cx,
                )
            })
        };
        let decoder = select(DECODERS, 0, cx);
        let container_labels = CONTAINERS
            .iter()
            .map(|(label, _)| *label)
            .collect::<Vec<_>>();
        let container = select(&container_labels, 0, cx);
        let encoder = select(ENCODERS, 1, cx);
        let preset = select(X26X_PRESETS, 5, cx);
        let rate = select(&["Quality", "Bitrate", "Encoder default"], 0, cx);
        let multipass = select(MULTIPASS, 0, cx);
        let vmaf = select(&["Off", "Full", "Subsample"], 0, cx);

        let encoder_subscription = cx.subscribe_in(
            &encoder,
            window,
            |this, _, _: &SelectEvent<SearchableVec<&'static str>>, window, cx| {
                let encoder = selected(&this.encoder, cx);
                let values = presets(encoder);
                if !values.is_empty() {
                    let default = default_preset(encoder);
                    this.preset.update(cx, |preset, cx| {
                        preset.set_items(SearchableVec::new(values.to_vec()), window, cx);
                        preset.set_selected_index(Some(IndexPath::new(default)), window, cx);
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
        let rate_subscription = cx.subscribe_in(
            &rate,
            window,
            |_, _, _: &SelectEvent<SearchableVec<&'static str>>, _, cx| cx.notify(),
        );
        let vmaf_subscription = cx.subscribe_in(
            &vmaf,
            window,
            |_, _, _: &SelectEvent<SearchableVec<&'static str>>, _, cx| cx.notify(),
        );
        let output = cx.new(|cx| InputState::new(window, cx).placeholder("Output file or folder"));
        let output_observation = cx.observe(&output, |_, _, cx| cx.notify());

        Self {
            source,
            output,
            report: cx.new(|cx| InputState::new(window, cx).placeholder("Optional report path")),
            decoder,
            decode_device: cx
                .new(|cx| InputState::new(window, cx).placeholder("Device (optional)")),
            container,
            encoder,
            preset,
            rate,
            quality: cx.new(|cx| InputState::new(window, cx).default_value("28")),
            bitrate: cx.new(|cx| InputState::new(window, cx).placeholder("Bits per second")),
            multipass,
            encode_device: cx
                .new(|cx| InputState::new(window, cx).default_value(DEFAULT_VAAPI_DEVICE)),
            vmaf,
            n_subsample: cx.new(|cx| InputState::new(window, cx).default_value("5")),
            encode: true,
            overwrite: false,
            verify: false,
            destination_error: None,
            _output_observation: output_observation,
            _encoder_subscription: encoder_subscription,
            _decoder_subscription: decoder_subscription,
            _rate_subscription: rate_subscription,
            _vmaf_subscription: vmaf_subscription,
        }
    }

    pub fn has_output(&self, cx: &gpui_kit::App) -> bool {
        !self.output.read(cx).value().trim().is_empty()
    }

    pub fn build(&self, cx: &gpui_kit::App) -> Result<RunRequest, String> {
        let source = self.source.read(cx);
        let input = source
            .valid_path()
            .ok_or_else(|| "Select an input file or folder".to_owned())?;
        let output = self.output.read(cx).value();
        let output = output.trim();
        if output.is_empty() {
            return Err("Choose an output file or folder".into());
        }

        let decoding = decoding(
            selected(&self.decoder, cx),
            &self.decode_device.read(cx).value(),
        );

        let video = if self.encode {
            let mut encoding = encoding(
                selected(&self.encoder, cx),
                selected(&self.preset, cx),
                selected(&self.multipass, cx),
                &self.encode_device.read(cx).value(),
            );
            let rate = match selected(&self.rate, cx) {
                "Quality" => {
                    let quality = self.quality.read(cx).value();
                    Some(RateControl::Quality(quality.trim().parse::<u8>().map_err(
                        |_| "Quality must be an integer from 0 to 255".to_owned(),
                    )?))
                }
                "Bitrate" => {
                    let bitrate = self.bitrate.read(cx).value();
                    Some(RateControl::Bitrate(
                        bitrate.trim().parse::<NonZeroU64>().map_err(|_| {
                            "Bitrate must be an integer greater than zero".to_owned()
                        })?,
                    ))
                }
                "Encoder default" => None,
                _ => unreachable!("rate choice is internal"),
            };
            encoding.set_rate(rate);
            VideoAction::Encode(encoding)
        } else {
            VideoAction::Copy
        };

        let mut request = TranscodeRequest::new(input, output)
            .with_decoding(decoding)
            .with_video(video)
            .with_overwrite(self.overwrite);
        if let Some(container) = container(selected(&self.container, cx)) {
            request = request.with_container(container);
        }
        let command = Command {
            request,
            operation: Operation::Transcode,
            recursive: source.is_directory(),
        };
        command.validate().map_err(|error| format!("{error:#}"))?;

        let vmaf = match selected(&self.vmaf, cx) {
            "Off" => None,
            "Full" => Some(VmafOptions::default()),
            "Subsample" => {
                let interval = self.n_subsample.read(cx).value();
                let interval = interval
                    .trim()
                    .parse::<NonZeroU32>()
                    .map_err(|_| "N subsample must be an integer greater than zero".to_owned())?;
                Some(VmafOptions {
                    n_subsample: Some(interval),
                })
            }
            _ => unreachable!("VMAF choice is internal"),
        };
        let report = self.report.read(cx).value();
        let report = (!report.trim().is_empty()).then(|| PathBuf::from(report.trim()));
        if report.as_ref().is_some_and(|report| {
            execution::same_path(report, input) || execution::same_path(report, Path::new(output))
        }) {
            return Err("Report path must differ from the source and output".into());
        }
        let options = Options {
            verify: self.verify,
            vmaf,
            terminal_output: false,
            report,
            ..Options::default()
        };
        let config = Config::load(None).map_err(|error| format!("{error:#}"))?;
        Ok(RunRequest {
            command,
            options,
            config,
        })
    }

    fn choose_destination(
        &mut self,
        target: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self.source.read(cx);
        if matches!(target, Destination::Output) && source.is_directory() {
            let response = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Choose output folder".into()),
            });
            cx.spawn_in(window, async move |this, cx| match response.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        _ = this.update_in(cx, |this, window, cx| {
                            this.set_destination(target, path, window, cx);
                        });
                    }
                }
                Ok(Ok(None)) => {}
                result => {
                    _ = this.update_in(cx, |this, _, cx| {
                        this.destination_error =
                            Some(format!("Could not open folder picker: {result:?}"));
                        cx.notify();
                    });
                }
            })
            .detach();
            return;
        }

        let directory = source
            .path()
            .and_then(|path| path.parent())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = source
            .path()
            .and_then(|path| path.file_stem())
            .and_then(|stem| stem.to_str())
            .unwrap_or("output");
        let suggested_name = match target {
            Destination::Output => {
                let extension =
                    container(selected(&self.container, cx)).map_or("mkv", Container::extension);
                format!("{name}-encoded.{extension}")
            }
            Destination::Report => format!("{name}-report.jsonl"),
        };
        let response = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        cx.spawn_in(window, async move |this, cx| match response.await {
            Ok(Ok(Some(path))) => {
                _ = this.update_in(cx, |this, window, cx| {
                    this.set_destination(target, path, window, cx);
                });
            }
            Ok(Ok(None)) => {}
            result => {
                _ = this.update_in(cx, |this, _, cx| {
                    this.destination_error =
                        Some(format!("Could not open save dialog: {result:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn set_destination(
        &mut self,
        target: Destination,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = match target {
            Destination::Output => &self.output,
            Destination::Report => &self.report,
        };
        input.update(cx, |input, cx| {
            input.set_value(path.to_string_lossy().to_string(), window, cx)
        });
        self.destination_error = None;
        cx.notify();
    }
}

impl Render for TranscodeSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let encoder = selected(&self.encoder, cx);
        let is_nvenc = encoder.ends_with("_nvenc");
        let is_vaapi = encoder.ends_with("_vaapi");
        let has_preset = !presets(encoder).is_empty();
        let quality_parameter = encoder
            .parse::<VideoEncoding>()
            .expect("encoder choices are accepted by yog-core")
            .quality_parameter();

        let mut video = Form::new().child(
            Field::new().label("Action").child(
                RadioGroup::new("video-action")
                    .children(["Encode", "Copy"])
                    .selected_index(Some(if self.encode { 0 } else { 1 }))
                    .on_change(cx.listener(|this, next, _, cx| {
                        this.encode = *next == 0;
                        cx.notify();
                    })),
            ),
        );
        if self.encode {
            video = video.child(
                Field::new()
                    .label("Encoder")
                    .child(Select::new(&self.encoder).w_full()),
            );
            if has_preset {
                video = video.child(
                    Field::new()
                        .label("Preset")
                        .child(Select::new(&self.preset).w_full()),
                );
            }
            video = video.child(
                Field::new()
                    .label("Rate control")
                    .child(Select::new(&self.rate).w_full()),
            );
            match selected(&self.rate, cx) {
                "Quality" => {
                    video = video.child(
                        Field::new()
                            .label(format!("Quality ({quality_parameter})"))
                            .child(Input::new(&self.quality)),
                    );
                }
                "Bitrate" => {
                    video = video.child(
                        Field::new()
                            .label("Bitrate (bps)")
                            .child(Input::new(&self.bitrate)),
                    );
                }
                _ => {}
            }
            if is_nvenc {
                video = video.child(
                    Field::new()
                        .label("Multipass")
                        .child(Select::new(&self.multipass).w_full()),
                );
            }
            if is_vaapi {
                video = video.child(
                    Field::new()
                        .label("Encode device")
                        .child(Input::new(&self.encode_device)),
                );
            }
        }

        let mut finishing = Form::new()
            .child(
                Field::new().label_indent(false).child(
                    Checkbox::new("overwrite")
                        .label("Overwrite existing output")
                        .checked(self.overwrite)
                        .on_change(cx.listener(|this, next, _, cx| {
                            this.overwrite = *next;
                            cx.notify();
                        })),
                ),
            )
            .child(
                Field::new().label_indent(false).child(
                    Checkbox::new("verify")
                        .label("Verify after encoding")
                        .checked(self.verify)
                        .on_change(cx.listener(|this, next, _, cx| {
                            this.verify = *next;
                            cx.notify();
                        })),
                ),
            )
            .child(
                Field::new()
                    .label("VMAF")
                    .child(Select::new(&self.vmaf).w_full()),
            );
        if selected(&self.vmaf, cx) == "Subsample" {
            finishing = finishing.child(
                Field::new()
                    .label("N subsample")
                    .child(Input::new(&self.n_subsample)),
            );
        }
        finishing = finishing.child(components::path_field(
            "Report",
            "browse-report",
            &self.report,
            cx.listener(|this, _, window, cx| {
                this.choose_destination(Destination::Report, window, cx)
            }),
        ));

        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .pb_4()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(div().font_medium().child("Output"))
                            .child(
                                Form::new()
                                    .child(components::path_field(
                                        "Destination",
                                        "browse-output",
                                        &self.output,
                                        cx.listener(|this, _, window, cx| {
                                            this.choose_destination(Destination::Output, window, cx)
                                        }),
                                    ))
                                    .child(
                                        Field::new()
                                            .label("Container")
                                            .child(Select::new(&self.container).w_full()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .pb_4()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(div().font_medium().child("Video"))
                            .child(video),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .pb_4()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(div().font_medium().child("Processing"))
                            .child(
                                Form::new()
                                    .child(
                                        Field::new()
                                            .label("Decoder")
                                            .child(Select::new(&self.decoder).w_full()),
                                    )
                                    .when(selected(&self.decoder, cx) != "Software", |form| {
                                        form.child(
                                            Field::new()
                                                .label("Decode device")
                                                .child(Input::new(&self.decode_device)),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(div().font_medium().child("After encoding"))
                            .child(finishing),
                    )
                    .when_some(self.destination_error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    }),
            )
    }
}
