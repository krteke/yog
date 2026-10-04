use gpui_kit::component::{
    ActiveTheme, Icon, IconName, StyledExt as _,
    button::Button,
    form::Field,
    input::{Input, InputState},
    resizable::v_resizable,
    resizable::{h_resizable, resizable_panel},
    scroll::Scrollbar,
};
use gpui_kit::{
    AnyElement, App, ClickEvent, Entity, InteractiveElement as _, IntoElement, ListState,
    ParentElement as _, Pixels, Styled as _, Window, canvas, div, list, px,
};

use super::messages::Messages;

/// A bounded viewport for variable-height items, with the component scrollbar.
pub fn virtual_list(
    id: &'static str,
    state: ListState,
    estimated_item_height: Pixels,
    mut render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> impl IntoElement {
    let heights = state.clone();
    let viewport = div()
        .id(id)
        .size_full()
        .min_w_0()
        .min_h_0()
        .relative()
        .overflow_hidden()
        .child(
            list(state.clone(), move |index, window, cx| {
                // List items are independent layout roots; stretch their contents explicitly.
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(render_item(index, window, cx))
                    .into_any_element()
            })
            .size_full(),
        )
        .child(
            canvas(
                move |_, _, _| {
                    // GPUI discards height hints during width changes. Restore them after
                    // list layout, before the scrollbar reads the total content height.
                    // ponytail: this rebuilds O(n) lightweight height entries after inserts
                    // or resizes; switch to range hints if GPUI exposes that API.
                    if heights.is_scrolled_to_end().is_none() {
                        heights.with_uniform_item_height(estimated_item_height);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute(),
        )
        .child(Scrollbar::vertical(&state));
    #[cfg(test)]
    let viewport = {
        use gpui_kit::test::TestSupportExt as _;
        viewport.test_support()
    };
    viewport
}

pub fn page_layout(
    id: &'static str,
    title: &'static str,
    work_area: AnyElement,
    inspector: AnyElement,
    cx: &App,
) -> impl IntoElement {
    div()
        .size_full()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .px_6()
                .py_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .text_xl()
                .font_medium()
                .child(title),
        )
        .child(
            h_resizable(id)
                .child(
                    resizable_panel()
                        .size_range(cx.theme().font_size * 20.0..Pixels::MAX)
                        .child(work_area),
                )
                .child(
                    resizable_panel()
                        .size(px(300.))
                        .size_range(px(200.)..px(420.))
                        .flex_none()
                        .child(inspector),
                ),
        )
}

pub fn path_field(
    label: &'static str,
    id: &'static str,
    input: &Entity<InputState>,
    on_browse: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Field {
    Field::new().label(label).child(
        div()
            .flex()
            .gap_2()
            .min_w_0()
            .child(Input::new(input).flex_1().min_w_0())
            .child(Button::new(id).label("Browse…").on_click(on_browse)),
    )
}

pub fn inspector(body: AnyElement, footer: AnyElement, cx: &App) -> impl IntoElement {
    div()
        .size_full()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(cx.theme().background)
        .child(
            div()
                .px_5()
                .py_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .font_medium()
                .child("Settings"),
        )
        .child(div().flex_1().min_h_0().child(body))
        .child(
            div()
                .px_5()
                .py_4()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(footer),
        )
}

pub fn empty_results(cx: &App) -> impl IntoElement {
    div()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().group_box)
        .min_h_48()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::Inbox).size_6())
        .child(
            div()
                .font_medium()
                .text_color(cx.theme().foreground)
                .child("No results yet"),
        )
}

pub fn work_and_messages(
    id: &'static str,
    results: AnyElement,
    messages: Entity<Messages>,
    cx: &App,
) -> AnyElement {
    if messages.read(cx).is_expanded() {
        let rem = cx.theme().font_size;
        v_resizable(id)
            .child(resizable_panel().child(results))
            .child(
                resizable_panel()
                    .size(rem * 18.0)
                    .size_range(rem * 12.0..rem * 32.0)
                    .flex_none()
                    .child(messages),
            )
            .into_any_element()
    } else {
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(results))
            .child(messages)
            .into_any_element()
    }
}

pub fn format_size(bytes: u64) -> String {
    let mib = bytes as f64 / 1_048_576.0;
    if mib >= 1_024.0 {
        format!("{:.1} GiB", mib / 1_024.0)
    } else if mib >= 10.0 {
        format!("{mib:.0} MiB")
    } else {
        format!("{mib:.1} MiB")
    }
}

pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60
    )
}
use std::time::Duration;
