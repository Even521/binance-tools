use evm_wallet::{
    MatchMode, SearchBackend, SearchOutcome, VanityPattern, WalletMaterial, cuda_device_name,
    cuda_unavailable_reason, is_cuda_available, search_vanity_with_backend,
};
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

enum SearchMessage {
    Progress(u64),
    Finished {
        attempts: u64,
        outcome: Result<SearchOutcome, evm_wallet::SearchError>,
    },
}

pub struct WalletGeneratorPage {
    mode: MatchMode,
    backend: SearchBackend,
    target_input: Entity<InputState>,
    running: bool,
    attempts: u64,
    status: Option<String>,
    error: Option<String>,
    result: Option<WalletMaterial>,
    secrets_visible: bool,
    discard_result: bool,
    cuda_available: bool,
    cuda_status: String,
    started_at: Option<Instant>,
    cancel: Option<Arc<AtomicBool>>,
    _poll_task: Task<()>,
}

impl WalletGeneratorPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            mode: MatchMode::Prefix,
            backend: SearchBackend::Cpu,
            target_input: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("例如：abc123")
                    .default_value("a")
            }),
            running: false,
            attempts: 0,
            status: None,
            error: None,
            result: None,
            secrets_visible: true,
            discard_result: false,
            cuda_available: is_cuda_available(),
            cuda_status: cuda_status_text(),
            started_at: None,
            cancel: None,
            _poll_task: Task::ready(()),
        }
    }

    fn set_mode(&mut self, mode: MatchMode, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.mode = mode;
        cx.notify();
    }

    fn set_backend(&mut self, backend: SearchBackend, cx: &mut Context<Self>) {
        if self.running || (backend == SearchBackend::Cuda && !self.cuda_available) {
            return;
        }
        self.backend = backend;
        cx.notify();
    }

    fn start_search(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }

        let target = self.target_input.read(cx).text().to_string();
        let pattern = match VanityPattern::new(self.mode, &target) {
            Ok(pattern) => pattern,
            Err(error) => {
                self.error = Some(error.to_string());
                self.status = None;
                cx.notify();
                return;
            }
        };

        self.result = None;
        self.discard_result = false;
        self.error = None;
        self.status = Some(format!(
            "正在使用 {} 搜索命中地址…",
            backend_label(self.backend)
        ));
        self.attempts = 0;
        self.started_at = Some(Instant::now());
        self.running = true;
        self.secrets_visible = true;

        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let attempts = Arc::new(AtomicU64::new(0));
        let worker_attempts = attempts.clone();
        let backend = self.backend;
        let (sender, receiver) = mpsc::channel();

        let worker = std::thread::Builder::new()
            .name("evm-vanity-search".to_string())
            .spawn(move || {
                let outcome = search_vanity_with_backend(
                    pattern,
                    backend,
                    || worker_cancel.load(Ordering::Relaxed),
                    |count| {
                        worker_attempts.store(count, Ordering::Relaxed);
                        let _ = sender.send(SearchMessage::Progress(count));
                    },
                );
                let final_attempts = worker_attempts.load(Ordering::Relaxed);
                let _ = sender.send(SearchMessage::Finished {
                    attempts: final_attempts,
                    outcome,
                });
            });

        if worker.is_err() {
            self.running = false;
            self.status = None;
            self.error = Some("无法启动钱包生成线程".to_string());
            cx.notify();
            return;
        }

        self.cancel = Some(cancel);
        self._poll_task = cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(120)).await;

                let live_attempts = attempts.load(Ordering::Relaxed);
                let mut messages = Vec::new();
                let mut disconnected = false;
                loop {
                    match receiver.try_recv() {
                        Ok(message) => messages.push(message),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }

                let finished = messages
                    .iter()
                    .any(|message| matches!(message, SearchMessage::Finished { .. }));
                let _ = this.update(cx, |this, cx| {
                    if this.running {
                        this.attempts = this.attempts.max(live_attempts);
                    }
                    for message in messages {
                        match message {
                            SearchMessage::Progress(count) => this.attempts = count,
                            SearchMessage::Finished { attempts, outcome } => {
                                this.attempts = attempts;
                                this.running = false;
                                this.started_at = None;
                                this.cancel = None;
                                match outcome {
                                    Ok(SearchOutcome::Found(material)) => {
                                        if this.discard_result {
                                            drop(material);
                                            this.status = Some("搜索已停止".to_string());
                                        } else {
                                            this.result = Some(material);
                                            this.status = Some("已找到匹配钱包".to_string());
                                        }
                                    }
                                    Ok(SearchOutcome::Cancelled) => {
                                        this.status = Some("搜索已停止".to_string());
                                    }
                                    Err(error) => {
                                        this.status = None;
                                        this.error = Some(error.to_string());
                                    }
                                }
                            }
                        }
                    }

                    if disconnected && this.running {
                        this.running = false;
                        this.started_at = None;
                        this.cancel = None;
                        this.status = None;
                        this.error = Some("钱包生成线程意外结束".to_string());
                    }
                    cx.notify();
                });

                if finished || disconnected {
                    break;
                }
            }
        });
    }

    pub fn stop_search(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
            self.status = Some("正在停止搜索…".to_string());
            cx.notify();
        }
    }

    pub fn clear_secrets(&mut self, cx: &mut Context<Self>) {
        self.result = None;
        self.discard_result = self.running;
        self.secrets_visible = true;
        cx.notify();
    }

    fn toggle_secrets(&mut self, cx: &mut Context<Self>) {
        self.secrets_visible = !self.secrets_visible;
        cx.notify();
    }

    fn copy_private_key(&mut self, cx: &mut Context<Self>) {
        let Some(private_key) = self
            .result
            .as_ref()
            .map(|material| material.private_key.clone())
        else {
            return;
        };
        let private_key = private_key
            .trim()
            .strip_prefix("0x")
            .unwrap_or(private_key.trim())
            .to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(private_key));
        self.status = Some("已复制私钥".to_string());
        self.error = None;
        cx.notify();
    }
}

fn cuda_status_text() -> String {
    if let Some(reason) = cuda_unavailable_reason() {
        reason.to_string()
    } else if let Some(name) = cuda_device_name() {
        format!("可用设备：{name}")
    } else {
        "CUDA 设备可用".to_string()
    }
}

fn backend_label(backend: SearchBackend) -> &'static str {
    match backend {
        SearchBackend::Cpu => "CPU",
        SearchBackend::Cuda => "GPU",
    }
}

fn progress_text(backend: SearchBackend, attempts: u64, started_at: Option<Instant>) -> String {
    let rate = started_at
        .map(|started| attempts as f64 / started.elapsed().as_secs_f64().max(0.001))
        .unwrap_or(0.0);
    format!(
        "{} 已尝试 {attempts} 个地址，{rate:.0}/秒",
        backend_label(backend)
    )
}

impl Drop for WalletGeneratorPage {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl Render for WalletGeneratorPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_theme = cx.theme();
        let mono_font_family = app_theme.mono_font_family.clone();
        let result = self.result.as_ref();

        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_xl().font_semibold().child("EVM 钱包生成"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(app_theme.muted_foreground)
                            .child(
                                "本地生成 BIP39 12 词钱包，并持续搜索符合规则的 Ethereum 地址。",
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(app_theme.border)
                    .bg(app_theme.background)
                    .p_4()
                    .child(div().text_sm().font_semibold().child("地址匹配规则"))
                    .child(
                        h_flex().gap_2().children(
                            [
                                (MatchMode::Prefix, "前缀"),
                                (MatchMode::Suffix, "后缀"),
                                (MatchMode::Contains, "包含"),
                            ]
                            .into_iter()
                            .map(|(mode, label)| {
                                let selected = self.mode == mode;
                                Button::new(match mode {
                                    MatchMode::Prefix => "mode-prefix",
                                    MatchMode::Suffix => "mode-suffix",
                                    MatchMode::Contains => "mode-contains",
                                })
                                .label(if selected {
                                    format!("✓ {label}")
                                } else {
                                    label.to_string()
                                })
                                .when(selected, |button| button.primary())
                                .when(!selected, |button| button.ghost())
                                .disabled(self.running)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.set_mode(mode, cx);
                                    },
                                ))
                            }),
                        ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().font_semibold().child("搜索设备"))
                            .child(
                                Button::new("wallet-backend-cpu")
                                    .label(if self.backend == SearchBackend::Cpu {
                                        "✓ CPU"
                                    } else {
                                        "CPU"
                                    })
                                    .when(self.backend == SearchBackend::Cpu, |button| {
                                        button.primary()
                                    })
                                    .when(self.backend != SearchBackend::Cpu, |button| {
                                        button.ghost()
                                    })
                                    .disabled(self.running)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_backend(SearchBackend::Cpu, cx)
                                    })),
                            )
                            .child(
                                Button::new("wallet-backend-cuda")
                                    .label(if self.backend == SearchBackend::Cuda {
                                        "✓ GPU"
                                    } else {
                                        "GPU"
                                    })
                                    .when(self.backend == SearchBackend::Cuda, |button| {
                                        button.primary()
                                    })
                                    .when(self.backend != SearchBackend::Cuda, |button| {
                                        button.ghost()
                                    })
                                    .disabled(self.running || !self.cuda_available)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_backend(SearchBackend::Cuda, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(app_theme.muted_foreground)
                                    .child(self.cuda_status.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(app_theme.muted_foreground)
                            .child("只匹配地址 0x 后的十六进制字符，大小写不敏感；最长 40 位。"),
                    )
                    .child(Input::new(&self.target_input).small())
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new("wallet-start")
                                    .label("开始搜索")
                                    .primary()
                                    .disabled(self.running)
                                    .on_click(cx.listener(|this, _, _, cx| this.start_search(cx))),
                            )
                            .child(
                                Button::new("wallet-stop")
                                    .label("停止")
                                    .disabled(!self.running)
                                    .on_click(cx.listener(|this, _, _, cx| this.stop_search(cx))),
                            )
                            .when(self.running, |row| {
                                row.child(
                                    div()
                                        .text_sm()
                                        .text_color(app_theme.muted_foreground)
                                        .child(progress_text(
                                            self.backend,
                                            self.attempts,
                                            self.started_at,
                                        )),
                                )
                            }),
                    )
                    .when_some(self.status.as_ref(), |panel, status| {
                        panel.child(
                            div()
                                .text_sm()
                                .text_color(app_theme.muted_foreground)
                                .child(status.clone()),
                        )
                    })
                    .when_some(self.error.as_ref(), |panel, error| {
                        panel.child(
                            div()
                                .text_sm()
                                .text_color(app_theme.danger)
                                .child(error.clone()),
                        )
                    }),
            )
            .when_some(result, |page, material| {
                let mnemonic_words =
                    material
                        .mnemonic
                        .split_whitespace()
                        .enumerate()
                        .map(|(index, word)| {
                            div()
                                .rounded_md()
                                .border_1()
                                .border_color(app_theme.border)
                                .px_3()
                                .py_2()
                                .text_sm()
                                .child(format!("{:02}  {word}", index + 1))
                        });
                let private_key = if self.secrets_visible {
                    material.private_key.clone()
                } else {
                    "••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••".to_string()
                };

                page.child(
                    v_flex()
                        .gap_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(app_theme.border)
                        .p_4()
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .child(div().text_base().font_semibold().child("命中钱包"))
                                .child(
                                    Button::new("wallet-toggle-secrets")
                                        .label(if self.secrets_visible {
                                            "隐藏助记词和私钥"
                                        } else {
                                            "显示助记词和私钥"
                                        })
                                        .ghost()
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.toggle_secrets(cx)),
                                        ),
                                ),
                        )
                        .child(div().text_sm().font_semibold().child("ETH 地址"))
                        .child(
                            div()
                                .rounded_md()
                                .bg(app_theme.background)
                                .px_3()
                                .py_2()
                                .text_sm()
                                .font_family(mono_font_family.clone())
                                .child(material.address.clone()),
                        )
                        .child(div().text_sm().font_semibold().child("助记词（12 词）"))
                        .child(h_flex().flex_wrap().gap_2().children(mnemonic_words))
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .child(div().text_sm().font_semibold().child("私钥（64 位，不带 0x）"))
                                .child(
                                    Button::new("wallet-copy-private-key")
                                        .label("复制私钥")
                                        .ghost()
                                        .small()
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.copy_private_key(cx)),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .rounded_md()
                                .bg(app_theme.background)
                                .px_3()
                                .py_2()
                                .text_sm()
                                .font_family(mono_font_family.clone())
                                .child(private_key),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(app_theme.muted_foreground)
                                .child("导入小狐狸时选择“私钥”，粘贴这 64 位字符，不要带 0x，也不要带空格。"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(app_theme.muted_foreground)
                                .child(format!("派生路径：{}", material.derivation_path)),
                        ),
                )
            })
    }
}
