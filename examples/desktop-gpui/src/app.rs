use crate::{theme, ui::dashboard::Dashboard};
use gpui::*;
use gpui_component::{Root, TitleBar};

pub fn run() {
    let app = Application::new().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        theme::init(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1024.0), px(768.0)),
                cx,
            ))),
            titlebar: Some(TitleBar::title_bar_options()),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| Dashboard::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open desktop window");
        })
        .detach();
    });
}
