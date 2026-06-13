use crate::ui::{
    ai::{
        chat::{AiChatEvent, AiChatPanel, OpenAiProviders, THREADS_SIDEBAR_WIDTH, ToggleAiChat},
        providers::{
            AiProvidersEvent, AiProvidersPage, CloseAiProviders, ToggleAiProvidersMaximized,
        },
        rules::{AiRulesEvent, AiRulesPage},
    },
    alpha::{AlphaExchangeInfoPage, AlphaTokensEvent, AlphaTokensPage},
    alpha_heatmap::{AlphaHeatmapEvent, AlphaHeatmapPage},
    alpha_ma_signal::AlphaDailyMaSignalPage,
    backtest::SpotBacktestPage,
    btcc::{
        wallet_generator::WalletGeneratorPage,
        wallet_list::{BtccWalletListEvent, BtccWalletListPage},
        wallet_manager::WalletGeneratorPage as WalletManagerPage,
    },
    calculator::CalculatorPage,
    heatmap::{MarketHeatmapEvent, MarketHeatmapPage},
    kline::KlineCandlestickPage,
    ma_signal::{DailyMaSignalEvent, DailyMaSignalPage},
    market::{MarketProductsEvent, MarketProductsPage},
    palette,
    spot::SpotPage,
    square::{SquareKeySettingsPage, SquareSendLogsPage, SquareTasksPage},
    strategy_help::StrategyHelpPage,
    task_board::TaskBoardPage,
    title_bar::{
        DesktopTitleBar, OpenAlphaDailyMaSignals, OpenAlphaExchangeInfo, OpenAlphaHeatmap,
        OpenAlphaTokens, OpenBtccWalletList, OpenCalculator, OpenDailyMaSignals,
        OpenDocumentConvert, OpenMarketHeatmap, OpenMarketProducts, OpenSpotBacktest,
        OpenSpotSymbols, OpenSquareKeySettings, OpenSquareSendLogs, OpenSquareTasks,
        OpenStrategyHelp, OpenTaskBoard, OpenWalletGenerator, OpenWalletManager,
    },
    tools::DocumentConvertPage,
};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{ActiveTheme, h_flex, scroll::ScrollableElement, v_flex};

const CHAT_PANEL_DEFAULT_RATIO: f32 = 0.34;
const CHAT_PANEL_MIN_WIDTH: f32 = 280.0;
const CHAT_PANEL_MAX_RATIO: f32 = 1.0;
const SASH_HIT_WIDTH: f32 = 1.0;

pub struct Dashboard {
    title_bar: Entity<DesktopTitleBar>,
    ai_chat_panel: Entity<AiChatPanel>,
    ai_providers_page: Option<Entity<AiProvidersPage>>,
    ai_rules_page: Option<Entity<AiRulesPage>>,
    market_products_page: Option<Entity<MarketProductsPage>>,
    market_heatmap_page: Option<Entity<MarketHeatmapPage>>,
    alpha_tokens_page: Option<Entity<AlphaTokensPage>>,
    alpha_exchange_info_page: Option<Entity<AlphaExchangeInfoPage>>,
    alpha_heatmap_page: Option<Entity<AlphaHeatmapPage>>,
    alpha_daily_ma_signal_page: Option<Entity<AlphaDailyMaSignalPage>>,
    spot_page: Option<Entity<SpotPage>>,
    spot_backtest_page: Option<Entity<SpotBacktestPage>>,
    daily_ma_signal_page: Option<Entity<DailyMaSignalPage>>,
    kline_candlestick_page: Option<Entity<KlineCandlestickPage>>,
    square_key_settings_page: Option<Entity<SquareKeySettingsPage>>,
    square_tasks_page: Option<Entity<SquareTasksPage>>,
    square_send_logs_page: Option<Entity<SquareSendLogsPage>>,
    calculator_page: Option<Entity<CalculatorPage>>,
    document_convert_page: Option<Entity<DocumentConvertPage>>,
    task_board_page: Option<Entity<TaskBoardPage>>,
    strategy_help_page: Option<Entity<StrategyHelpPage>>,
    btcc_wallet_list_page: Option<Entity<BtccWalletListPage>>,
    wallet_generator_page: Option<Entity<WalletGeneratorPage>>,
    wallet_manager_page: Option<Entity<WalletManagerPage>>,
    active_page: ActivePage,
    /// Current proportional width of the AI chat panel.
    chat_panel_ratio: f32,
    /// Whether the user is currently dragging the resize sash.
    dragging_sash: bool,
    ai_providers_maximized: bool,
    /// Window mouse x position when drag started.
    drag_origin_x: Option<Pixels>,
    /// Panel ratio when drag started.
    drag_origin_ratio: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivePage {
    MarketProducts,
    MarketHeatmap,
    AlphaTokens,
    AlphaExchangeInfo,
    AlphaHeatmap,
    AlphaDailyMaSignal,
    Spot,
    SpotBacktest,
    DailyMaSignal,
    KlineCandlestick,
    SquareKeySettings,
    SquareTasks,
    SquareSendLogs,
    Calculator,
    DocumentConvert,
    TaskBoard,
    StrategyHelp,
    BtccWalletList,
    WalletGenerator,
    WalletManager,
}

impl Dashboard {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title_bar = cx.new(|cx| DesktopTitleBar::new(window, cx));
        let ai_chat_panel = title_bar.read(cx).ai_chat_panel().clone();
        let market_products_page = cx.new(|cx| MarketProductsPage::new(window, cx));
        let _subscriptions = vec![
            cx.observe_in(&ai_chat_panel, window, |_, _, _, cx| {
                cx.notify();
            }),
            cx.subscribe_in(&ai_chat_panel, window, Self::on_ai_chat_event),
            cx.subscribe_in(
                &market_products_page,
                window,
                Self::on_market_products_event,
            ),
        ];

        Self {
            title_bar,
            ai_chat_panel,
            ai_providers_page: None,
            ai_rules_page: None,
            market_products_page: Some(market_products_page),
            market_heatmap_page: None,
            alpha_tokens_page: None,
            alpha_exchange_info_page: None,
            alpha_heatmap_page: None,
            alpha_daily_ma_signal_page: None,
            spot_page: None,
            spot_backtest_page: None,
            daily_ma_signal_page: None,
            kline_candlestick_page: None,
            square_key_settings_page: None,
            square_tasks_page: None,
            square_send_logs_page: None,
            calculator_page: None,
            document_convert_page: None,
            task_board_page: None,
            strategy_help_page: None,
            btcc_wallet_list_page: None,
            wallet_generator_page: None,
            wallet_manager_page: None,
            active_page: ActivePage::MarketProducts,
            chat_panel_ratio: CHAT_PANEL_DEFAULT_RATIO,
            dragging_sash: false,
            ai_providers_maximized: false,
            drag_origin_x: None,
            drag_origin_ratio: None,
            _subscriptions,
        }
    }

    fn on_open_spot_symbols(
        &mut self,
        _: &OpenSpotSymbols,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_spot_page(window, cx);
    }

    fn on_open_market_products(
        &mut self,
        _: &OpenMarketProducts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_market_products_page(window, cx);
    }

    fn on_open_market_heatmap(
        &mut self,
        _: &OpenMarketHeatmap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_market_heatmap_page(window, cx);
    }

    fn on_open_spot_backtest(
        &mut self,
        _: &OpenSpotBacktest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_spot_backtest_page(window, cx);
    }

    fn on_open_alpha_tokens(
        &mut self,
        _: &OpenAlphaTokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_alpha_tokens_page(window, cx);
    }

    fn on_open_alpha_exchange_info(
        &mut self,
        _: &OpenAlphaExchangeInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_alpha_exchange_info_page(window, cx);
    }

    fn on_open_alpha_daily_ma_signals(
        &mut self,
        _: &OpenAlphaDailyMaSignals,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_alpha_daily_ma_signal_page(window, cx);
    }

    fn on_open_alpha_heatmap(
        &mut self,
        _: &OpenAlphaHeatmap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_alpha_heatmap_page(window, cx);
    }

    fn on_alpha_tokens_event(
        &mut self,
        _: &Entity<AlphaTokensPage>,
        event: &AlphaTokensEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AlphaTokensEvent::OpenKline(symbol) => {
                self.open_alpha_kline_candlestick_page(symbol.clone(), window, cx);
            }
        }
    }

    fn on_alpha_heatmap_event(
        &mut self,
        _: &Entity<AlphaHeatmapPage>,
        event: &AlphaHeatmapEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AlphaHeatmapEvent::OpenKline(symbol) => {
                self.open_alpha_kline_candlestick_page(symbol.clone(), window, cx);
            }
        }
    }

    fn on_toggle_ai_chat(&mut self, _: &ToggleAiChat, _: &mut Window, cx: &mut Context<Self>) {
        self.ai_chat_panel.update(cx, |panel, cx| panel.toggle(cx));
        cx.notify();
    }

    fn on_ai_chat_event(
        &mut self,
        _: &Entity<AiChatPanel>,
        event: &AiChatEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AiChatEvent::OpenProviders => self.open_ai_providers_page(window, cx),
            AiChatEvent::OpenRules => self.open_ai_rules_page(window, cx),
        }
    }

    fn on_market_products_event(
        &mut self,
        _: &Entity<MarketProductsPage>,
        event: &MarketProductsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MarketProductsEvent::AnalyzeWithAi {
                prompt,
                display_content,
                rule_context,
            } => {
                self.ai_providers_page = None;
                self.ai_rules_page = None;
                self.ai_providers_maximized = false;
                self.ai_chat_panel.update(cx, |panel, cx| {
                    panel.submit_external_prompt(
                        prompt.clone(),
                        display_content.clone(),
                        Some((rule_context.key.clone(), rule_context.label.clone())),
                        cx,
                    );
                });
                cx.notify();
            }
            MarketProductsEvent::OpenKline(symbol) => {
                self.open_kline_candlestick_page(symbol.clone(), window, cx);
            }
        }
    }

    fn on_market_heatmap_event(
        &mut self,
        _: &Entity<MarketHeatmapPage>,
        event: &MarketHeatmapEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MarketHeatmapEvent::OpenKline(symbol) => {
                self.open_kline_candlestick_page(symbol.clone(), window, cx);
            }
        }
    }

    fn on_ai_providers_event(
        &mut self,
        _: &Entity<AiProvidersPage>,
        event: &AiProvidersEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AiProvidersEvent::Close => self.close_ai_providers(cx),
            AiProvidersEvent::Saved => {
                self.ai_chat_panel.update(cx, |panel, cx| {
                    panel.reload_ai_settings(cx);
                });
                cx.notify();
            }
            AiProvidersEvent::ToggleMaximized => self.toggle_ai_providers_maximized(cx),
        }
    }

    fn on_ai_rules_event(
        &mut self,
        _: &Entity<AiRulesPage>,
        event: &AiRulesEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AiRulesEvent::Close => self.close_ai_rules(cx),
        }
    }

    fn on_open_ai_providers(
        &mut self,
        _: &OpenAiProviders,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_ai_providers_page(window, cx);
    }

    fn on_close_ai_providers(
        &mut self,
        _: &CloseAiProviders,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_ai_providers(cx);
    }

    fn on_toggle_ai_providers_maximized(
        &mut self,
        _: &ToggleAiProvidersMaximized,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_ai_providers_maximized(cx);
    }

    fn close_ai_providers(&mut self, cx: &mut Context<Self>) {
        self.ai_chat_panel.update(cx, |panel, cx| {
            panel.reload_ai_settings(cx);
        });
        self.ai_providers_page = None;
        self.ai_providers_maximized = false;
        cx.notify();
    }

    fn close_ai_rules(&mut self, cx: &mut Context<Self>) {
        self.ai_rules_page = None;
        cx.notify();
    }

    fn toggle_ai_providers_maximized(&mut self, cx: &mut Context<Self>) {
        self.ai_providers_maximized = !self.ai_providers_maximized;
        if let Some(page) = &self.ai_providers_page {
            page.update(cx, |page, cx| {
                page.set_maximized(self.ai_providers_maximized, cx);
            });
        }
        cx.notify();
    }

    fn on_open_daily_ma_signals(
        &mut self,
        _: &OpenDailyMaSignals,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_daily_ma_signal_page(window, cx);
    }

    fn on_daily_ma_signal_event(
        &mut self,
        _: &Entity<DailyMaSignalPage>,
        event: &DailyMaSignalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DailyMaSignalEvent::OpenKline(symbol) => {
                self.open_kline_candlestick_page(symbol.clone(), window, cx);
            }
        }
    }

    fn on_open_square_key_settings(
        &mut self,
        _: &OpenSquareKeySettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_square_key_settings_page(window, cx);
    }

    fn on_open_square_tasks(
        &mut self,
        _: &OpenSquareTasks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_square_tasks_page(window, cx);
    }

    fn on_open_square_send_logs(
        &mut self,
        _: &OpenSquareSendLogs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_square_send_logs_page(window, cx);
    }

    fn on_open_document_convert(
        &mut self,
        _: &OpenDocumentConvert,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_document_convert_page(window, cx);
    }

    fn on_open_calculator(
        &mut self,
        _: &OpenCalculator,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_calculator_page(window, cx);
    }

    fn on_open_task_board(
        &mut self,
        _: &OpenTaskBoard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_task_board_page(window, cx);
    }

    fn on_open_strategy_help(
        &mut self,
        _: &OpenStrategyHelp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_strategy_help_page(window, cx);
    }

    fn on_open_btcc_wallet_list(
        &mut self,
        _: &OpenBtccWalletList,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_btcc_wallet_list_page(window, cx);
    }

    fn on_btcc_wallet_list_event(
        &mut self,
        _: &Entity<BtccWalletListPage>,
        event: &BtccWalletListEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            BtccWalletListEvent::OpenTransfer(address) => {
                self.open_wallet_manager_page(window, cx);
                if let Some(page) = &self.wallet_manager_page {
                    page.update(cx, |page, cx| {
                        page.set_transfer_address(address.clone(), cx)
                    });
                }
            }
        }
    }

    fn on_open_wallet_generator(
        &mut self,
        _: &OpenWalletGenerator,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_wallet_generator_page(window, cx);
    }

    fn on_open_wallet_manager(
        &mut self,
        _: &OpenWalletManager,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_wallet_manager_page(window, cx);
    }

    fn open_spot_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.spot_page.is_none() {
            self.spot_page = Some(cx.new(|cx| SpotPage::new(window, cx)));
        }
        self.active_page = ActivePage::Spot;
        cx.notify();
    }

    fn open_spot_backtest_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.spot_backtest_page.is_none() {
            self.spot_backtest_page = Some(cx.new(|cx| SpotBacktestPage::new(window, cx)));
        }
        self.active_page = ActivePage::SpotBacktest;
        cx.notify();
    }

    fn open_market_products_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.market_products_page.is_none() {
            let page = cx.new(|cx| MarketProductsPage::new(window, cx));
            self._subscriptions.push(cx.subscribe_in(
                &page,
                window,
                Self::on_market_products_event,
            ));
            self.market_products_page = Some(page);
        }
        self.active_page = ActivePage::MarketProducts;
        cx.notify();
    }

    fn open_market_heatmap_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.market_heatmap_page.is_none() {
            let page = cx.new(|cx| MarketHeatmapPage::new(window, cx));
            self._subscriptions
                .push(cx.subscribe_in(&page, window, Self::on_market_heatmap_event));
            self.market_heatmap_page = Some(page);
        }
        self.active_page = ActivePage::MarketHeatmap;
        cx.notify();
    }

    fn open_alpha_tokens_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.alpha_tokens_page.is_none() {
            let page = cx.new(|cx| AlphaTokensPage::new(window, cx));
            self._subscriptions
                .push(cx.subscribe_in(&page, window, Self::on_alpha_tokens_event));
            self.alpha_tokens_page = Some(page);
        }
        self.active_page = ActivePage::AlphaTokens;
        cx.notify();
    }

    fn open_alpha_exchange_info_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.alpha_exchange_info_page.is_none() {
            self.alpha_exchange_info_page =
                Some(cx.new(|cx| AlphaExchangeInfoPage::new(window, cx)));
        }
        self.active_page = ActivePage::AlphaExchangeInfo;
        cx.notify();
    }

    fn open_alpha_heatmap_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.alpha_heatmap_page.is_none() {
            let page = cx.new(|cx| AlphaHeatmapPage::new(window, cx));
            self._subscriptions
                .push(cx.subscribe_in(&page, window, Self::on_alpha_heatmap_event));
            self.alpha_heatmap_page = Some(page);
        }
        self.active_page = ActivePage::AlphaHeatmap;
        cx.notify();
    }

    fn open_alpha_daily_ma_signal_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.alpha_daily_ma_signal_page.is_none() {
            self.alpha_daily_ma_signal_page =
                Some(cx.new(|cx| AlphaDailyMaSignalPage::new(window, cx)));
        }
        self.active_page = ActivePage::AlphaDailyMaSignal;
        cx.notify();
    }

    fn open_daily_ma_signal_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.daily_ma_signal_page.is_none() {
            let page = cx.new(|cx| DailyMaSignalPage::new(window, cx));
            self._subscriptions.push(cx.subscribe_in(
                &page,
                window,
                Self::on_daily_ma_signal_event,
            ));
            self.daily_ma_signal_page = Some(page);
        }
        self.active_page = ActivePage::DailyMaSignal;
        cx.notify();
    }

    fn open_kline_candlestick_page(
        &mut self,
        symbol: String,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(page) = &self.kline_candlestick_page {
            page.update(cx, |page, cx| page.set_symbol(symbol, cx));
        } else {
            self.kline_candlestick_page = Some(cx.new(|cx| KlineCandlestickPage::new(symbol, cx)));
        }
        self.active_page = ActivePage::KlineCandlestick;
        cx.notify();
    }

    fn open_alpha_kline_candlestick_page(
        &mut self,
        symbol: String,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(page) = &self.kline_candlestick_page {
            page.update(cx, |page, cx| page.set_alpha_symbol(symbol, cx));
        } else {
            self.kline_candlestick_page =
                Some(cx.new(|cx| KlineCandlestickPage::new_alpha(symbol, cx)));
        }
        self.active_page = ActivePage::KlineCandlestick;
        cx.notify();
    }

    fn open_square_key_settings_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.square_key_settings_page.is_none() {
            self.square_key_settings_page =
                Some(cx.new(|cx| SquareKeySettingsPage::new(window, cx)));
        }
        self.active_page = ActivePage::SquareKeySettings;
        cx.notify();
    }

    fn open_square_tasks_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.square_tasks_page.is_none() {
            self.square_tasks_page = Some(cx.new(|cx| SquareTasksPage::new(window, cx)));
        } else if let Some(page) = &self.square_tasks_page {
            page.update(cx, |page, cx| page.reload(cx));
        }
        self.active_page = ActivePage::SquareTasks;
        cx.notify();
    }

    fn open_square_send_logs_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.square_send_logs_page.is_none() {
            self.square_send_logs_page = Some(cx.new(|cx| SquareSendLogsPage::new(window, cx)));
        }
        self.active_page = ActivePage::SquareSendLogs;
        cx.notify();
    }

    fn open_document_convert_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.document_convert_page.is_none() {
            self.document_convert_page = Some(cx.new(|cx| DocumentConvertPage::new(window, cx)));
        }
        self.active_page = ActivePage::DocumentConvert;
        cx.notify();
    }

    fn open_calculator_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.calculator_page.is_none() {
            self.calculator_page = Some(cx.new(|cx| CalculatorPage::new(window, cx)));
        }
        self.active_page = ActivePage::Calculator;
        cx.notify();
    }

    fn open_task_board_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.task_board_page.is_none() {
            self.task_board_page = Some(cx.new(|cx| TaskBoardPage::new(window, cx)));
        }
        self.active_page = ActivePage::TaskBoard;
        cx.notify();
    }

    fn open_strategy_help_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.strategy_help_page.is_none() {
            self.strategy_help_page = Some(cx.new(|cx| StrategyHelpPage::new(window, cx)));
        }
        self.active_page = ActivePage::StrategyHelp;
        cx.notify();
    }

    fn open_btcc_wallet_list_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.btcc_wallet_list_page.is_none() {
            let page = cx.new(|cx| BtccWalletListPage::new(window, cx));
            self._subscriptions.push(cx.subscribe_in(
                &page,
                window,
                Self::on_btcc_wallet_list_event,
            ));
            self.btcc_wallet_list_page = Some(page);
        }
        self.active_page = ActivePage::BtccWalletList;
        cx.notify();
    }

    fn open_wallet_generator_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wallet_generator_page.is_none() {
            self.wallet_generator_page = Some(cx.new(|cx| WalletGeneratorPage::new(window, cx)));
        }
        self.active_page = ActivePage::WalletGenerator;
        cx.notify();
    }

    fn open_wallet_manager_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wallet_manager_page.is_none() {
            self.wallet_manager_page = Some(cx.new(|cx| WalletManagerPage::new(window, cx)));
        }
        self.active_page = ActivePage::WalletManager;
        cx.notify();
    }

    fn open_ai_providers_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_providers_page.is_none() {
            let page = cx.new(|cx| AiProvidersPage::new(window, cx));
            self._subscriptions
                .push(cx.subscribe_in(&page, window, Self::on_ai_providers_event));
            self.ai_providers_page = Some(page);
        }
        if let Some(page) = &self.ai_providers_page {
            page.update(cx, |page, cx| {
                page.set_maximized(self.ai_providers_maximized, cx);
            });
        }
        cx.notify();
    }

    fn open_ai_rules_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_rules_page.is_none() {
            let page = cx.new(|cx| AiRulesPage::new(window, cx));
            self._subscriptions
                .push(cx.subscribe_in(&page, window, Self::on_ai_rules_event));
            self.ai_rules_page = Some(page);
        }
        cx.notify();
    }

    // ── Sash (resize handle) ────────────────────────────────────

    fn begin_resize(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dragging_sash = true;
        self.drag_origin_x = Some(window.mouse_position().x);
        self.drag_origin_ratio = Some(self.chat_panel_ratio);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_resize_drag(
        &mut self,
        _event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(origin_x), Some(origin_ratio)) = (self.drag_origin_x, self.drag_origin_ratio)
        else {
            return;
        };

        let viewport_width = window.viewport_size().width;
        if viewport_width <= px(0.) {
            return;
        }

        let origin_width = self.panel_width_for_viewport(viewport_width, origin_ratio);
        // Sash is on the left edge of the docked panel. Moving right shrinks it.
        let delta = window.mouse_position().x - origin_x;
        let new_width = self.clamp_panel_width(viewport_width, origin_width - delta);
        self.chat_panel_ratio = (new_width / viewport_width).clamp(0.18, CHAT_PANEL_MAX_RATIO);
        cx.stop_propagation();
        cx.notify();
    }

    fn end_resize(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.dragging_sash = false;
        self.drag_origin_x = None;
        self.drag_origin_ratio = None;
        cx.stop_propagation();
        cx.notify();
    }

    fn clamp_panel_width(&self, viewport_width: Pixels, width: Pixels) -> Pixels {
        let max_width = viewport_width * CHAT_PANEL_MAX_RATIO;
        width.max(px(CHAT_PANEL_MIN_WIDTH)).min(max_width)
    }

    fn panel_width_for_viewport(&self, viewport_width: Pixels, ratio: f32) -> Pixels {
        self.clamp_panel_width(viewport_width, viewport_width * ratio)
    }

    fn dock_width_for_viewport(&self, viewport_width: Pixels, cx: &App) -> Pixels {
        if self.ai_providers_page.is_none() && self.ai_chat_panel.read(cx).threads_sidebar_visible()
        {
            let sidebar_width = px(THREADS_SIDEBAR_WIDTH);
            let viewport_without_sidebar = (viewport_width - sidebar_width).max(px(0.));
            let base_width =
                self.panel_width_for_viewport(viewport_without_sidebar, self.chat_panel_ratio);
            self.clamp_panel_width(viewport_width, base_width + px(THREADS_SIDEBAR_WIDTH))
        } else {
            self.panel_width_for_viewport(viewport_width, self.chat_panel_ratio)
        }
    }

    fn render_content(&self, _cx: &mut Context<Self>) -> AnyElement {
        match self.active_page {
            ActivePage::MarketProducts => self
                .market_products_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::MarketHeatmap => self
                .market_heatmap_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::Spot => self
                .spot_page
                .as_ref()
                .map(|spot_page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(spot_page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::SpotBacktest => self
                .spot_backtest_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::AlphaTokens => self
                .alpha_tokens_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::AlphaExchangeInfo => self
                .alpha_exchange_info_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::AlphaHeatmap => self
                .alpha_heatmap_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::AlphaDailyMaSignal => self
                .alpha_daily_ma_signal_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::DailyMaSignal => self
                .daily_ma_signal_page
                .as_ref()
                .map(|daily_ma_signal_page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(daily_ma_signal_page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::KlineCandlestick => self
                .kline_candlestick_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::SquareKeySettings => self
                .square_key_settings_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::SquareTasks => self
                .square_tasks_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::SquareSendLogs => self
                .square_send_logs_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::Calculator => self
                .calculator_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::DocumentConvert => self
                .document_convert_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::TaskBoard => self
                .task_board_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::StrategyHelp => self
                .strategy_help_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::BtccWalletList => self
                .btcc_wallet_list_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::WalletGenerator => self
                .wallet_generator_page
                .as_ref()
                .map(|page| {
                    div()
                        .flex_1()
                        .size_full()
                        .p_6()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            ActivePage::WalletManager => self
                .wallet_manager_page
                .as_ref()
                .map(|page| {
                    div()
                        .size_full()
                        .overflow_y_scrollbar()
                        .child(page.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
        }
    }

    /// Renders the draggable sash between content and chat panel.
    fn render_sash(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme();

        div()
            .w(px(SASH_HIT_WIDTH))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_col_resize()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_resize))
            .child(div().w(px(1.)).h_full().bg(palette::border(app_theme)))
    }
}

impl Render for Dashboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme();
        let chat_visible =
            self.ai_chat_panel.read(cx).is_visible() || self.ai_providers_page.is_some();
        let bg = app_theme.background;
        let fg = palette::text(app_theme);
        let panel_width = self.dock_width_for_viewport(window.viewport_size().width, cx);

        v_flex()
            .size_full()
            .relative()
            .text_size(px(12.))
            .bg(bg)
            .text_color(fg)
            .on_action(cx.listener(Self::on_open_spot_symbols))
            .on_action(cx.listener(Self::on_open_market_products))
            .on_action(cx.listener(Self::on_open_market_heatmap))
            .on_action(cx.listener(Self::on_open_spot_backtest))
            .on_action(cx.listener(Self::on_open_alpha_tokens))
            .on_action(cx.listener(Self::on_open_alpha_exchange_info))
            .on_action(cx.listener(Self::on_open_alpha_heatmap))
            .on_action(cx.listener(Self::on_open_alpha_daily_ma_signals))
            .on_action(cx.listener(Self::on_toggle_ai_chat))
            .on_action(cx.listener(Self::on_open_ai_providers))
            .on_action(cx.listener(Self::on_close_ai_providers))
            .on_action(cx.listener(Self::on_toggle_ai_providers_maximized))
            .on_action(cx.listener(Self::on_open_daily_ma_signals))
            .on_action(cx.listener(Self::on_open_square_key_settings))
            .on_action(cx.listener(Self::on_open_square_tasks))
            .on_action(cx.listener(Self::on_open_square_send_logs))
            .on_action(cx.listener(Self::on_open_calculator))
            .on_action(cx.listener(Self::on_open_document_convert))
            .on_action(cx.listener(Self::on_open_task_board))
            .on_action(cx.listener(Self::on_open_strategy_help))
            .on_action(cx.listener(Self::on_open_btcc_wallet_list))
            .on_action(cx.listener(Self::on_open_wallet_generator))
            .on_action(cx.listener(Self::on_open_wallet_manager))
            .when(self.dragging_sash, |parent| {
                parent
                    .cursor_col_resize()
                    .on_mouse_move(cx.listener(Self::on_resize_drag))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, window, cx| {
                            this.end_resize(window, cx);
                        }),
                    )
            })
            // ── Title bar ──
            .child(self.title_bar.clone())
            .child(
                h_flex()
                    .flex_1()
                    .overflow_hidden()
                    .when(
                        self.ai_providers_page.is_some() && self.ai_providers_maximized,
                        |parent| {
                            parent.child(
                                div().flex_1().h_full().bg(bg).child(
                                    self.ai_providers_page
                                        .as_ref()
                                        .map(|page| page.clone().into_any_element())
                                        .unwrap_or_else(|| div().into_any_element()),
                                ),
                            )
                        },
                    )
                    .when(
                        !(self.ai_providers_page.is_some() && self.ai_providers_maximized),
                        |parent| {
                            parent
                                .child(
                                    div()
                                        .flex_1()
                                        .h_full()
                                        .overflow_hidden()
                                        .child(self.render_content(cx)),
                                )
                                .when(chat_visible, |parent| {
                                    parent.child(self.render_sash(cx)).child(
                                        div()
                                            .w(panel_width)
                                            .min_w(px(CHAT_PANEL_MIN_WIDTH))
                                            .h_full()
                                            .bg(bg)
                                            .child(
                                                self.ai_providers_page
                                                    .as_ref()
                                                    .map(|page| page.clone().into_any_element())
                                                    .unwrap_or_else(|| {
                                                        self.ai_chat_panel
                                                            .clone()
                                                            .into_any_element()
                                                    }),
                                            ),
                                    )
                                })
                        },
                    ),
            )
            .when_some(self.ai_rules_page.clone(), |parent, page| {
                parent.child(page)
            })
    }
}
