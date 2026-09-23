use std::path::{Path, PathBuf};

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, StyledExt as _,
    button::{Button, ButtonVariants},
};
use gpui_kit::{
    Context, ExternalPaths, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, Styled as _, Window, div, prelude::FluentBuilder as _,
};

pub struct SourcePicker {
    path: Option<PathBuf>,
    is_directory: bool,
    allow_directory: bool,
    error: Option<String>,
}

impl SourcePicker {
    pub fn new() -> Self {
        Self {
            path: None,
            is_directory: false,
            allow_directory: true,
            error: None,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn valid_path(&self) -> Option<&Path> {
        (!self.is_directory || self.allow_directory)
            .then(|| self.path())
            .flatten()
    }

    pub fn is_directory(&self) -> bool {
        self.is_directory
    }

    pub fn set_allow_directory(&mut self, allow: bool, cx: &mut Context<Self>) {
        self.allow_directory = allow;
        self.error = None;
        cx.notify();
    }

    fn set_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_file() && !path.is_dir() {
            self.error = Some(format!("Cannot open {}", path.display()));
        } else if path.is_dir() && !self.allow_directory {
            self.error = Some("This workflow requires a video file".into());
        } else {
            self.is_directory = path.is_dir();
            self.path = Some(path);
            self.error = None;
        }
        cx.notify();
    }

    pub fn choose(&mut self, directory: bool, cx: &mut Context<Self>) {
        if directory && !self.allow_directory {
            self.error = Some("This workflow requires a video file".into());
            cx.notify();
            return;
        }

        let response = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: Some(
                if directory {
                    "Open folder"
                } else {
                    "Open video"
                }
                .into(),
            ),
        });

        cx.spawn(async move |this, cx| match response.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    _ = this.update(cx, |this, cx| this.set_path(path, cx));
                }
            }
            Ok(Ok(None)) => {}
            result => {
                _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("Could not open file picker: {result:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

impl Render for SourcePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self.path.clone();
        let invalid_for_mode = self.is_directory && !self.allow_directory;

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(div().text_lg().font_medium().child("Source"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("open-video")
                                    .label("Open file…")
                                    .on_click(cx.listener(|this, _, _, cx| this.choose(false, cx))),
                            )
                            .when(self.allow_directory, |this| {
                                this.child(
                                    Button::new("open-folder").label("Open folder…").on_click(
                                        cx.listener(|this, _, _, cx| this.choose(true, cx)),
                                    ),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .id("source-drop-zone")
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .min_h_56()
                    .w_full()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                        if let [path] = paths.paths() {
                            this.set_path(path.clone(), cx);
                        } else {
                            this.error = Some("Drop one file or folder at a time".into());
                            cx.notify();
                        }
                    }))
                    .child(
                        Icon::new(if self.is_directory {
                            IconName::FolderOpen
                        } else {
                            IconName::File
                        })
                        .size_8(),
                    )
                    .child(
                        div().text_lg().font_medium().max_w_full().truncate().child(
                            path.as_ref()
                                .and_then(|path| path.file_name())
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_else(|| "Drop a video here".into()),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(if invalid_for_mode {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            })
                            .max_w_full()
                            .truncate()
                            .child(if invalid_for_mode {
                                "Select a video file for this workflow".into()
                            } else {
                                path.as_ref()
                                    .map(|path| path.display().to_string())
                                    .unwrap_or_default()
                            }),
                    )
                    .when_some(self.error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    }),
            )
            .when_some(path, |this, path| {
                this.child(
                    Button::new("reveal-source")
                        .ghost()
                        .label("Reveal in file manager")
                        .on_click(move |_, _, cx| cx.reveal_path(&path)),
                )
            })
    }
}
