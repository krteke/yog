use super::*;

pub struct CandidateEditor {
    video: Entity<VideoInputs>,
    qualities: Entity<InputState>,
    _video_observation: Subscription,
}

impl CandidateEditor {
    pub fn new(qualities: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let video = cx.new(|cx| VideoInputs::new(window, cx));
        let video_observation = cx.observe(&video, |_, _, cx| cx.notify());
        Self {
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

impl Render for CandidateEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
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
