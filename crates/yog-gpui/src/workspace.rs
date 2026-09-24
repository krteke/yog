mod components;
mod emulate;
pub(crate) mod messages;
mod predict;
mod source;
mod transcode;

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Root, Sizable as _, StyledExt as _, Theme, ThemeMode, TitleBar,
    button::{Button, ButtonVariants},
    menu::DropdownMenu,
    resizable::{h_resizable, resizable_panel},
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
    status_bar::StatusBar,
    tag::Tag,
};
use gpui_kit::{
    Anchor, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, KeyBinding, Menu, MenuItem, ParentElement as _, Render, Styled as _, Subscription,
    Window, actions, div, prelude::FluentBuilder as _, px,
};
use yog_runtime::RunStatus;

use self::{
    emulate::EmulatePage,
    predict::PredictPage,
    source::SourcePicker,
    transcode::{ActivityChanged, TranscodeActivity, TranscodePage},
};

actions!(
    yog,
    [
        OpenVideo,
        OpenDirectory,
        ShowTranscode,
        ShowPredict,
        ShowEmulate,
        ToggleSidebar,
        UseSystemTheme,
        UseLightTheme,
        UseDarkTheme
    ]
);

pub fn bind_keys(cx: &mut App) {
    #[cfg(target_os = "macos")]
    let shortcuts = ["cmd-o", "cmd-shift-o", "cmd-1", "cmd-2", "cmd-3", "cmd-b"];
    #[cfg(not(target_os = "macos"))]
    let shortcuts = [
        "ctrl-o",
        "ctrl-shift-o",
        "ctrl-1",
        "ctrl-2",
        "ctrl-3",
        "ctrl-b",
    ];

    cx.bind_keys([
        KeyBinding::new(shortcuts[0], OpenVideo, Some("YogWorkspace")),
        KeyBinding::new(shortcuts[1], OpenDirectory, Some("YogWorkspace")),
        KeyBinding::new(shortcuts[2], ShowTranscode, Some("YogWorkspace")),
        KeyBinding::new(shortcuts[3], ShowPredict, Some("YogWorkspace")),
        KeyBinding::new(shortcuts[4], ShowEmulate, Some("YogWorkspace")),
        KeyBinding::new(shortcuts[5], ToggleSidebar, Some("YogWorkspace")),
    ]);
    cx.set_menus([
        Menu::new("File").items([
            MenuItem::action("Open file…", OpenVideo),
            MenuItem::action("Open folder…", OpenDirectory),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle sidebar", ToggleSidebar),
            MenuItem::separator(),
            MenuItem::action("Transcode", ShowTranscode),
            MenuItem::action("Predict", ShowPredict),
            MenuItem::action("Emulate", ShowEmulate),
        ]),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Transcode,
    Predict,
    Emulate,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Appearance {
    System,
    Light,
    Dark,
}

impl Appearance {
    fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

pub struct Workspace {
    focus: FocusHandle,
    mode: Mode,
    appearance: Appearance,
    sidebar_collapsed: bool,
    source: Entity<SourcePicker>,
    transcode: Entity<TranscodePage>,
    predict: Entity<PredictPage>,
    emulate: Entity<EmulatePage>,
    _source_observation: Subscription,
    _activity_subscription: Subscription,
    _appearance_subscription: Subscription,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let source = cx.new(|_| SourcePicker::new());
        let transcode = cx.new(|cx| TranscodePage::new(source.clone(), window, cx));
        let predict = cx.new(|cx| PredictPage::new(source.clone(), window, cx));
        let emulate = cx.new(|cx| EmulatePage::new(source.clone(), window, cx));
        let source_observation = cx.observe(&source, |_, _, cx| cx.notify());
        let activity_subscription =
            cx.subscribe(&transcode, |_, _, _: &ActivityChanged, cx| cx.notify());

        let weak = cx.weak_entity();
        let appearance_subscription = window.observe_window_appearance(move |window, cx| {
            if weak
                .upgrade()
                .is_some_and(|workspace| workspace.read(cx).appearance == Appearance::System)
            {
                Theme::change(window.appearance(), Some(window), cx);
            }
        });

        Self {
            focus,
            mode: Mode::Transcode,
            appearance: Appearance::System,
            sidebar_collapsed: true,
            source,
            transcode,
            predict,
            emulate,
            _source_observation: source_observation,
            _activity_subscription: activity_subscription,
            _appearance_subscription: appearance_subscription,
        }
    }

    fn change_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.transcode.update(cx, |transcode, cx| {
            transcode.set_page_visible(mode == Mode::Transcode, window, cx);
        });
        self.mode = mode;
        self.source.update(cx, |source, cx| {
            source.set_allow_directory(mode != Mode::Emulate, cx);
        });
        cx.notify();
    }

    fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance = appearance;
        let theme = match appearance {
            Appearance::System => window.appearance().into(),
            Appearance::Light => ThemeMode::Light,
            Appearance::Dark => ThemeMode::Dark,
        };
        Theme::change(theme, Some(window), cx);
        cx.notify();
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Sidebar<SidebarMenu> {
        Sidebar::new("workflows").collapsible(true).child(
            SidebarMenu::new()
                .child(
                    SidebarMenuItem::new("Transcode")
                        .icon(IconName::Play)
                        .active(self.mode == Mode::Transcode)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.change_mode(Mode::Transcode, window, cx);
                        })),
                )
                .child(
                    SidebarMenuItem::new("Predict")
                        .icon(IconName::Cpu)
                        .active(self.mode == Mode::Predict)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.change_mode(Mode::Predict, window, cx);
                        })),
                )
                .child(
                    SidebarMenuItem::new("Emulate")
                        .icon(IconName::ChartPie)
                        .active(self.mode == Mode::Emulate)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.change_mode(Mode::Emulate, window, cx);
                        })),
                ),
        )
    }

    fn render_theme_menu(&self) -> impl IntoElement {
        let appearance = self.appearance;
        let focus = self.focus.clone();
        Button::new("appearance")
            .ghost()
            .small()
            .label(format!("Theme: {}", appearance.label()))
            .dropdown_caret(true)
            .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
                menu.action_context(focus.clone())
                    .menu_with_check(
                        "System",
                        appearance == Appearance::System,
                        Box::new(UseSystemTheme),
                    )
                    .menu_with_check(
                        "Light",
                        appearance == Appearance::Light,
                        Box::new(UseLightTheme),
                    )
                    .menu_with_check(
                        "Dark",
                        appearance == Appearance::Dark,
                        Box::new(UseDarkTheme),
                    )
            })
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> StatusBar {
        let source = self
            .source
            .read(cx)
            .path()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "No source selected".into());
        let bar = StatusBar::new().right(div().max_w_48().truncate().child(source));
        let Some(activity) = self.transcode.read(cx).activity() else {
            return bar;
        };
        let (tag, state, detail) = match activity {
            TranscodeActivity::Running(detail) => (Tag::info(), "Running", detail),
            TranscodeActivity::Cancelling => (Tag::warning(), "Cancelling", "Transcode".into()),
            TranscodeActivity::Finished(RunStatus::Success) => {
                (Tag::success(), "Completed", "Transcode".into())
            }
            TranscodeActivity::Finished(RunStatus::Failure) => {
                (Tag::danger(), "Failed", "Transcode".into())
            }
            TranscodeActivity::Finished(RunStatus::Cancelled) => {
                (Tag::secondary(), "Cancelled", "Transcode".into())
            }
        };
        let view_label = format!("View transcode: {state}, {detail}");
        let status = div()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .child(tag.small().outline().child(state))
            .child(div().min_w_0().truncate().child(detail))
            .when(self.mode != Mode::Transcode, |this| {
                this.child(
                    Button::new("view-transcode")
                        .ghost()
                        .small()
                        .label("View")
                        .accessibility_label(view_label)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.change_mode(Mode::Transcode, window, cx);
                        })),
                )
            });
        bar.left(status)
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status_bar = self.render_status_bar(cx);
        let page = match self.mode {
            Mode::Transcode => self.transcode.clone().into_any_element(),
            Mode::Predict => self.predict.clone().into_any_element(),
            Mode::Emulate => self.emulate.clone().into_any_element(),
        };
        let content = if self.sidebar_collapsed {
            div()
                .flex()
                .items_stretch()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(self.render_sidebar(cx).collapsed(true))
                .child(div().flex_1().min_w_0().min_h_0().child(page))
                .into_any_element()
        } else {
            h_resizable("main-panes")
                .child(
                    resizable_panel()
                        .size(px(224.))
                        .size_range(px(160.)..px(240.))
                        .flex_none()
                        .child(self.render_sidebar(cx).w_full()),
                )
                .child(resizable_panel().child(page))
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .key_context("YogWorkspace")
            .on_action(cx.listener(|this, _: &OpenVideo, _, cx| {
                this.source
                    .update(cx, |source, cx| source.choose(false, cx));
            }))
            .on_action(cx.listener(|this, _: &OpenDirectory, _, cx| {
                this.source.update(cx, |source, cx| source.choose(true, cx));
            }))
            .on_action(cx.listener(|this, _: &ShowTranscode, window, cx| {
                this.change_mode(Mode::Transcode, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowPredict, window, cx| {
                this.change_mode(Mode::Predict, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowEmulate, window, cx| {
                this.change_mode(Mode::Emulate, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_collapsed = !this.sidebar_collapsed;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &UseSystemTheme, window, cx| {
                this.set_appearance(Appearance::System, window, cx);
            }))
            .on_action(cx.listener(|this, _: &UseLightTheme, window, cx| {
                this.set_appearance(Appearance::Light, window, cx);
            }))
            .on_action(cx.listener(|this, _: &UseDarkTheme, window, cx| {
                this.set_appearance(Appearance::Dark, window, cx);
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("toggle-sidebar")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(if self.sidebar_collapsed {
                                        IconName::PanelLeftOpen
                                    } else {
                                        IconName::PanelLeftClose
                                    }))
                                    .tooltip(if self.sidebar_collapsed {
                                        "Expand sidebar"
                                    } else {
                                        "Collapse sidebar"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.sidebar_collapsed = !this.sidebar_collapsed;
                                        cx.notify();
                                    })),
                            )
                            .child(div().font_medium().child("Yog")),
                    )
                    .child(self.render_theme_menu()),
            )
            .child(content)
            .child(status_bar)
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
