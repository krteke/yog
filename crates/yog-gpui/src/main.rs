mod logging;
mod workspace;

use gpui_kit::component::{Root, Theme, TitleBar};
use gpui_kit::*;
use workspace::Workspace;

fn main() {
    logging::init();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            Theme::change(cx.window_appearance(), None, cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(800.)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Yog".into()),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            };

            workspace::bind_keys(cx);

            cx.spawn(async move |cx| {
                cx.open_window(options, |window, cx| {
                    let workspace = cx.new(|cx| Workspace::new(window, cx));
                    cx.new(|cx| Root::new(workspace, window, cx))
                })
                .expect("failed to open Yog window");
            })
            .detach();
        });
}
