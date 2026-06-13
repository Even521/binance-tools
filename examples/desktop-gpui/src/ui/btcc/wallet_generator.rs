use crate::ui::palette;
use binance_tools::wallet::{BitcoinWallet, generate_bitcoin_wallet};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};

pub struct WalletGeneratorPage {
    wallet: Option<BitcoinWallet>,
    error: Option<String>,
    copied: Option<String>,
}

impl WalletGeneratorPage {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            wallet: None,
            error: None,
            copied: None,
        };
        page.generate(cx);
        page
    }

    fn generate(&mut self, cx: &mut Context<Self>) {
        match generate_bitcoin_wallet() {
            Ok(wallet) => {
                self.wallet = Some(wallet);
                self.error = None;
                self.copied = None;
            }
            Err(err) => {
                self.wallet = None;
                self.error = Some(err.to_string());
                self.copied = None;
            }
        }
        cx.notify();
    }

    fn copy_value(&mut self, label: &'static str, value: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        self.copied = Some(format!("已复制 {label}"));
        cx.notify();
    }

    fn render_field(
        &self,
        id: &'static str,
        label: &'static str,
        value: String,
        sensitive: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme().clone();
        let copy_value = value.clone();

        v_flex()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(palette::border(&app_theme))
            .bg(if sensitive {
                app_theme.danger.opacity(0.06)
            } else {
                app_theme.background
            })
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(if sensitive {
                                app_theme.danger
                            } else {
                                palette::muted(&app_theme)
                            })
                            .child(label),
                    )
                    .child(
                        Button::new(id)
                            .outline()
                            .xsmall()
                            .label("复制")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy_value(label, copy_value.clone(), cx);
                            })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .p_3()
                    .rounded(px(6.))
                    .bg(app_theme.muted.opacity(0.10))
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .font_family(app_theme.mono_font_family.clone())
                    .text_color(palette::text_strong(&app_theme))
                    .child(value),
            )
            .into_any_element()
    }

    fn render_wallet(&self, wallet: &BitcoinWallet, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .child(self.render_field(
                "copy-wallet-network",
                "网络",
                wallet.network.clone(),
                false,
                cx,
            ))
            .child(self.render_field(
                "copy-wallet-address",
                "BTCC 地址",
                wallet.address.clone(),
                false,
                cx,
            ))
            .child(self.render_field(
                "copy-wallet-path",
                "派生路径",
                wallet.derivation_path.clone(),
                false,
                cx,
            ))
            .child(self.render_field(
                "copy-wallet-private-key",
                "WIF 私钥",
                wallet.private_key_wif.clone(),
                true,
                cx,
            ))
            .when(!wallet.mnemonic.is_empty(), |parent| {
                parent.child(self.render_field(
                    "copy-wallet-mnemonic",
                    "助记词",
                    wallet.mnemonic.clone(),
                    true,
                    cx,
                ))
            })
            .into_any_element()
    }
}

impl Render for WalletGeneratorPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme().clone();
        let wallet = self.wallet.clone();

        v_flex()
            .size_full()
            .p_4()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(&app_theme))
                    .bg(app_theme.background)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_semibold()
                                    .text_color(palette::text_strong(&app_theme))
                                    .child("BTCC 钱包生成"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(&app_theme))
                                    .child("离线生成 BTCC 助记词、地址和 WIF 私钥；不写入数据库。"),
                            ),
                    )
                    .child(
                        Button::new("generate-btcc-wallet")
                            .primary()
                            .small()
                            .label("生成新钱包")
                            .on_click(cx.listener(|this, _, _, cx| this.generate(cx))),
                    ),
            )
            .child(
                v_flex()
                    .mt_3()
                    .gap_2()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(app_theme.danger.opacity(0.25))
                    .bg(app_theme.danger.opacity(0.08))
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(app_theme.danger)
                    .child("安全提示：助记词和私钥等同于资产控制权。请勿截图、请勿发送给任何人。"),
            )
            .when_some(self.copied.clone(), |parent, copied| {
                parent.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(app_theme.success.opacity(0.12))
                        .text_color(app_theme.success)
                        .child(copied),
                )
            })
            .when_some(self.error.clone(), |parent, error| {
                parent.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(app_theme.danger.opacity(0.12))
                        .text_color(app_theme.danger)
                        .child(error),
                )
            })
            .child(
                div().flex_1().overflow_y_scrollbar().child(match wallet {
                    Some(wallet) => div()
                        .mt_3()
                        .child(self.render_wallet(&wallet, cx))
                        .into_any_element(),
                    None => div()
                        .mt_3()
                        .p_4()
                        .text_color(palette::muted(&app_theme))
                        .child("点击生成新钱包")
                        .into_any_element(),
                }),
            )
    }
}
