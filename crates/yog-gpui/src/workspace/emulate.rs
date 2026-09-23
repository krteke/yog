use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath, StyledExt as _,
    button::{Button, ButtonVariants},
    form::{Field, Form},
    input::{Input, InputState},
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectState},
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, prelude::FluentBuilder as _,
};

use super::{components, source::SourcePicker};

#[derive(Clone, Copy)]
enum ImageFormat {
    Png,
    Svg,
}

pub struct EmulatePage {
    source: Entity<SourcePicker>,
    decoder: Entity<SelectState<SearchableVec<&'static str>>>,
    container: Entity<SelectState<SearchableVec<&'static str>>>,
    encoder: Entity<SelectState<SearchableVec<&'static str>>>,
    preset: Entity<SelectState<SearchableVec<&'static str>>>,
    quality_points: Entity<InputState>,
    png: Entity<InputState>,
    svg: Entity<InputState>,
    destination_error: Option<String>,
}

impl EmulatePage {
    pub fn new(source: Entity<SourcePicker>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut select = |items, selected, cx: &mut Context<Self>| {
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(items),
                    Some(IndexPath::new(selected)),
                    window,
                    cx,
                )
            })
        };
        Self {
            source,
            decoder: select(vec!["Software", "VAAPI", "CUDA", "QSV"], 0, cx),
            container: select(vec!["Auto", "Matroska", "MP4"], 0, cx),
            encoder: select(vec!["x264", "x265"], 1, cx),
            preset: select(vec!["fast", "medium", "slow", "slower"], 1, cx),
            quality_points: cx.new(|cx| InputState::new(window, cx).default_value("20,24,28,32")),
            png: cx.new(|cx| InputState::new(window, cx).placeholder("Save PNG chart")),
            svg: cx.new(|cx| InputState::new(window, cx).placeholder("Save SVG chart")),
            destination_error: None,
        }
    }

    fn choose_destination(
        &mut self,
        format: ImageFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let directory = self
            .source
            .read(cx)
            .path()
            .and_then(|path| path.parent())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = self
            .source
            .read(cx)
            .path()
            .and_then(|path| path.file_stem())
            .and_then(|stem| stem.to_str())
            .unwrap_or("output");
        let suggested_name = match format {
            ImageFormat::Png => format!("{name}-curve.png"),
            ImageFormat::Svg => format!("{name}-curve.svg"),
        };
        let response = cx.prompt_for_new_path(&directory, Some(&suggested_name));

        cx.spawn_in(window, async move |this, cx| match response.await {
            Ok(Ok(Some(path))) => {
                _ = this.update_in(cx, |this, window, cx| {
                    let input = match format {
                        ImageFormat::Png => &this.png,
                        ImageFormat::Svg => &this.svg,
                    };
                    input.update(cx, |input, cx| {
                        input.set_value(path.to_string_lossy().to_string(), window, cx);
                    });
                    this.destination_error = None;
                    cx.notify();
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

    fn render_work_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(self.source.clone())
                    .child(div().font_medium().child("Results"))
                    .child(components::empty_results(cx)),
            )
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let form = Form::new()
            .child(
                Field::new()
                    .label("Decoder")
                    .child(Select::new(&self.decoder).w_full()),
            )
            .child(
                Field::new()
                    .label("Container")
                    .child(Select::new(&self.container).w_full()),
            )
            .child(
                Field::new()
                    .label("Encoder")
                    .child(Select::new(&self.encoder).w_full()),
            )
            .child(
                Field::new()
                    .label("Preset")
                    .child(Select::new(&self.preset).w_full()),
            )
            .child(
                Field::new()
                    .label("Quality points (CRF)")
                    .child(Input::new(&self.quality_points)),
            )
            .child(components::path_field(
                "PNG chart",
                "browse-png",
                &self.png,
                cx.listener(|this, _, window, cx| {
                    this.choose_destination(ImageFormat::Png, window, cx)
                }),
            ))
            .child(components::path_field(
                "SVG chart",
                "browse-svg",
                &self.svg,
                cx.listener(|this, _, window, cx| {
                    this.choose_destination(ImageFormat::Svg, window, cx)
                }),
            ));

        let body = div()
            .size_full()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(div().p_5().child(form));
        let footer = div()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(self.destination_error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                div().flex().justify_end().child(
                    Button::new("emulate")
                        .primary()
                        .label("Generate chart")
                        .disabled(true),
                ),
            );
        components::inspector(body.into_any_element(), footer.into_any_element(), cx)
    }
}

impl Render for EmulatePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let work_area = self.render_work_area(cx).into_any_element();
        let inspector = self.render_inspector(cx).into_any_element();
        components::page_layout("emulate-panes", "Emulate", work_area, inspector, cx)
    }
}
