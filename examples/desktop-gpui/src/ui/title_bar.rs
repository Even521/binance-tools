use crate::theme;
use crate::ui::ai::chat::AiChatPanel;
use crate::ui::palette;
use gpui::{actions, prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt, TitleBar,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
};

actions!(
    desktop_gpui,
    [
        OpenDailyMaSignals,
        OpenKlineCandlestick,
        OpenMarketProducts,
        OpenSpotSymbols,
        OpenSquareKeySettings,
        OpenSquareSendLogs,
        OpenSquareTasks
    ]
);

pub struct DesktopTitleBar {
    ai_chat_panel: Entity<AiChatPanel>,
}

impl DesktopTitleBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            ai_chat_panel: cx.new(|cx| AiChatPanel::new(window, cx)),
        }
    }

    pub fn ai_chat_panel(&self) -> &Entity<AiChatPanel> {
        &self.ai_chat_panel
    }
}

impl Render for DesktopTitleBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme();
        let ai_active = self.ai_chat_panel.read(cx).is_visible();

        TitleBar::new()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_1()
                    .text_size(px(11.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new("theme-menu")
                            .label("主题")
                            .ghost()
                            .xsmall()
                            .dropdown_menu(|mut menu, _, cx| {
                                let current_theme = cx.theme().theme_name().clone();

                                for theme_name in theme::theme_names(cx) {
                                    let checked = theme_name == current_theme;
                                    let item_name = theme_name.clone();
                                    menu = menu.item(
                                        PopupMenuItem::element(move |_, _| {
                                            div().text_size(px(11.)).child(item_name.clone())
                                        })
                                        .checked(checked)
                                        .on_click({
                                            let theme_name = theme_name.clone();
                                            move |_, window, cx| {
                                                theme::apply_and_save_theme(
                                                    &theme_name,
                                                    Some(window),
                                                    cx,
                                                );
                                            }
                                        }),
                                    );
                                }

                                menu.min_w(px(200.)).max_h(px(320.)).scrollable(true)
                            }),
                    )
                    .child(
                        Button::new("spot-page")
                            .label("现货")
                            .ghost()
                            .xsmall()
                            .dropdown_menu(|menu, _, _| {
                                menu.item(
                                    PopupMenuItem::element(|_, _| {
                                        div().text_size(px(11.)).child("市场榜单")
                                    })
                                    .on_click(
                                        |_, window, cx| {
                                            window
                                                .dispatch_action(Box::new(OpenMarketProducts), cx);
                                        },
                                    ),
                                )
                                .item(
                                    PopupMenuItem::element(|_, _| {
                                        div().text_size(px(11.)).child("币种")
                                    })
                                    .on_click(
                                        |_, window, cx| {
                                            window.dispatch_action(Box::new(OpenSpotSymbols), cx);
                                        },
                                    ),
                                )
                                .item(
                                    PopupMenuItem::element(|_, _| {
                                        div().text_size(px(11.)).child("日均线信号")
                                    })
                                    .on_click(
                                        |_, window, cx| {
                                            window
                                                .dispatch_action(Box::new(OpenDailyMaSignals), cx);
                                        },
                                    ),
                                )
                                .min_w(px(110.))
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new("square-menu")
                            .label("币安广场")
                            .ghost()
                            .xsmall()
                            .dropdown_menu(|menu, _, _| {
                                menu.item(
                                    PopupMenuItem::element(|_, _| {
                                        div().text_size(px(11.)).child("Key设置")
                                    })
                                    .on_click(
                                        |_, window, cx| {
                                            window.dispatch_action(
                                                Box::new(OpenSquareKeySettings),
                                                cx,
                                            );
                                        },
                                    ),
                                )
                                .item(
                                    PopupMenuItem::element(|_, _| {
                                        div().text_size(px(11.)).child("任务页面")
                                    })
                                    .on_click(
                                        |_, window, cx| {
                                            window.dispatch_action(Box::new(OpenSquareTasks), cx);
                                        },
                                    ),
                                )
                                .item(
                                    PopupMenuItem::element(|_, _| {
                                        div().text_size(px(11.)).child("发送消息日志")
                                    })
                                    .on_click(
                                        |_, window, cx| {
                                            window
                                                .dispatch_action(Box::new(OpenSquareSendLogs), cx);
                                        },
                                    ),
                                )
                                .min_w(px(130.))
                            }),
                    )
                    .child(
                        Button::new("help-menu")
                            .label("帮助")
                            .ghost()
                            .xsmall()
                            .dropdown_menu(|menu, _, _| {
                                menu.item(
                                    PopupMenuItem::element(|_, _| {
                                        div().text_size(px(11.)).child("推特")
                                    })
                                    .on_click(|_, _, cx| {
                                        cx.open_url("https://x.com/even366");
                                    }),
                                )
                                .item(
                                    PopupMenuItem::element(|_, _| {
                                        div().text_size(px(11.)).child("XChat")
                                    })
                                    .on_click(|_, _, cx| {
                                        cx.open_url("https://x.com/i/chat/group_join/g2048002289320034517/qKHtG5MR82");
                                    }),
                                )
                                .item(
                                    PopupMenuItem::element(|_, _| {
                                        div().text_size(px(11.)).child("邮箱")
                                    })
                                    .on_click(|_, _, cx| {
                                        cx.open_url("mailto:even366@qq.com");
                                    }),
                                )
                                .min_w(px(120.))
                            }),
                    ),
            )
            // ── AI badge (right side of title bar) ──
            .child(
                div().flex().items_center().ml_auto().mr_1().child(
                    Button::new("ai-badge")
                        .ghost()
                        .xsmall()
                        .when(ai_active, |button| button.bg(app_theme.muted.opacity(0.18)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ai_chat_panel.update(cx, |panel, cx| panel.toggle(cx));
                            cx.notify();
                        }))
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(if ai_active {
                                            app_theme.primary.opacity(0.9)
                                        } else {
                                            palette::muted(app_theme)
                                        })
                                        .child("✦"),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .font_semibold()
                                        .text_color(if ai_active {
                                            palette::text_strong(app_theme)
                                        } else {
                                            palette::muted(app_theme)
                                        })
                                        .child("AI"),
                                ),
                        ),
                ),
            )
    }
}
