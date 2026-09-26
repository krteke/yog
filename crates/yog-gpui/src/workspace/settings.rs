use std::{
    fs,
    io::{self, Write as _},
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    path::{Path, PathBuf},
    time::Duration,
};

use gpui_kit::component::{
    ActiveTheme, StyledExt as _,
    button::{Button, ButtonVariants},
    form::{Field, Form},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div,
};
use serde::{Deserialize, Serialize};
use yog_runtime::{Config, Options};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct SavedSettings {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    timeout_seconds: Option<NonZeroU64>,
    progress_tick_interval_ms: NonZeroU64,
    prediction_samples: NonZeroUsize,
    prediction_sample_seconds: f64,
    chart_width: NonZeroU32,
    chart_height: NonZeroU32,
}

impl Default for SavedSettings {
    fn default() -> Self {
        Self::from_config(Config::default())
    }
}

impl SavedSettings {
    fn from_config(config: Config) -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            timeout_seconds: None,
            progress_tick_interval_ms: config.progress_tick_interval_ms,
            prediction_samples: config.prediction.samples,
            prediction_sample_seconds: config.prediction.sample_sec.as_secs_f64(),
            chart_width: config.emulation.width,
            chart_height: config.emulation.height,
        }
    }

    fn validate(&self) -> Result<(), (&'static str, String)> {
        if self.ffmpeg.as_os_str().is_empty() {
            return Err(("ffmpeg", "FFmpeg path cannot be empty".into()));
        }
        if self.ffprobe.as_os_str().is_empty() {
            return Err(("ffprobe", "FFprobe path cannot be empty".into()));
        }
        if self.prediction_sample_seconds <= 0.0 {
            return Err((
                "sample_seconds",
                "Sample duration must be greater than zero".into(),
            ));
        }
        Duration::try_from_secs_f64(self.prediction_sample_seconds).map_err(|_| {
            (
                "sample_seconds",
                "Sample duration must be a finite number greater than zero".to_owned(),
            )
        })?;
        Ok(())
    }

    fn load(path: &Path) -> Result<Self, String> {
        let settings = match fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents)
                .map_err(|error| format!("Invalid GUI settings at {}: {error}", path.display()))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let config = Config::load(None).map_err(|error| format!("{error:#}"))?;
                Self::from_config(config)
            }
            Err(error) => {
                return Err(format!(
                    "Could not read GUI settings at {}: {error}",
                    path.display()
                ));
            }
        };
        settings.validate().map_err(|(_, message)| message)?;
        Ok(settings)
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        let contents = toml::to_string_pretty(self)
            .map_err(|error| format!("Could not format GUI settings: {error}"))?;
        let directory = path.parent().expect("GUI settings path has a directory");
        fs::create_dir_all(directory)
            .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory)
            .map_err(|error| format!("Could not create temporary settings: {error}"))?;
        temporary
            .write_all(contents.as_bytes())
            .map_err(|error| format!("Could not write GUI settings: {error}"))?;
        temporary
            .persist(path)
            .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
        Ok(())
    }

    fn runtime(&self) -> (Options, Config) {
        let options = Options {
            ffmpeg: self.ffmpeg.clone(),
            ffprobe: self.ffprobe.clone(),
            timeout: self
                .timeout_seconds
                .map(|seconds| Duration::from_secs(seconds.get())),
            terminal_output: false,
            ..Options::default()
        };
        let mut config = Config {
            progress_tick_interval_ms: self.progress_tick_interval_ms,
            ..Config::default()
        };
        config.prediction.samples = self.prediction_samples;
        config.prediction.sample_sec = Duration::from_secs_f64(self.prediction_sample_seconds);
        config.emulation.width = self.chart_width;
        config.emulation.height = self.chart_height;
        (options, config)
    }
}

pub struct SettingsPanel {
    path: Option<PathBuf>,
    current: SavedSettings,
    load_error: Option<String>,
    status: Option<(bool, String)>,
    validation_error: Option<(&'static str, String)>,
    ffmpeg: Entity<InputState>,
    ffprobe: Entity<InputState>,
    timeout: Entity<InputState>,
    progress_tick: Entity<InputState>,
    samples: Entity<InputState>,
    sample_seconds: Entity<InputState>,
    chart_width: Entity<InputState>,
    chart_height: Entity<InputState>,
    _input_subscriptions: Vec<Subscription>,
}

impl SettingsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let path = dirs::config_dir().map(|directory| directory.join("yog").join("gui.toml"));
        let loaded = path.as_deref().map(SavedSettings::load).unwrap_or_else(|| {
            Config::load(None)
                .map(SavedSettings::from_config)
                .map_err(|error| format!("{error:#}"))
        });
        let (current, load_error) = match loaded {
            Ok(settings) => (settings, None),
            Err(error) => (SavedSettings::default(), Some(error)),
        };

        let ffmpeg = cx.new(|cx| {
            InputState::new(window, cx).default_value(current.ffmpeg.to_string_lossy().to_string())
        });
        let ffprobe = cx.new(|cx| {
            InputState::new(window, cx).default_value(current.ffprobe.to_string_lossy().to_string())
        });
        let timeout = cx.new(|cx| {
            InputState::new(window, cx).default_value(
                current
                    .timeout_seconds
                    .map_or(String::new(), |seconds| seconds.get().to_string()),
            )
        });
        let progress_tick = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current.progress_tick_interval_ms.get().to_string())
        });
        let samples = cx.new(|cx| {
            InputState::new(window, cx).default_value(current.prediction_samples.get().to_string())
        });
        let sample_seconds = cx.new(|cx| {
            InputState::new(window, cx).default_value(current.prediction_sample_seconds.to_string())
        });
        let chart_width = cx.new(|cx| {
            InputState::new(window, cx).default_value(current.chart_width.get().to_string())
        });
        let chart_height = cx.new(|cx| {
            InputState::new(window, cx).default_value(current.chart_height.get().to_string())
        });
        let input_subscriptions = [
            &ffmpeg,
            &ffprobe,
            &timeout,
            &progress_tick,
            &samples,
            &sample_seconds,
            &chart_width,
            &chart_height,
        ]
        .into_iter()
        .map(|input| {
            cx.subscribe_in(input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.status = None;
                    this.validation_error = None;
                    cx.notify();
                }
            })
        })
        .collect();

        Self {
            path,
            current,
            load_error,
            status: None,
            validation_error: None,
            ffmpeg,
            ffprobe,
            timeout,
            progress_tick,
            samples,
            sample_seconds,
            chart_width,
            chart_height,
            _input_subscriptions: input_subscriptions,
        }
    }

    pub fn run_defaults(&self) -> Result<(Options, Config), String> {
        if let Some(error) = &self.load_error {
            return Err(format!("Settings could not be loaded: {error}"));
        }
        Ok(self.current.runtime())
    }

    pub fn discard_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values = [
            (
                &self.ffmpeg,
                self.current.ffmpeg.to_string_lossy().into_owned(),
            ),
            (
                &self.ffprobe,
                self.current.ffprobe.to_string_lossy().into_owned(),
            ),
            (
                &self.timeout,
                self.current
                    .timeout_seconds
                    .map_or(String::new(), |seconds| seconds.get().to_string()),
            ),
            (
                &self.progress_tick,
                self.current.progress_tick_interval_ms.get().to_string(),
            ),
            (
                &self.samples,
                self.current.prediction_samples.get().to_string(),
            ),
            (
                &self.sample_seconds,
                self.current.prediction_sample_seconds.to_string(),
            ),
            (
                &self.chart_width,
                self.current.chart_width.get().to_string(),
            ),
            (
                &self.chart_height,
                self.current.chart_height.get().to_string(),
            ),
        ];
        for (field, value) in values {
            field.update(cx, |field, cx| field.set_value(value, window, cx));
        }
        self.status = None;
        self.validation_error = None;
        cx.notify();
    }

    fn parse(&self, cx: &gpui_kit::App) -> Result<SavedSettings, (&'static str, String)> {
        fn positive<T: std::str::FromStr>(
            value: &str,
            key: &'static str,
            label: &str,
        ) -> Result<T, (&'static str, String)> {
            value
                .trim()
                .parse()
                .map_err(|_| (key, format!("{label} must be an integer greater than zero")))
        }

        let timeout = self.timeout.read(cx).value();
        let sample_seconds = self.sample_seconds.read(cx).value();
        let settings = SavedSettings {
            ffmpeg: self.ffmpeg.read(cx).value().trim().into(),
            ffprobe: self.ffprobe.read(cx).value().trim().into(),
            timeout_seconds: (!timeout.trim().is_empty())
                .then(|| positive(&timeout, "timeout", "Timeout"))
                .transpose()?,
            progress_tick_interval_ms: positive(
                &self.progress_tick.read(cx).value(),
                "progress_tick",
                "Progress interval",
            )?,
            prediction_samples: positive(
                &self.samples.read(cx).value(),
                "samples",
                "Sample count",
            )?,
            prediction_sample_seconds: sample_seconds.trim().parse().map_err(|_| {
                (
                    "sample_seconds",
                    "Sample duration must be a positive number".to_owned(),
                )
            })?,
            chart_width: positive(
                &self.chart_width.read(cx).value(),
                "chart_width",
                "Chart width",
            )?,
            chart_height: positive(
                &self.chart_height.read(cx).value(),
                "chart_height",
                "Chart height",
            )?,
        };
        settings.validate()?;
        Ok(settings)
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let settings = match self.parse(cx) {
            Ok(settings) => settings,
            Err(error) => {
                self.status = Some((false, error.1.clone()));
                self.validation_error = Some(error);
                cx.notify();
                return;
            }
        };
        self.validation_error = None;
        let result = (|| {
            let path = self
                .path
                .as_deref()
                .ok_or_else(|| "Could not locate the user configuration directory".to_owned())?;
            settings.save(path)?;
            Ok(settings)
        })();
        match result {
            Ok(settings) => {
                self.current = settings;
                self.load_error = None;
                self.status = Some((true, "Saved. New runs will use these settings.".into()));
            }
            Err(error) => self.status = Some((false, error)),
        }
        cx.notify();
    }

    fn input_field(
        &self,
        key: &'static str,
        label: &'static str,
        input: &Entity<InputState>,
        hint: Option<&'static str>,
    ) -> Field {
        let field = Field::new().label(label).child(Input::new(input).id(key));
        if let Some((_, error)) = self
            .validation_error
            .as_ref()
            .filter(|(field, _)| *field == key)
        {
            let error = error.clone();
            field.description_fn(move |_, cx| {
                div().text_color(cx.theme().danger).child(error.clone())
            })
        } else if let Some(hint) = hint {
            field.description(hint)
        } else {
            field
        }
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .w_full()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .pb_6()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(div().text_lg().font_medium().child("Execution"))
                            .child(
                                Form::new()
                                    .child(self.input_field(
                                        "ffmpeg",
                                        "FFmpeg",
                                        &self.ffmpeg,
                                        Some("Executable name or path"),
                                    ))
                                    .child(self.input_field(
                                        "ffprobe",
                                        "FFprobe",
                                        &self.ffprobe,
                                        Some("Executable name or path"),
                                    ))
                                    .child(self.input_field(
                                        "timeout",
                                        "Timeout (seconds)",
                                        &self.timeout,
                                        Some("Leave empty for no time limit"),
                                    ))
                                    .child(self.input_field(
                                        "progress_tick",
                                        "Progress interval (ms)",
                                        &self.progress_tick,
                                        None,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .pb_6()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(div().text_lg().font_medium().child("Prediction"))
                            .child(
                                Form::new()
                                    .child(self.input_field(
                                        "samples",
                                        "Samples",
                                        &self.samples,
                                        None,
                                    ))
                                    .child(self.input_field(
                                        "sample_seconds",
                                        "Sample duration (seconds)",
                                        &self.sample_seconds,
                                        None,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(div().text_lg().font_medium().child("Emulation chart"))
                            .child(
                                Form::new()
                                    .child(self.input_field(
                                        "chart_width",
                                        "Width (px)",
                                        &self.chart_width,
                                        None,
                                    ))
                                    .child(self.input_field(
                                        "chart_height",
                                        "Height (px)",
                                        &self.chart_height,
                                        None,
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .px_6()
                    .py_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .min_w_0()
                            .text_sm()
                            .text_color(
                                if self.load_error.is_some()
                                    || self.status.as_ref().is_some_and(|(saved, _)| !saved)
                                {
                                    theme.danger
                                } else {
                                    theme.muted_foreground
                                },
                            )
                            .child(
                                self.status
                                    .as_ref()
                                    .map(|(_, message)| message.clone())
                                    .or_else(|| self.load_error.clone())
                                    .unwrap_or_else(|| {
                                        "Changes affect new runs after saving".into()
                                    }),
                            ),
                    )
                    .child(
                        Button::new("save-settings")
                            .primary()
                            .label("Save")
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;
    use gpui_kit::{
        ScrollDelta, TestAppContext,
        component::{Root, WindowExt as _},
        point, px, size,
        test::TestWindowExt as _,
    };

    #[test]
    fn saved_settings_are_loaded_and_used_by_new_runs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gui.toml");
        let settings = SavedSettings {
            ffmpeg: "/opt/tools/ffmpeg".into(),
            timeout_seconds: NonZeroU64::new(12),
            prediction_samples: NonZeroUsize::new(8).unwrap(),
            chart_width: NonZeroU32::new(1920).unwrap(),
            ..SavedSettings::default()
        };
        settings.save(&path).unwrap();

        let loaded = SavedSettings::load(&path).unwrap();
        let (options, config) = loaded.runtime();
        assert_eq!(options.ffmpeg, PathBuf::from("/opt/tools/ffmpeg"));
        assert_eq!(options.timeout, Some(Duration::from_secs(12)));
        assert!(!options.terminal_output);
        assert_eq!(config.prediction.samples.get(), 8);
        assert_eq!(config.emulation.width.get(), 1920);

        let replacement = SavedSettings {
            ffprobe: "/opt/tools/ffprobe".into(),
            ..loaded
        };
        replacement.save(&path).unwrap();
        assert_eq!(
            SavedSettings::load(&path).unwrap().ffprobe,
            PathBuf::from("/opt/tools/ffprobe")
        );
    }

    #[test]
    fn invalid_persisted_sample_duration_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gui.toml");
        let settings = SavedSettings {
            prediction_sample_seconds: 0.0,
            ..SavedSettings::default()
        };
        settings.save(&path).unwrap();

        assert!(SavedSettings::load(&path).is_err());
    }

    #[gpui_kit::test]
    fn saving_from_the_page_updates_future_run_defaults(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gui.toml");
        let mut page = None;
        let handle = cx.open_window(size(px(900.), px(1200.)), |window, cx| {
            let view = cx.new(|cx| SettingsPanel::new(window, cx));
            page = Some(view.clone());
            Root::new(view, window, cx)
        });
        let page = page.unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            page.update(cx, |page, cx| {
                page.path = Some(path.clone());
                page.load_error = None;
                cx.notify();
            });
            window.render_frame(cx);
            window.click("ffmpeg", cx);
            window.press("secondary-a", cx);
            window.input("/opt/custom/ffmpeg", cx);
            window.click("save-settings", cx);
            assert_eq!(
                page.read(cx).run_defaults().unwrap().0.ffmpeg,
                PathBuf::from("/opt/custom/ffmpeg")
            );
            let saved = fs::read_to_string(&path).unwrap();
            window.click("samples", cx);
            window.press("secondary-a", cx);
            window.input("0", cx);
            window.click("save-settings", cx);
            assert_eq!(
                page.read(cx).validation_error.as_ref().unwrap().0,
                "samples"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), saved);
        })
        .unwrap();
        assert_eq!(
            SavedSettings::load(&path).unwrap().ffmpeg,
            PathBuf::from("/opt/custom/ffmpeg")
        );
    }

    #[gpui_kit::test]
    fn settings_button_opens_sheet_and_closing_discards_unsaved_edits(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gui.toml");
        let mut workspace = None;
        let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            let settings = workspace.read(cx).settings.clone();
            settings.update(cx, |settings, cx| {
                settings.path = Some(path.clone());
                settings.load_error = None;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(!window.has_active_sheet(cx));
            window.click("open-settings", cx);
            assert!(window.has_active_sheet(cx));
            assert!(window.find("sheet-content").visible());

            let original = window.find("ffmpeg").value().unwrap().to_owned();
            window.click("ffmpeg", cx);
            window.press("secondary-a", cx);
            window.input("/temporary/ffmpeg", cx);
            assert_eq!(window.find("ffmpeg").value(), Some("/temporary/ffmpeg"));

            window.press("escape", cx);
            assert!(!window.has_active_sheet(cx));
            window.click("open-settings", cx);
            assert_eq!(window.find("ffmpeg").value(), Some(original.as_str()));

            window.click("ffmpeg", cx);
            window.press("secondary-a", cx);
            window.input("/saved/ffmpeg", cx);
            window.scroll(
                "sheet-content",
                ScrollDelta::Pixels(point(px(0.), px(-1000.))),
                cx,
            );
            assert!(window.find("save-settings").visible());
            window.click("save-settings", cx);
            assert_eq!(
                workspace
                    .read(cx)
                    .settings
                    .read(cx)
                    .run_defaults()
                    .unwrap()
                    .0
                    .ffmpeg,
                PathBuf::from("/saved/ffmpeg")
            );
        })
        .unwrap();
        assert_eq!(
            SavedSettings::load(&path).unwrap().ffmpeg,
            PathBuf::from("/saved/ffmpeg")
        );
    }
}
