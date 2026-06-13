use crate::ui::palette;
use gpui::*;
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    text::TextView,
    v_flex,
};

struct StrategyDoc {
    title: &'static str,
    file: &'static str,
    content: &'static str,
}

const STRATEGY_DOCS: &[StrategyDoc] = &[
    StrategyDoc {
        title: "MA Cross",
        file: "01-ma-cross.md",
        content: include_str!("../../../../docs/策略知识/回测策略/01-ma-cross.md"),
    },
    StrategyDoc {
        title: "Grid",
        file: "02-grid.md",
        content: include_str!("../../../../docs/策略知识/回测策略/02-grid.md"),
    },
    StrategyDoc {
        title: "Trend Grid",
        file: "03-trend-grid.md",
        content: include_str!("../../../../docs/策略知识/回测策略/03-trend-grid.md"),
    },
    StrategyDoc {
        title: "Turtle",
        file: "04-turtle.md",
        content: include_str!("../../../../docs/策略知识/回测策略/04-turtle.md"),
    },
    StrategyDoc {
        title: "Martingale",
        file: "05-martingale.md",
        content: include_str!("../../../../docs/策略知识/回测策略/05-martingale.md"),
    },
    StrategyDoc {
        title: "RSI",
        file: "06-rsi.md",
        content: include_str!("../../../../docs/策略知识/回测策略/06-rsi.md"),
    },
    StrategyDoc {
        title: "MACD",
        file: "07-macd.md",
        content: include_str!("../../../../docs/策略知识/回测策略/07-macd.md"),
    },
    StrategyDoc {
        title: "Bollinger Bands",
        file: "08-bollinger-bands.md",
        content: include_str!("../../../../docs/策略知识/回测策略/08-bollinger-bands.md"),
    },
    StrategyDoc {
        title: "Volume Spike",
        file: "09-volume-spike.md",
        content: include_str!("../../../../docs/策略知识/回测策略/09-volume-spike.md"),
    },
    StrategyDoc {
        title: "OBV",
        file: "10-obv.md",
        content: include_str!("../../../../docs/策略知识/回测策略/10-obv.md"),
    },
    StrategyDoc {
        title: "Stochastic",
        file: "11-stochastic.md",
        content: include_str!("../../../../docs/策略知识/回测策略/11-stochastic.md"),
    },
    StrategyDoc {
        title: "CCI",
        file: "12-cci.md",
        content: include_str!("../../../../docs/策略知识/回测策略/12-cci.md"),
    },
    StrategyDoc {
        title: "SuperTrend",
        file: "13-supertrend.md",
        content: include_str!("../../../../docs/策略知识/回测策略/13-supertrend.md"),
    },
];

pub struct StrategyHelpPage {
    selected: usize,
}

impl StrategyHelpPage {
    pub fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Self { selected: 0 }
    }

    fn selected_doc(&self) -> &'static StrategyDoc {
        &STRATEGY_DOCS[self.selected.min(STRATEGY_DOCS.len().saturating_sub(1))]
    }

    fn select_strategy(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index.min(STRATEGY_DOCS.len().saturating_sub(1));
        cx.notify();
    }

    fn render_strategy_item(
        &self,
        index: usize,
        doc: &'static StrategyDoc,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();
        let selected = self.selected == index;

        Button::new(("strategy-help-item", index))
            .ghost()
            .small()
            .w_full()
            .bg(if selected {
                app_theme.primary.opacity(0.12)
            } else {
                gpui::transparent_black()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_strategy(index, cx);
            }))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(if selected {
                                app_theme.primary
                            } else {
                                palette::text_strong(app_theme)
                            })
                            .child(doc.title),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(palette::muted(app_theme))
                            .child(doc.file),
                    ),
            )
            .into_any_element()
    }
}

impl Render for StrategyHelpPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let strategy_items = STRATEGY_DOCS
            .iter()
            .enumerate()
            .map(|(index, doc)| self.render_strategy_item(index, doc, cx))
            .collect::<Vec<_>>();
        let doc = self.selected_doc();
        let app_theme = cx.theme();

        v_flex()
            .size_full()
            .gap_3()
            .child(
                div()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(app_theme.background)
                    .px_4()
                    .py_3()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(18.))
                                            .font_semibold()
                                            .child("策略说明"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette::muted(app_theme))
                                            .child("选择左侧策略，查看创始人、计算公式、适合行情和历史故事。"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(app_theme))
                                    .child(format!("{} 个策略", STRATEGY_DOCS.len())),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .gap_3()
                    .overflow_hidden()
                    .child(
                        div()
                            .w(px(260.))
                            .h_full()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(palette::border(app_theme))
                            .bg(app_theme.background)
                            .overflow_hidden()
                            .child(
                                v_flex()
                                    .size_full()
                                    .child(
                                        div()
                                            .h(px(34.))
                                            .px_3()
                                            .flex()
                                            .items_center()
                                            .border_b_1()
                                            .border_color(palette::border(app_theme))
                                            .font_semibold()
                                            .child("策略列表"),
                                    )
                                    .child(
                                        div().flex_1().overflow_hidden().child(
                                            v_flex()
                                                .size_full()
                                                .gap_1()
                                                .p_2()
                                                .overflow_y_scrollbar()
                                                .children(strategy_items),
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(palette::border(app_theme))
                            .bg(app_theme.background)
                            .overflow_hidden()
                            .child(
                                v_flex()
                                    .size_full()
                                    .child(
                                        h_flex()
                                            .h(px(40.))
                                            .px_4()
                                            .items_center()
                                            .justify_between()
                                            .border_b_1()
                                            .border_color(palette::border(app_theme))
                                            .child(
                                                div()
                                                    .font_semibold()
                                                    .text_size(px(14.))
                                                    .child(doc.title),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(palette::muted(app_theme))
                                                    .child(doc.file),
                                            ),
                                    )
                                    .child(
                                        div().flex_1().overflow_hidden().child(
                                            v_flex().size_full().overflow_y_scrollbar().p_4().child(
                                                TextView::markdown(
                                                    ("strategy-help-doc", self.selected),
                                                    doc.content.to_string(),
                                                    window,
                                                    cx,
                                                )
                                                .selectable(true),
                                            ),
                                        ),
                                    ),
                            ),
                    ),
            )
    }
}
