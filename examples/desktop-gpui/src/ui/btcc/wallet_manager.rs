use crate::ui::palette;
use binance_tools::wallet::{
    BTCC_NATIVE_SEGWIT_PATH, BitcoinWallet, BtccAddressInfo, BtccExplorerClient, BtccSendRequest,
    BtccSignedTransaction, btcc_to_sats, build_signed_transaction, wallet_from_mnemonic,
    wallet_from_private_key_wif,
};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

pub struct WalletGeneratorPage {
    wallet: Option<BitcoinWallet>,
    transfer_address: Option<String>,
    balance: Option<BtccAddressInfo>,
    import_input: Entity<InputState>,
    to_input: Entity<InputState>,
    amount_input: Entity<InputState>,
    fee_rate_input: Entity<InputState>,
    loading: bool,
    import_dialog_open: bool,
    export_dialog_open: bool,
    status: Option<String>,
    error: Option<String>,
    copied: Option<String>,
    last_signed: Option<BtccSignedTransaction>,
    last_txid: Option<String>,
    _task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl WalletGeneratorPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let import_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("粘贴助记词或 WIF 私钥")
                .default_value("")
        });
        let to_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("收款地址，当前支持 cc1 开头地址")
                .default_value("")
        });
        let amount_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("发送数量，例如 0.1")
                .default_value("")
        });
        let fee_rate_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("费率 sat/vB")
                .default_value("2")
        });
        let _subscriptions = vec![
            cx.subscribe_in(&import_input, window, Self::on_input_event),
            cx.subscribe_in(&to_input, window, Self::on_input_event),
            cx.subscribe_in(&amount_input, window, Self::on_input_event),
            cx.subscribe_in(&fee_rate_input, window, Self::on_input_event),
        ];

        Self {
            wallet: None,
            transfer_address: None,
            balance: None,
            import_input,
            to_input,
            amount_input,
            fee_rate_input,
            loading: false,
            import_dialog_open: false,
            export_dialog_open: false,
            status: None,
            error: None,
            copied: None,
            last_signed: None,
            last_txid: None,
            _task: Task::ready(()),
            _subscriptions,
        }
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.status = None;
            self.error = None;
            self.last_signed = None;
            self.last_txid = None;
            cx.notify();
        }
    }

    fn import_wallet(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let value = self.input_value(&self.import_input, cx);
        let wallet = if value.split_whitespace().count() >= 12 {
            wallet_from_mnemonic(&value)
        } else {
            wallet_from_private_key_wif(&value)
        };

        match wallet {
            Ok(wallet) => {
                self.transfer_address = Some(wallet.address.clone());
                self.wallet = Some(wallet);
                self.balance = None;
                self.status = Some("钱包已导入".to_string());
                self.error = None;
                self.last_signed = None;
                self.last_txid = None;
                self.import_dialog_open = false;
            }
            Err(err) => self.error = Some(format!("导入失败：{err}")),
        }
        cx.notify();
    }

    fn open_import_dialog(&mut self, cx: &mut Context<Self>) {
        self.import_dialog_open = true;
        self.status = None;
        self.error = None;
        cx.notify();
    }

    pub fn set_transfer_address(&mut self, address: String, cx: &mut Context<Self>) {
        self.transfer_address = Some(address);
        self.wallet = None;
        self.balance = None;
        self.import_dialog_open = false;
        self.export_dialog_open = false;
        self.last_signed = None;
        self.last_txid = None;
        self.status = Some("已选择转账钱包，请先查询余额。".to_string());
        self.error = None;
        cx.notify();
    }

    fn close_import_dialog(&mut self, cx: &mut Context<Self>) {
        self.import_dialog_open = false;
        cx.notify();
    }

    fn open_export_dialog(&mut self, cx: &mut Context<Self>) {
        self.export_dialog_open = true;
        cx.notify();
    }

    fn close_export_dialog(&mut self, cx: &mut Context<Self>) {
        self.export_dialog_open = false;
        cx.notify();
    }

    fn refresh_balance(&mut self, cx: &mut Context<Self>) {
        let Some(address) = self
            .wallet
            .as_ref()
            .map(|wallet| wallet.address.clone())
            .or_else(|| self.transfer_address.clone())
        else {
            self.error = Some("请先导入钱包".to_string());
            cx.notify();
            return;
        };

        self.loading = true;
        self.status = Some("正在查询 BTCC 余额".to_string());
        self.error = None;
        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { BtccExplorerClient::default().address_info(&address) },
                )
                .await;

            _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(info) => {
                        this.balance = Some(info);
                        this.status = Some("余额已更新".to_string());
                    }
                    Err(err) => this.error = Some(format!("查询余额失败：{err}")),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn sign_transaction(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(wallet) = self.wallet.clone() else {
            self.error =
                Some("当前钱包还没有解锁签名，请先导入该地址对应的助记词或 WIF 私钥。".to_string());
            cx.notify();
            return;
        };
        let Some(balance) = self.balance.clone() else {
            self.error = Some("请先查询余额，获取可用 UTXO".to_string());
            cx.notify();
            return;
        };

        match self.build_send_request(cx) {
            Ok(request) => match build_signed_transaction(&wallet, &balance.utxos, &request) {
                Ok(signed) => {
                    self.status = Some(format!(
                        "交易已签名：输入 {} 个，手续费 {} sats，找零 {} sats",
                        signed.input_count, signed.fee_sats, signed.change_sats
                    ));
                    self.last_signed = Some(signed);
                    self.last_txid = None;
                    self.error = None;
                }
                Err(err) => self.error = Some(format!("签名失败：{err}")),
            },
            Err(err) => self.error = Some(err),
        }
        cx.notify();
    }

    fn broadcast_transaction(&mut self, cx: &mut Context<Self>) {
        let Some(rawtx) = self.last_signed.as_ref().map(|signed| signed.rawtx.clone()) else {
            self.error = Some("请先生成签名交易".to_string());
            cx.notify();
            return;
        };

        self.loading = true;
        self.status = Some("正在广播交易".to_string());
        self.error = None;
        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    BtccExplorerClient::default().broadcast_raw_transaction(&rawtx)
                })
                .await;

            _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(result) => {
                        this.last_txid = Some(result.txid.clone());
                        this.status = Some(format!("广播成功：{}", result.txid));
                    }
                    Err(err) => this.error = Some(format!("广播失败：{err}")),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn build_send_request(&self, cx: &mut Context<Self>) -> Result<BtccSendRequest, String> {
        let to_address = self.input_value(&self.to_input, cx);
        if to_address.trim().is_empty() {
            return Err("请输入收款地址".to_string());
        }

        let amount_sats = btcc_to_sats(&self.input_value(&self.amount_input, cx))
            .map_err(|err| format!("金额错误：{err}"))?;
        let fee_rate_sat_vb = self
            .input_value(&self.fee_rate_input, cx)
            .trim()
            .parse::<u64>()
            .map_err(|_| "费率必须是整数 sat/vB".to_string())?;

        Ok(BtccSendRequest {
            to_address,
            amount_sats,
            fee_rate_sat_vb,
        })
    }

    fn input_value(&self, input: &Entity<InputState>, cx: &mut Context<Self>) -> String {
        input.read_with(cx, |input, _| input.value()).to_string()
    }

    fn copy_value(&mut self, label: &'static str, value: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        self.copied = Some(format!("已复制 {label}"));
        cx.notify();
    }

    fn render_labeled_input(
        &self,
        label: &'static str,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme().clone();
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette::muted(&app_theme))
                    .child(label),
            )
            .child(Input::new(input).small())
            .into_any_element()
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
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_semibold()
                            .text_color(palette::text_strong(&cx.theme().clone()))
                            .child("账户信息"),
                    )
                    .child(
                        Button::new("export-btcc-wallet")
                            .outline()
                            .small()
                            .label("导出钱包")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_export_dialog(cx);
                            })),
                    ),
            )
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
            .into_any_element()
    }

    fn render_balance(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        let balance = self.balance.clone();
        let confirmed = balance
            .as_ref()
            .map(|b| format!("{:.8} BTCC", b.confirmed_btcc))
            .unwrap_or_else(|| "--".to_string());
        let unconfirmed = balance
            .as_ref()
            .map(|b| format!("{:.8} BTCC", b.unconfirmed_btcc))
            .unwrap_or_else(|| "--".to_string());
        let utxos = balance
            .as_ref()
            .map(|b| b.utxo_total.to_string())
            .unwrap_or_else(|| "--".to_string());
        let total = balance
            .as_ref()
            .map(|b| format!("{:.8} BTCC", b.total_btcc))
            .unwrap_or_else(|| "--".to_string());

        h_flex()
            .gap_2()
            .flex_wrap()
            .child(self.metric_card("确认余额", confirmed, cx))
            .child(self.metric_card("未确认", unconfirmed, cx))
            .child(self.metric_card("UTXO", utxos, cx))
            .child(self.metric_card("总额", total, cx))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette::muted(&app_theme))
                    .child("余额与 UTXO 来自 explorer.btc-classic.org"),
            )
            .into_any_element()
    }

    fn metric_card(
        &self,
        label: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme().clone();
        v_flex()
            .gap_2()
            .min_w(px(140.))
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(palette::border(&app_theme))
            .bg(app_theme.background)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette::muted(&app_theme))
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(18.))
                    .font_semibold()
                    .text_color(palette::text_strong(&app_theme))
                    .child(value),
            )
            .into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        v_flex()
            .gap_2()
            .when_some(self.status.clone(), |parent, status| {
                parent.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(app_theme.success.opacity(0.10))
                        .text_color(app_theme.success)
                        .child(status),
                )
            })
            .when_some(self.error.clone(), |parent, error| {
                parent.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(app_theme.danger.opacity(0.10))
                        .text_color(app_theme.danger)
                        .child(error),
                )
            })
            .when_some(self.copied.clone(), |parent, copied| {
                parent.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(app_theme.primary.opacity(0.10))
                        .text_color(app_theme.primary)
                        .child(copied),
                )
            })
            .into_any_element()
    }

    fn render_import_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(gpui::black().opacity(0.30))
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(560.))
                    .gap_4()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(&app_theme))
                    .bg(app_theme.background)
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_semibold()
                                    .text_color(palette::text_strong(&app_theme))
                                    .child("导入 BTCC 钱包"),
                            )
                            .child(
                                Button::new("close-import-wallet-dialog")
                                    .outline()
                                    .xsmall()
                                    .label("关闭")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_import_dialog(cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(20.))
                            .text_color(palette::muted(&app_theme))
                            .child("粘贴 12/24 个英文助记词，或粘贴 WIF 私钥。导入后可查询余额并发送交易。"),
                    )
                    .child(self.render_labeled_input(
                        "助记词 / WIF 私钥",
                        &self.import_input,
                        cx,
                    ))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("cancel-import-wallet")
                                    .outline()
                                    .small()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_import_dialog(cx);
                                    })),
                            )
                            .child(
                                Button::new("confirm-import-wallet")
                                    .primary()
                                    .small()
                                    .label("导入")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_wallet(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_export_dialog(&self, wallet: &BitcoinWallet, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(gpui::black().opacity(0.30))
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(640.))
                    .max_h(px(720.))
                    .gap_4()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(&app_theme))
                    .bg(app_theme.background)
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(18.))
                                            .font_semibold()
                                            .text_color(palette::text_strong(&app_theme))
                                            .child("导出 BTCC 钱包"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(app_theme.danger)
                                            .child("私钥和助记词泄露后资产无法追回，请只在安全环境中查看。"),
                                    ),
                            )
                            .child(
                                Button::new("close-export-wallet-dialog")
                                    .outline()
                                    .xsmall()
                                    .label("关闭")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_export_dialog(cx);
                                    })),
                            ),
                    )
                    .child(self.render_field(
                        "copy-export-wallet-address",
                        "BTCC 地址",
                        wallet.address.clone(),
                        false,
                        cx,
                    ))
                    .child(self.render_field(
                        "copy-export-private-key",
                        "WIF 私钥",
                        wallet.private_key_wif.clone(),
                        true,
                        cx,
                    ))
                    .when(!wallet.mnemonic.is_empty(), |parent| {
                        parent.child(self.render_field(
                            "copy-export-mnemonic",
                            "助记词",
                            wallet.mnemonic.clone(),
                            true,
                            cx,
                        ))
                    })
                    .child(
                        h_flex()
                            .justify_end()
                            .child(
                                Button::new("done-export-wallet")
                                    .primary()
                                    .small()
                                    .label("完成")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_export_dialog(cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();

        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(520.))
                    .gap_4()
                    .items_center()
                    .p_6()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(&app_theme))
                    .bg(app_theme.background)
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_semibold()
                            .text_color(palette::text_strong(&app_theme))
                            .child("先导入一个 BTCC 钱包"),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(px(13.))
                            .line_height(px(22.))
                            .text_color(palette::muted(&app_theme))
                            .child(
                                "导入助记词或 WIF 私钥后，页面会显示余额、UTXO 和发送交易表单。",
                            ),
                    )
                    .child(
                        Button::new("empty-import-wallet")
                            .primary()
                            .small()
                            .label("导入钱包")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_import_dialog(cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_send_panel(
        &self,
        signed: Option<BtccSignedTransaction>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme().clone();

        v_flex()
            .gap_3()
            .min_h(px(220.))
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(palette::border(&app_theme))
            .bg(app_theme.background)
            .child(
                div()
                    .text_size(px(16.))
                    .font_semibold()
                    .text_color(palette::text_strong(&app_theme))
                    .child("发送交易"),
            )
            .child(self.render_labeled_input("收款地址", &self.to_input, cx))
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(self.render_labeled_input(
                        "数量 BTCC",
                        &self.amount_input,
                        cx,
                    )))
                    .child(div().w(px(180.)).child(self.render_labeled_input(
                        "费率 sat/vB",
                        &self.fee_rate_input,
                        cx,
                    ))),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::muted(&app_theme))
                            .child("先生成签名交易，确认 raw transaction 后再广播。"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("sign-btcc-tx")
                                    .outline()
                                    .small()
                                    .label("生成签名交易")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.sign_transaction(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("broadcast-btcc-tx")
                                    .primary()
                                    .small()
                                    .label("广播交易")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.broadcast_transaction(cx);
                                    })),
                            ),
                    ),
            )
            .when_some(signed, |parent, signed| {
                parent
                    .child(self.render_field(
                        "copy-btcc-rawtx",
                        "已签名 Raw Transaction",
                        signed.rawtx,
                        true,
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::muted(&app_theme))
                            .child(format!(
                                "输入 {} sats，发送 {} sats，找零 {} sats，手续费 {} sats",
                                signed.total_input_sats,
                                signed.send_sats,
                                signed.change_sats,
                                signed.fee_sats
                            )),
                    )
            })
            .when_some(self.last_txid.clone(), |parent, txid| {
                parent.child(self.render_field("copy-btcc-txid", "广播 TXID", txid, false, cx))
            })
            .into_any_element()
    }

    fn render_wallet_workspace(
        &self,
        wallet: &BitcoinWallet,
        signed: Option<BtccSignedTransaction>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme().clone();

        h_flex()
            .gap_4()
            .items_start()
            .child(
                div()
                    .w(px(520.))
                    .min_h(px(390.))
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(&app_theme))
                    .bg(app_theme.background)
                    .child(self.render_wallet(wallet, cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .gap_4()
                    .child(
                        div()
                            .min_h(px(140.))
                            .p_4()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(palette::border(&app_theme))
                            .bg(app_theme.background)
                            .child(self.render_balance(cx)),
                    )
                    .child(
                        div()
                            .min_h(px(230.))
                            .child(self.render_send_panel(signed, cx)),
                    ),
            )
            .into_any_element()
    }

    fn render_transfer_workspace(
        &self,
        address: String,
        signed: Option<BtccSignedTransaction>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_theme = cx.theme().clone();
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(palette::border(&app_theme))
                    .bg(app_theme.background)
                    .child(
                        v_flex()
                            .w(px(120.))
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_semibold()
                                    .text_color(palette::text_strong(&app_theme))
                                    .child("转账钱包"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(&app_theme))
                                    .child("来自钱包列表"),
                            ),
                    )
                    .child(compact_info("BTCC 地址", address, px(320.), cx))
                    .child(compact_info(
                        "派生路径",
                        BTCC_NATIVE_SEGWIT_PATH.to_string(),
                        px(170.),
                        cx,
                    ))
                    .child(compact_info(
                        "地址类型",
                        "Native SegWit (cc1q)".to_string(),
                        px(160.),
                        cx,
                    ))
                    .child(compact_info(
                        "数据 API",
                        "https://api.btc-classic.org".to_string(),
                        px(210.),
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .gap_4()
                    .items_start()
                    .child(
                        div()
                            .w(px(430.))
                            .min_h(px(170.))
                            .p_4()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(palette::border(&app_theme))
                            .bg(app_theme.background)
                            .child(self.render_balance(cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(230.))
                            .child(self.render_send_panel(signed, cx)),
                    ),
            )
            .into_any_element()
    }
}

fn compact_info(
    label: &'static str,
    value: String,
    width: Pixels,
    cx: &mut Context<WalletGeneratorPage>,
) -> AnyElement {
    let app_theme = cx.theme().clone();
    v_flex()
        .w(width)
        .gap_2()
        .p_2()
        .rounded(px(6.))
        .border_1()
        .border_color(palette::border(&app_theme))
        .bg(app_theme.muted.opacity(0.06))
        .child(
            div()
                .text_size(px(11.))
                .text_color(palette::muted(&app_theme))
                .child(label),
        )
        .child(
            div()
                .font_family("monospace")
                .text_size(px(12.))
                .text_color(palette::text_strong(&app_theme))
                .child(value),
        )
        .into_any_element()
}

impl Render for WalletGeneratorPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme().clone();
        let wallet = self.wallet.clone();
        let transfer_address = self.transfer_address.clone();
        let export_wallet = self.wallet.clone();
        let signed = self.last_signed.clone();

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
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_semibold()
                                    .text_color(palette::text_strong(&app_theme))
                                    .child("BTCC 钱包管理"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(&app_theme))
                                    .child("导入钱包，查询余额，构建签名交易并通过 BTCC Explorer 广播。"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("open-import-wallet-dialog")
                                    .outline()
                                    .small()
                                    .label("导入钱包")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_import_dialog(cx);
                                    })),
                            )
                            .child(
                                Button::new("refresh-btcc-balance")
                                    .primary()
                                    .small()
                                    .label(if self.loading { "处理中..." } else { "查询余额" })
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh_balance(cx))),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .mt_3()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(app_theme.danger.opacity(0.25))
                    .bg(app_theme.danger.opacity(0.08))
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(app_theme.danger)
                    .child("安全提示：助记词和私钥等同于资产控制权。广播交易不可撤销，请先小额测试并确认收款地址、金额和手续费。"),
            )
            .child(div().mt_3().child(self.render_status(cx)))
            .child(
                div().mt_3().child(match wallet {
                    Some(wallet) => self.render_wallet_workspace(&wallet, signed, cx),
                    None => match transfer_address {
                        Some(address) => self.render_transfer_workspace(address, signed, cx),
                        None => self.render_empty_state(cx),
                    },
                }),
            )
            .when(self.import_dialog_open, |parent| {
                parent.child(self.render_import_dialog(cx))
            })
            .when_some(
                export_wallet.filter(|_| self.export_dialog_open),
                |parent, wallet| parent.child(self.render_export_dialog(&wallet, cx)),
            )
    }
}
