use gpui_kit::component::{
    Disableable, IndexPath, StyledExt as _,
    button::{Button, ButtonVariants},
    form::{Field, Form},
    input::{Input, InputState},
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectState},
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div,
};

use super::{components, source::SourcePicker};

pub struct PredictPage {
    source: Entity<SourcePicker>,
    decoder: Entity<SelectState<SearchableVec<&'static str>>>,
    container: Entity<SelectState<SearchableVec<&'static str>>>,
    encoder: Entity<SelectState<SearchableVec<&'static str>>>,
    preset: Entity<SelectState<SearchableVec<&'static str>>>,
    quality: Entity<InputState>,
}

impl PredictPage {
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
            quality: cx.new(|cx| InputState::new(window, cx).default_value("28")),
        }
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
                    .label("Quality (CRF)")
                    .child(Input::new(&self.quality)),
            );

        let body = div()
            .size_full()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(div().p_5().child(form));
        let footer = div().flex().justify_end().child(
            Button::new("predict")
                .primary()
                .label("Predict")
                .disabled(true),
        );
        components::inspector(body.into_any_element(), footer.into_any_element(), cx)
    }
}

impl Render for PredictPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let work_area = self.render_work_area(cx).into_any_element();
        let inspector = self.render_inspector(cx).into_any_element();
        components::page_layout("predict-panes", "Predict", work_area, inspector, cx)
    }
}
