use crate::ui::palette;
use binance_tools::{
    db::btcc_wallet::{
        BtccWalletRecord, BtccWalletSecrets, btcc_wallet_password_exists_blocking,
        create_btcc_wallet_password_blocking, create_encrypted_btcc_wallet_blocking,
        decrypt_btcc_wallet_secrets_blocking, delete_btcc_wallet_blocking,
        list_btcc_wallets_blocking, update_btcc_wallet_blocking,
    },
    wallet::{BitcoinWallet, generate_btcc_wallet},
};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub enum BtccWalletListEvent {
    OpenTransfer(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorMode {
    CreateMnemonic,
    VerifyMnemonic,
    EditExisting,
}

pub struct BtccWalletListPage {
    wallets: Vec<BtccWalletRecord>,
    selected_id: Option<i64>,
    editor_open: bool,
    editor_mode: EditorMode,
    generated_wallet: Option<BitcoinWallet>,
    verify_positions: Vec<usize>,
    vault_initialized: bool,
    export_wallet_id: Option<i64>,
    exported_secrets: Option<BtccWalletSecrets>,
    name_input: Entity<InputState>,
    address_input: Entity<InputState>,
    note_input: Entity<InputState>,
    vault_password_input: Entity<InputState>,
    vault_confirm_input: Entity<InputState>,
    action_password_input: Entity<InputState>,
    verify_inputs: Vec<Entity<InputState>>,
    status: Option<String>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl BtccWalletListPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("钱包名称，例如 主钱包")
                .default_value("")
        });
        let address_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("BTCC 地址，cc1 开头")
                .default_value("")
        });
        let note_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(2, 5)
                .placeholder("备注，可选")
                .default_value("")
        });
        let vault_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("至少 6 位钱包密码")
                .default_value("")
        });
        let vault_confirm_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("再次输入钱包密码")
                .default_value("")
        });
        let action_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入钱包密码")
                .default_value("")
        });
        let verify_inputs = (0..3)
            .map(|_| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("输入对应单词")
                        .default_value("")
                })
            })
            .collect::<Vec<_>>();

        let mut _subscriptions = vec![
            cx.subscribe_in(&name_input, window, Self::on_input_event),
            cx.subscribe_in(&address_input, window, Self::on_input_event),
            cx.subscribe_in(&note_input, window, Self::on_input_event),
            cx.subscribe_in(&vault_password_input, window, Self::on_input_event),
            cx.subscribe_in(&vault_confirm_input, window, Self::on_input_event),
            cx.subscribe_in(&action_password_input, window, Self::on_input_event),
        ];
        for input in &verify_inputs {
            _subscriptions.push(cx.subscribe_in(input, window, Self::on_input_event));
        }

        let mut page = Self {
            wallets: Vec::new(),
            selected_id: None,
            editor_open: false,
            editor_mode: EditorMode::CreateMnemonic,
            generated_wallet: None,
            verify_positions: Vec::new(),
            vault_initialized: false,
            export_wallet_id: None,
            exported_secrets: None,
            name_input,
            address_input,
            note_input,
            vault_password_input,
            vault_confirm_input,
            action_password_input,
            verify_inputs,
            status: None,
            error: None,
            _subscriptions,
        };
        page.reload_vault_state();
        page.reload();
        page
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
            cx.notify();
        }
    }

    fn reload(&mut self) {
        match list_btcc_wallets_blocking() {
            Ok(wallets) => {
                self.wallets = wallets;
                self.error = None;
            }
            Err(err) => {
                self.error = Some(err.to_string());
            }
        }
    }

    fn reload_vault_state(&mut self) {
        match btcc_wallet_password_exists_blocking() {
            Ok(exists) => {
                self.vault_initialized = exists;
                self.error = None;
            }
            Err(err) => {
                self.vault_initialized = false;
                self.error = Some(err.to_string());
            }
        }
    }

    fn create_vault_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.vault_password_input.read(cx).text().to_string();
        let confirm = self.vault_confirm_input.read(cx).text().to_string();
        if password.chars().count() < 6 {
            self.error = Some("钱包密码不能少于 6 位。".to_string());
            self.status = None;
            cx.notify();
            return;
        }
        if password != confirm {
            self.error = Some("两次输入的钱包密码不一致。".to_string());
            self.status = None;
            cx.notify();
            return;
        }

        match create_btcc_wallet_password_blocking(password) {
            Ok(()) => {
                self.vault_initialized = true;
                self.status = Some("钱包密码已设置。".to_string());
                self.error = None;
                self.vault_password_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.vault_confirm_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(err) => {
                self.error = Some(err.to_string());
                self.status = None;
            }
        }
        cx.notify();
    }

    fn open_create_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match generate_btcc_wallet() {
            Ok(wallet) => {
                self.selected_id = None;
                self.editor_open = true;
                self.editor_mode = EditorMode::CreateMnemonic;
                self.verify_positions.clear();
                self.generated_wallet = Some(wallet);
                self.name_input
                    .update(cx, |input, cx| input.set_value("BTCC 钱包", window, cx));
                self.address_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.note_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                for input in &self.verify_inputs {
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                }
                self.status = Some("已生成新助记词，请先手抄保存。".to_string());
                self.error = None;
            }
            Err(err) => {
                self.error = Some(format!("生成钱包失败：{err}"));
                self.status = None;
            }
        }
        cx.notify();
    }

    fn open_edit_editor(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wallet) = self.wallets.iter().find(|wallet| wallet.id == id).cloned() else {
            return;
        };
        self.selected_id = Some(id);
        self.editor_open = true;
        self.editor_mode = EditorMode::EditExisting;
        self.generated_wallet = None;
        self.verify_positions.clear();
        self.name_input
            .update(cx, |input, cx| input.set_value(wallet.name, window, cx));
        self.address_input
            .update(cx, |input, cx| input.set_value(wallet.address, window, cx));
        self.note_input
            .update(cx, |input, cx| input.set_value(wallet.note, window, cx));
        self.status = None;
        self.error = None;
        cx.notify();
    }

    fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.editor_open = false;
        self.selected_id = None;
        self.generated_wallet = None;
        self.verify_positions.clear();
        self.error = None;
        cx.notify();
    }

    fn start_verify_generated_wallet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wallet) = &self.generated_wallet else {
            self.error = Some("没有可验证的钱包，请重新创建。".to_string());
            self.status = None;
            cx.notify();
            return;
        };
        let words = mnemonic_words(wallet);
        if words.len() < 3 {
            self.error = Some("助记词数量不足，无法验证。".to_string());
            self.status = None;
            cx.notify();
            return;
        }

        self.verify_positions = choose_verify_positions(words.len());
        for input in &self.verify_inputs {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.editor_mode = EditorMode::VerifyMnemonic;
        self.status = Some("请输入随机抽取的 3 个助记词。".to_string());
        self.error = None;
        cx.notify();
    }

    fn verify_and_save_generated_wallet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wallet) = self.generated_wallet.clone() else {
            self.error = Some("没有可保存的钱包，请重新创建。".to_string());
            self.status = None;
            cx.notify();
            return;
        };

        let words = mnemonic_words(&wallet);
        for (index, position) in self.verify_positions.iter().enumerate() {
            let expected = words.get(*position).copied().unwrap_or_default();
            let actual = self.verify_inputs[index].read(cx).text().to_string();
            let actual = actual.trim();
            if !actual.eq_ignore_ascii_case(expected) {
                self.error = Some(format!("第 {} 个单词验证失败，请重新核对。", position + 1));
                self.status = None;
                cx.notify();
                return;
            }
        }

        let name = self.name_input.read(cx).text().to_string();
        let note = self.note_input.read(cx).text().to_string();
        let password = self.action_password_input.read(cx).text().to_string();
        if password.chars().count() < 6 {
            self.error = Some("请输入 6 位以上钱包密码，用于加密保存。".to_string());
            self.status = None;
            cx.notify();
            return;
        }

        let result = create_encrypted_btcc_wallet_blocking(
            name,
            wallet.address.clone(),
            wallet.derivation_path.clone(),
            "generated".to_string(),
            wallet.public_key.to_string(),
            note,
            wallet.mnemonic.clone(),
            wallet.private_key_wif.clone(),
            password,
        );

        match result {
            Ok(_) => {
                self.reload();
                self.editor_open = false;
                self.generated_wallet = None;
                self.verify_positions.clear();
                self.action_password_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.status =
                    Some("验证通过，钱包已加密保存。数据库不会保存明文助记词或私钥。".to_string());
                self.error = None;
            }
            Err(err) => {
                self.error = Some(err.to_string());
                self.status = None;
            }
        }
        cx.notify();
    }

    fn save_existing_wallet(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id else {
            self.error = Some("请先选择一个钱包。".to_string());
            self.status = None;
            cx.notify();
            return;
        };

        let name = self.name_input.read(cx).text().to_string();
        let note = self.note_input.read(cx).text().to_string();

        match update_btcc_wallet_blocking(id, name, note) {
            Ok(()) => {
                self.reload();
                self.editor_open = false;
                self.selected_id = None;
                self.status = Some("钱包已保存".to_string());
                self.error = None;
            }
            Err(err) => {
                self.error = Some(err.to_string());
                self.status = None;
            }
        }
        cx.notify();
    }

    fn delete_wallet(&mut self, id: i64, cx: &mut Context<Self>) {
        match delete_btcc_wallet_blocking(id) {
            Ok(()) => {
                self.reload();
                self.editor_open = false;
                self.selected_id = None;
                self.status = Some("钱包已移除".to_string());
                self.error = None;
            }
            Err(err) => {
                self.error = Some(err.to_string());
                self.status = None;
            }
        }
        cx.notify();
    }

    fn copy_value(&mut self, label: &'static str, value: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        self.status = Some(format!("已复制 {label}"));
        self.error = None;
        cx.notify();
    }

    fn open_transfer(&mut self, address: String, cx: &mut Context<Self>) {
        cx.emit(BtccWalletListEvent::OpenTransfer(address));
    }

    fn open_export(&mut self, wallet_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.export_wallet_id = Some(wallet_id);
        self.exported_secrets = None;
        self.error = None;
        self.status = None;
        self.action_password_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn close_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.export_wallet_id = None;
        self.exported_secrets = None;
        self.error = None;
        self.action_password_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn decrypt_export(&mut self, cx: &mut Context<Self>) {
        let Some(wallet_id) = self.export_wallet_id else {
            self.error = Some("请先选择要导出的钱包。".to_string());
            cx.notify();
            return;
        };
        let password = self.action_password_input.read(cx).text().to_string();
        match decrypt_btcc_wallet_secrets_blocking(wallet_id, password) {
            Ok(secrets) => {
                self.exported_secrets = Some(secrets);
                self.status = Some("钱包已解密，请只在安全环境查看。".to_string());
                self.error = None;
            }
            Err(err) => {
                self.exported_secrets = None;
                self.error = Some(err.to_string());
                self.status = None;
            }
        }
        cx.notify();
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        h_flex()
            .items_center()
            .justify_between()
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
                            .child("BTCC 钱包列表"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette::muted(&app_theme))
                            .child("创建钱包必须先手抄助记词，并验证 3 个随机单词后才会保存。"),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("btcc-wallet-create")
                            .label("+ 创建钱包")
                            .primary()
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_create_editor(window, cx);
                            })),
                    )
                    .child(
                        Button::new("btcc-wallet-list-refresh")
                            .label("刷新")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reload();
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_vault_setup(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        v_flex()
            .mt_3()
            .gap_4()
            .p_5()
            .rounded(px(8.))
            .border_1()
            .border_color(app_theme.primary.opacity(0.35))
            .bg(app_theme.background)
            .child(
                div()
                    .text_size(px(18.))
                    .font_semibold()
                    .text_color(palette::text_strong(&app_theme))
                    .child("设置 BTCC 钱包密码"),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(palette::muted(&app_theme))
                    .child("首次使用需要设置 6 位以上密码。后续创建钱包会用它加密助记词和私钥，导出钱包时也需要它解密。"),
            )
            .child(field("钱包密码", self.vault_password_input.clone()))
            .child(field("确认密码", self.vault_confirm_input.clone()))
            .child(
                h_flex().justify_end().child(
                    Button::new("create-btcc-wallet-vault-password")
                        .label("保存密码")
                        .primary()
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.create_vault_password(window, cx);
                        })),
                ),
            )
            .into_any_element()
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.editor_mode {
            EditorMode::CreateMnemonic => self.render_mnemonic_step(cx),
            EditorMode::VerifyMnemonic => self.render_verify_step(cx),
            EditorMode::EditExisting => self.render_edit_existing(cx),
        }
    }

    fn render_mnemonic_step(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        let wallet = self.generated_wallet.as_ref();

        v_flex()
            .mt_3()
            .gap_4()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(app_theme.primary.opacity(0.35))
            .bg(app_theme.background)
            .child(editor_title("创建钱包：1/2 手抄助记词", cx))
            .child(
                div()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(app_theme.warning.opacity(0.35))
                    .bg(app_theme.warning.opacity(0.08))
                    .text_size(px(12.))
                    .text_color(app_theme.warning.opacity(0.95))
                    .child("助记词等于资产控制权。请离线手抄，不要截图，不要发给任何人。"),
            )
            .child(field("钱包名称", self.name_input.clone()))
            .when_some(wallet, |el, wallet| {
                el.child(
                    v_flex()
                        .gap_4()
                        .child(readonly_field("BTCC 地址", wallet.address.clone(), cx))
                        .child(readonly_field(
                            "派生路径",
                            wallet.derivation_path.clone(),
                            cx,
                        ))
                        .child(mnemonic_grid(mnemonic_words(wallet), cx)),
                )
            })
            .child(field("备注", self.note_input.clone()))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("btcc-wallet-create-close")
                            .label("关闭")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.close_editor(cx))),
                    )
                    .child(
                        Button::new("btcc-wallet-create-next")
                            .label("下一步验证")
                            .primary()
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_verify_generated_wallet(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_verify_step(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        v_flex()
            .mt_3()
            .gap_3()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(app_theme.primary.opacity(0.35))
            .bg(app_theme.background)
            .child(editor_title("创建钱包：2/2 验证助记词", cx))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette::muted(&app_theme))
                    .child("请输入下面序号对应的助记词。全部正确后，钱包地址才会保存到列表。"),
            )
            .child(
                h_flex().gap_3().items_start().children(
                    self.verify_positions
                        .iter()
                        .enumerate()
                        .map(|(index, pos)| {
                            div()
                                .flex_1()
                                .child(field(
                                    Box::leak(format!("第 {} 个单词", pos + 1).into_boxed_str()),
                                    self.verify_inputs[index].clone(),
                                ))
                                .into_any_element()
                        }),
                ),
            )
            .child(field("钱包密码", self.action_password_input.clone()))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("btcc-wallet-verify-back")
                            .label("返回")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.editor_mode = EditorMode::CreateMnemonic;
                                this.error = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("btcc-wallet-verify-save")
                            .label("验证并保存")
                            .primary()
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.verify_and_save_generated_wallet(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_edit_existing(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        v_flex()
            .mt_3()
            .gap_3()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(app_theme.primary.opacity(0.35))
            .bg(app_theme.background)
            .child(editor_title("编辑钱包", cx))
            .child(field("名称", self.name_input.clone()))
            .child(readonly_field(
                "BTCC 地址",
                self.address_input.read(cx).text().to_string(),
                cx,
            ))
            .child(field("备注", self.note_input.clone()))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("btcc-wallet-edit-close")
                            .label("关闭")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.close_editor(cx))),
                    )
                    .child(
                        Button::new("btcc-wallet-edit-save")
                            .label("保存")
                            .primary()
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_existing_wallet(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_export_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        let wallet = self
            .export_wallet_id
            .and_then(|id| self.wallets.iter().find(|wallet| wallet.id == id));

        v_flex()
            .mt_3()
            .gap_3()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(app_theme.warning.opacity(0.35))
            .bg(app_theme.background)
            .child(editor_title("导出钱包", cx))
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(20.))
                    .text_color(app_theme.warning.opacity(0.95))
                    .child("只有输入钱包密码后才会解密显示助记词和 WIF 私钥。导出后请在安全环境保存，关闭面板会清空本次显示内容。"),
            )
            .when_some(wallet.cloned(), |el, wallet| {
                el.child(readonly_field("钱包名称", wallet.name, cx))
                    .child(readonly_field("BTCC 地址", wallet.address, cx))
            })
            .child(field("钱包密码", self.action_password_input.clone()))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("btcc-wallet-export-close")
                            .label("关闭")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_export(window, cx);
                            })),
                    )
                    .child(
                        Button::new("btcc-wallet-export-decrypt")
                            .label("解密导出")
                            .primary()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.decrypt_export(cx);
                            })),
                    ),
            )
            .when_some(self.exported_secrets.clone(), |el, secrets| {
                let mnemonic = secrets.mnemonic.clone();
                let wif = secrets.private_key_wif.clone();
                el.child(copyable_secret_field("助记词", mnemonic, "助记词", cx))
                    .child(copyable_secret_field("WIF 私钥", wif, "WIF 私钥", cx))
            })
            .into_any_element()
    }

    fn render_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        let total_balance: i64 = self.wallets.iter().map(|wallet| wallet.balance_sats).sum();
        let total_utxo: i64 = self.wallets.iter().map(|wallet| wallet.utxo_count).sum();

        v_flex()
            .mt_3()
            .rounded(px(8.))
            .border_1()
            .border_color(palette::border(&app_theme))
            .bg(app_theme.background)
            .overflow_hidden()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(palette::border(&app_theme))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_semibold()
                            .text_color(palette::text_strong(&app_theme))
                            .child("钱包列表"),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .text_size(px(12.))
                            .text_color(palette::muted(&app_theme))
                            .child(format!("钱包数量 {}", self.wallets.len()))
                            .child(format!("总余额 {}", format_sats(total_balance)))
                            .child(format!("UTXO {}", total_utxo)),
                    ),
            )
            .child(self.render_table_header(cx))
            .when(self.wallets.is_empty(), |el| {
                el.child(
                    div()
                        .p_6()
                        .text_size(px(13.))
                        .text_color(palette::muted(&app_theme))
                        .child("还没有钱包。点击右上角“创建钱包”生成并验证助记词。"),
                )
            })
            .children(
                self.wallets
                    .iter()
                    .map(|wallet| self.render_wallet_row(wallet, cx)),
            )
            .into_any_element()
    }

    fn render_table_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        h_flex()
            .items_center()
            .px_3()
            .py_2()
            .gap_3()
            .bg(app_theme.muted.opacity(0.10))
            .border_b_1()
            .border_color(palette::border(&app_theme))
            .text_size(px(12.))
            .text_color(palette::muted(&app_theme))
            .child(col("钱包名称", 160.))
            .child(col("BTCC 地址", 320.))
            .child(col("余额", 140.))
            .child(col("未确认", 120.))
            .child(col("UTXO", 70.))
            .child(col("来源", 90.))
            .child(col("最近同步", 150.))
            .child(div().w(px(260.)).child("操作"))
            .into_any_element()
    }

    fn render_wallet_row(&self, wallet: &BtccWalletRecord, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme().clone();
        let address = wallet.address.clone();
        let transfer_address = wallet.address.clone();
        let edit_id = wallet.id;
        let export_id = wallet.id;
        let delete_id = wallet.id;

        h_flex()
            .items_center()
            .px_3()
            .py_2()
            .gap_3()
            .border_b_1()
            .border_color(palette::border(&app_theme))
            .text_size(px(12.))
            .child(
                div()
                    .w(px(160.))
                    .font_semibold()
                    .text_color(palette::text_strong(&app_theme))
                    .child(wallet.name.clone()),
            )
            .child(
                div()
                    .w(px(320.))
                    .font_family("monospace")
                    .text_color(palette::text_strong(&app_theme))
                    .child(wallet.address.clone()),
            )
            .child(cell(format_sats(wallet.balance_sats), 140., cx))
            .child(cell(format_sats(wallet.unconfirmed_sats), 120., cx))
            .child(cell(wallet.utxo_count.to_string(), 70., cx))
            .child(cell(wallet.source_type.clone(), 90., cx))
            .child(cell(
                wallet
                    .last_synced_at
                    .clone()
                    .unwrap_or_else(|| "--".to_string()),
                150.,
                cx,
            ))
            .child(
                h_flex()
                    .w(px(260.))
                    .gap_2()
                    .child(
                        Button::new(("btcc-wallet-transfer", wallet.id as u64))
                            .label("转账")
                            .primary()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_transfer(transfer_address.clone(), cx);
                            })),
                    )
                    .child(
                        Button::new(("btcc-wallet-edit", wallet.id as u64))
                            .label("编辑")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_edit_editor(edit_id, window, cx);
                            })),
                    )
                    .child(
                        Button::new(("btcc-wallet-export", wallet.id as u64))
                            .label("导出")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_export(export_id, window, cx);
                            })),
                    )
                    .child(
                        Button::new(("btcc-wallet-copy", wallet.id as u64))
                            .label("复制")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy_value("BTCC 地址", address.clone(), cx);
                            })),
                    )
                    .child(
                        Button::new(("btcc-wallet-delete", wallet.id as u64))
                            .label("删除")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.delete_wallet(delete_id, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl EventEmitter<BtccWalletListEvent> for BtccWalletListPage {}

impl Render for BtccWalletListPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .gap_4()
            .overflow_y_scrollbar()
            .child(self.render_header(cx))
            .when_some(self.error.clone(), |el, error| {
                el.child(message_box(error, true, cx))
            })
            .when_some(self.status.clone(), |el, status| {
                el.child(message_box(status, false, cx))
            })
            .when(!self.vault_initialized, |el| {
                el.child(self.render_vault_setup(cx))
            })
            .when(self.vault_initialized && self.editor_open, |el| {
                el.child(self.render_editor(cx))
            })
            .when(
                self.vault_initialized && !self.editor_open && self.export_wallet_id.is_some(),
                |el| el.child(self.render_export_panel(cx)),
            )
            .when(self.vault_initialized && !self.editor_open, |el| {
                el.child(self.render_table(cx))
            })
    }
}

fn field(label: &'static str, input: Entity<InputState>) -> AnyElement {
    v_flex()
        .gap_2()
        .child(div().text_size(px(12.)).child(label))
        .child(Input::new(&input).small())
        .into_any_element()
}

fn readonly_field(
    label: &'static str,
    value: String,
    cx: &mut Context<BtccWalletListPage>,
) -> AnyElement {
    let app_theme = cx.theme().clone();
    v_flex()
        .gap_2()
        .child(div().text_size(px(12.)).child(label))
        .child(
            div()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(palette::border(&app_theme))
                .font_family("monospace")
                .text_size(px(12.))
                .text_color(palette::text_strong(&app_theme))
                .child(value),
        )
        .into_any_element()
}

fn copyable_secret_field(
    label: &'static str,
    value: String,
    copy_label: &'static str,
    cx: &mut Context<BtccWalletListPage>,
) -> AnyElement {
    let app_theme = cx.theme().clone();
    let copy_value = value.clone();
    let copy_id = if copy_label == "助记词" {
        1_u64
    } else {
        2_u64
    };
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .justify_between()
                .child(div().text_size(px(12.)).child(label))
                .child(
                    Button::new(("btcc-wallet-secret-copy", copy_id))
                        .label("复制")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_value(copy_label, copy_value.clone(), cx);
                        })),
                ),
        )
        .child(
            div()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(palette::border(&app_theme))
                .font_family("monospace")
                .text_size(px(12.))
                .line_height(px(20.))
                .text_color(palette::text_strong(&app_theme))
                .child(value),
        )
        .into_any_element()
}

fn editor_title(title: &'static str, cx: &mut Context<BtccWalletListPage>) -> AnyElement {
    let app_theme = cx.theme();
    div()
        .text_size(px(16.))
        .font_semibold()
        .text_color(palette::text_strong(app_theme))
        .child(title)
        .into_any_element()
}

fn mnemonic_grid(words: Vec<&str>, cx: &mut Context<BtccWalletListPage>) -> AnyElement {
    let app_theme = cx.theme().clone();
    v_flex()
        .gap_3()
        .children(words.chunks(6).enumerate().map(|(row_index, row)| {
            h_flex()
                .gap_3()
                .children(row.iter().enumerate().map(|(col_index, word)| {
                    let index = row_index * 6 + col_index;
                    h_flex()
                        .gap_2()
                        .w(px(150.))
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(palette::border(&app_theme))
                        .bg(app_theme.muted.opacity(0.08))
                        .child(
                            div()
                                .w(px(22.))
                                .text_size(px(11.))
                                .text_color(palette::muted(&app_theme))
                                .child(format!("{}", index + 1)),
                        )
                        .child(
                            div()
                                .font_family("monospace")
                                .text_size(px(13.))
                                .text_color(palette::text_strong(&app_theme))
                                .child((*word).to_string()),
                        )
                        .into_any_element()
                }))
                .into_any_element()
        }))
        .into_any_element()
}

fn col(label: &'static str, width: f32) -> AnyElement {
    div().w(px(width)).child(label).into_any_element()
}

fn cell(value: String, width: f32, cx: &mut Context<BtccWalletListPage>) -> AnyElement {
    div()
        .w(px(width))
        .text_color(palette::muted(cx.theme()))
        .child(value)
        .into_any_element()
}

fn message_box(
    text: String,
    danger: bool,
    cx: &mut Context<BtccWalletListPage>,
) -> impl IntoElement {
    let app_theme = cx.theme();
    let color = if danger {
        app_theme.danger
    } else {
        app_theme.success
    };
    div()
        .p_3()
        .rounded(px(8.))
        .border_1()
        .border_color(color.opacity(0.35))
        .bg(color.opacity(0.08))
        .text_size(px(12.))
        .text_color(color.opacity(0.95))
        .child(text)
}

fn format_sats(sats: i64) -> String {
    if sats == 0 {
        "--".to_string()
    } else {
        format!("{:.8} BTCC", sats as f64 / 100_000_000.0)
    }
}

fn mnemonic_words(wallet: &BitcoinWallet) -> Vec<&str> {
    wallet.mnemonic.split_whitespace().collect()
}

fn choose_verify_positions(word_count: usize) -> Vec<usize> {
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0xC0FFEE);
    let mut positions = Vec::new();
    while positions.len() < 3 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let position = (seed as usize) % word_count;
        if !positions.contains(&position) {
            positions.push(position);
        }
    }
    positions.sort_unstable();
    positions
}
