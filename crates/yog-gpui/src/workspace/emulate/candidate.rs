use super::*;
use gpui_kit::{FocusHandle, Focusable, InteractiveElement as _};

pub struct CandidateEditor {
    focus: FocusHandle,
    video: Entity<VideoInputs>,
    qualities: Entity<InputState>,
    _video_observation: Subscription,
}

impl CandidateEditor {
    pub fn new(qualities: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let video = cx.new(|cx| VideoInputs::new(window, cx));
        let video_observation = cx.observe(&video, |_, _, cx| cx.notify());
        Self {
            focus: cx.focus_handle(),
            video,
            qualities: cx.new(|cx| InputState::new(window, cx).default_value(qualities)),
            _video_observation: video_observation,
        }
    }

    pub fn build(&self, cx: &App) -> Result<Candidate, String> {
        let (decoding, encoding) = self.video.read(cx).build(cx);
        let qualities = parse_quality_points(&self.qualities.read(cx).value())?;
        Ok(Candidate {
            decoding,
            encoding,
            qualities,
        })
    }
}

impl Focusable for CandidateEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CandidateEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap_3()
            .child(self.video.clone())
            .child(
                Form::new().child(
                    Field::new()
                        .label(format!(
                            "Quality points ({})",
                            self.video.read(cx).quality_parameter(cx)
                        ))
                        .child(Input::new(&self.qualities)),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Mode, Workspace};
    use gpui_kit::component::Root;
    use gpui_kit::{TestAppContext, size, test::TestWindowExt as _};

    #[gpui_kit::test]
    fn focused_candidate_keeps_keyboard_editing_when_scrolled_offscreen(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            let page = workspace.read(cx).emulate.clone();
            page.update(cx, |page, cx| {
                for _ in 0..20 {
                    page.add_candidate(window, cx);
                }
                page.candidate_list.scroll_to_end();
            });
            workspace.update(cx, |workspace, cx| {
                workspace.change_mode(Mode::Emulate, window, cx)
            });
            window.render_frame(cx);
            let qualities = page.read(cx).candidates[19].read(cx).qualities.clone();
            let input_id = ("input", qualities.entity_id());
            assert!(window.find(input_id).visible());
            window.focus(&qualities.focus_handle(cx), cx);
            window.render_frame(cx);
            window.scroll(
                "emulate-candidates",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(1_000_000.))),
                cx,
            );
            assert!(
                window.try_find(input_id).is_some(),
                "focused editor disappeared from the keyboard dispatch tree"
            );
            window.press("secondary-a", cx);
            window.input("17,31", cx);
            assert_eq!(qualities.read(cx).value().as_ref(), "17,31");
        })
        .unwrap();
    }
}
