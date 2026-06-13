use crate::ui::palette;
use binance_tools::calculator::{AngleMode, Calculator};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{ActiveTheme, StyledExt, h_flex, v_flex};

const DIVIDE_LABEL: &str = "\u{00f7}";
const MULTIPLY_LABEL: &str = "\u{00d7}";
const MINUS_LABEL: &str = "\u{2212}";
const PI_LABEL: &str = "\u{03c0}";
const SQRT_LABEL: &str = "\u{221a}";

pub struct CalculatorPage {
    calculator: Calculator,
    error: Option<String>,
}

impl CalculatorPage {
    pub fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Self {
            calculator: Calculator::default(),
            error: None,
        }
    }

    fn press(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.error = self.calculator.press(key).err();
        cx.notify();
    }

    fn render_key(
        &self,
        id: &'static str,
        label: &'static str,
        kind: KeyKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme();
        let (bg, fg) = match kind {
            KeyKind::Primary => (app_theme.primary, app_theme.primary_foreground),
            KeyKind::Operator => (
                app_theme.muted.opacity(0.36),
                palette::text_strong(app_theme),
            ),
            KeyKind::Function => (app_theme.primary.opacity(0.14), app_theme.primary),
            KeyKind::Number => (
                app_theme.muted.opacity(0.18),
                palette::text_strong(app_theme),
            ),
            KeyKind::Active => (app_theme.primary.opacity(0.22), app_theme.primary),
        };

        div()
            .id(id)
            .h(px(54.))
            .flex_1()
            .rounded(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(bg)
            .text_color(fg)
            .text_size(px(15.))
            .font_medium()
            .hover(|style| style.opacity(0.86))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.press(label, cx)))
            .into_any_element()
    }

    fn render_mode_group(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        let is_deg = self.calculator.angle_mode() == AngleMode::Deg;

        h_flex()
            .h(px(54.))
            .flex_1()
            .rounded(px(20.))
            .overflow_hidden()
            .bg(app_theme.primary.opacity(0.12))
            .child(
                div()
                    .id("calculator-mode-deg")
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(15.))
                    .font_medium()
                    .bg(if is_deg {
                        app_theme.primary.opacity(0.16)
                    } else {
                        transparent_black()
                    })
                    .text_color(if is_deg {
                        app_theme.primary
                    } else {
                        palette::text(app_theme)
                    })
                    .child("Deg")
                    .on_click(cx.listener(|this, _, _, cx| this.press("Deg", cx))),
            )
            .child(div().w(px(1.)).h(px(18.)).bg(palette::border(app_theme)))
            .child(
                div()
                    .id("calculator-mode-rad")
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(15.))
                    .font_medium()
                    .bg(if !is_deg {
                        app_theme.primary.opacity(0.16)
                    } else {
                        transparent_black()
                    })
                    .text_color(if !is_deg {
                        app_theme.primary
                    } else {
                        palette::text(app_theme)
                    })
                    .child("Rad")
                    .on_click(cx.listener(|this, _, _, cx| this.press("Rad", cx))),
            )
            .into_any_element()
    }

    fn render_row(&self, keys: Vec<AnyElement>) -> AnyElement {
        h_flex().gap_2().children(keys).into_any_element()
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let inv = self.calculator.inverse();
        vec![
            self.render_row(vec![
                self.render_mode_group(cx),
                self.render_key("calculator-factorial", "x!", KeyKind::Function, cx),
                self.render_key("calculator-left-paren", "(", KeyKind::Function, cx),
                self.render_key("calculator-right-paren", ")", KeyKind::Function, cx),
                self.render_key("calculator-percent", "%", KeyKind::Function, cx),
                self.render_key("calculator-clear", "AC", KeyKind::Function, cx),
            ]),
            self.render_row(vec![
                self.render_key(
                    "calculator-inv",
                    "Inv",
                    if inv {
                        KeyKind::Active
                    } else {
                        KeyKind::Function
                    },
                    cx,
                ),
                self.render_key("calculator-sin", "sin", KeyKind::Function, cx),
                self.render_key("calculator-ln", "ln", KeyKind::Function, cx),
                self.render_key("calculator-7", "7", KeyKind::Number, cx),
                self.render_key("calculator-8", "8", KeyKind::Number, cx),
                self.render_key("calculator-9", "9", KeyKind::Number, cx),
                self.render_key("calculator-divide", DIVIDE_LABEL, KeyKind::Operator, cx),
            ]),
            self.render_row(vec![
                self.render_key("calculator-pi", PI_LABEL, KeyKind::Function, cx),
                self.render_key("calculator-cos", "cos", KeyKind::Function, cx),
                self.render_key("calculator-log", "log", KeyKind::Function, cx),
                self.render_key("calculator-4", "4", KeyKind::Number, cx),
                self.render_key("calculator-5", "5", KeyKind::Number, cx),
                self.render_key("calculator-6", "6", KeyKind::Number, cx),
                self.render_key("calculator-multiply", MULTIPLY_LABEL, KeyKind::Operator, cx),
            ]),
            self.render_row(vec![
                self.render_key("calculator-e", "e", KeyKind::Function, cx),
                self.render_key("calculator-tan", "tan", KeyKind::Function, cx),
                self.render_key("calculator-sqrt", SQRT_LABEL, KeyKind::Function, cx),
                self.render_key("calculator-1", "1", KeyKind::Number, cx),
                self.render_key("calculator-2", "2", KeyKind::Number, cx),
                self.render_key("calculator-3", "3", KeyKind::Number, cx),
                self.render_key("calculator-minus", MINUS_LABEL, KeyKind::Operator, cx),
            ]),
            self.render_row(vec![
                self.render_key("calculator-ans", "Ans", KeyKind::Function, cx),
                self.render_key("calculator-exp", "EXP", KeyKind::Function, cx),
                self.render_key("calculator-power", "x^y", KeyKind::Function, cx),
                self.render_key("calculator-0", "0", KeyKind::Number, cx),
                self.render_key("calculator-dot", ".", KeyKind::Number, cx),
                self.render_key("calculator-equals", "=", KeyKind::Primary, cx),
                self.render_key("calculator-plus", "+", KeyKind::Operator, cx),
            ]),
        ]
    }
}

impl Render for CalculatorPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.render_rows(cx);
        let app_theme = cx.theme();

        v_flex().size_full().items_center().justify_center().child(
            v_flex()
                .w(px(920.))
                .max_w_full()
                .gap_3()
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .h(px(94.))
                                .rounded(px(16.))
                                .border_1()
                                .border_color(palette::border(app_theme))
                                .bg(app_theme.background)
                                .px_4()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(36.))
                                        .text_size(px(22.))
                                        .text_color(palette::muted(app_theme))
                                        .child("\u{21ba}"),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_align(TextAlign::Right)
                                        .text_size(px(42.))
                                        .text_color(palette::text_strong(app_theme))
                                        .child(self.calculator.display().to_string()),
                                ),
                        )
                        .when_some(self.error.as_ref(), |parent, error| {
                            parent.child(
                                div()
                                    .px_2()
                                    .text_size(px(12.))
                                    .text_color(app_theme.danger)
                                    .child(error.clone()),
                            )
                        }),
                )
                .children(rows)
                .child(
                    h_flex()
                        .mt_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(palette::border(app_theme))
                        .justify_center()
                        .text_size(px(15.))
                        .text_color(palette::text_strong(app_theme))
                        .child("数学求解器")
                        .child(
                            div()
                                .ml_2()
                                .text_color(palette::muted(app_theme))
                                .child("\u{203a}"),
                        ),
                ),
        )
    }
}

#[derive(Clone, Copy)]
enum KeyKind {
    Primary,
    Operator,
    Function,
    Number,
    Active,
}
