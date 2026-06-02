use crate::ui::palette;
use gpui::{prelude::FluentBuilder, *};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    text::TextView,
    v_flex,
};
use std::path::{Path, PathBuf};

pub struct DocumentConvertPage {
    mode: ConvertMode,
    source_input: Entity<InputState>,
    converted: String,
    current_path: Option<PathBuf>,
    status: Option<String>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConvertMode {
    MarkdownToHtml,
    HtmlToMarkdown,
}

impl ConvertMode {
    fn label(self) -> &'static str {
        match self {
            Self::MarkdownToHtml => "MD -> HTML",
            Self::HtmlToMarkdown => "HTML -> MD",
        }
    }

    fn output_extension(self) -> &'static str {
        match self {
            Self::MarkdownToHtml => "html",
            Self::HtmlToMarkdown => "md",
        }
    }
}

impl DocumentConvertPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source_input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(18, 80)
                .placeholder("打开 Markdown / HTML 文件，或直接粘贴内容...")
                .default_value("")
        });
        let _subscriptions = vec![cx.subscribe_in(&source_input, window, Self::on_input_event)];

        Self {
            mode: ConvertMode::MarkdownToHtml,
            source_input,
            converted: String::new(),
            current_path: None,
            status: None,
            error: None,
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
            self.converted.clear();
            self.status = None;
            self.error = None;
            cx.notify();
        }
    }

    fn set_mode(&mut self, mode: ConvertMode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.converted.clear();
        self.status = None;
        self.error = None;
        cx.notify();
    }

    fn open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 Markdown 或 HTML 文件".into()),
        });
        let view = cx.entity();

        cx.spawn_in(window, async move |_, window| {
            let path = match path.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let Some(path) = path else {
                return;
            };
            let result = std::fs::read_to_string(&path).map(|content| (path, content));

            _ = window.update(|window, cx| {
                view.update(cx, |this, cx| match result {
                    Ok((path, content)) => {
                        this.mode = mode_for_path(&path);
                        this.current_path = Some(path.clone());
                        this.converted.clear();
                        this.status = Some(format!("已打开 {}", path.display()));
                        this.error = None;
                        this.source_input.update(cx, |input, cx| {
                            input.set_value(content, window, cx);
                        });
                    }
                    Err(err) => {
                        this.error = Some(format!("打开文件失败：{err}"));
                        this.status = None;
                    }
                });
            });
        })
        .detach();
    }

    fn convert(&mut self, cx: &mut Context<Self>) {
        let source = self.source_input.read(cx).text().to_string();
        if source.trim().is_empty() {
            self.error = Some("内容为空，无法转换".to_string());
            self.status = None;
            cx.notify();
            return;
        }

        self.converted = match self.mode {
            ConvertMode::MarkdownToHtml => markdown_to_html_document(&source),
            ConvertMode::HtmlToMarkdown => html_to_markdown(&source),
        };
        self.status = Some(format!("转换完成：{}", self.mode.label()));
        self.error = None;
        cx.notify();
    }

    fn save_converted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.converted.trim().is_empty() {
            self.convert(cx);
        }
        if self.converted.trim().is_empty() {
            return;
        }

        let directory = default_save_dir();
        let file_name = self.suggested_output_name();
        let content = self.converted.clone();
        let path = cx.prompt_for_new_path(&directory, Some(&file_name));
        let view = cx.entity();

        cx.spawn_in(window, async move |_, window| {
            let path = match path.await {
                Ok(Ok(Some(path))) => Some(path),
                _ => None,
            };
            let Some(path) = path else {
                return;
            };
            let result = std::fs::write(&path, content).map(|_| path);

            _ = window.update(|_, cx| {
                view.update(cx, |this, cx| match result {
                    Ok(path) => {
                        this.status = Some(format!("已保存 {}", path.display()));
                        this.error = None;
                        cx.notify();
                    }
                    Err(err) => {
                        this.error = Some(format!("保存失败：{err}"));
                        this.status = None;
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }

    fn copy_converted(&mut self, cx: &mut Context<Self>) {
        if self.converted.trim().is_empty() {
            self.convert(cx);
        }
        if !self.converted.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.converted.clone()));
            self.status = Some("已复制转换结果".to_string());
            self.error = None;
            cx.notify();
        }
    }

    fn suggested_output_name(&self) -> String {
        let stem = self
            .current_path
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("converted");
        format!("{stem}.{}", self.mode.output_extension())
    }

    fn render_mode_button(&self, mode: ConvertMode, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        let selected = self.mode == mode;

        Button::new(match mode {
            ConvertMode::MarkdownToHtml => "convert-md-html",
            ConvertMode::HtmlToMarkdown => "convert-html-md",
        })
        .outline()
        .small()
        .label(mode.label())
        .bg(if selected {
            app_theme.muted.opacity(0.36)
        } else {
            app_theme.transparent
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_mode(mode, cx);
        }))
        .into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        let message = self.error.clone().or_else(|| self.status.clone());
        let is_error = self.error.is_some();

        div()
            .min_h(px(24.))
            .when_some(message, |parent, message| {
                parent.child(
                    div()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(if is_error {
                            palette::error_border()
                        } else {
                            app_theme.success.opacity(0.35)
                        })
                        .bg(if is_error {
                            palette::error_background()
                        } else {
                            app_theme.success.opacity(0.10)
                        })
                        .px_2()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(if is_error {
                            palette::error_text()
                        } else {
                            palette::text(app_theme)
                        })
                        .child(message),
                )
            })
            .into_any_element()
    }

    fn render_preview(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let app_theme = cx.theme();
        let source = self.source_input.read(cx).text().to_string();
        let preview_markdown = match self.mode {
            ConvertMode::MarkdownToHtml => source.clone(),
            ConvertMode::HtmlToMarkdown => html_to_markdown(&source),
        };

        div()
            .size_full()
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
                            .items_center()
                            .h(px(32.))
                            .px_3()
                            .border_b_1()
                            .border_color(palette::border(app_theme))
                            .text_size(px(12.))
                            .text_color(palette::muted(app_theme))
                            .child(if self.mode == ConvertMode::MarkdownToHtml {
                                "Markdown 预览"
                            } else {
                                "HTML 转 Markdown 预览"
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_y_scrollbar()
                            .p_3()
                            .when(source.trim().is_empty(), |parent| {
                                parent.child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(palette::muted(app_theme))
                                        .child("打开文件或输入内容后，这里会实时显示预览"),
                                )
                            })
                            .when(!source.trim().is_empty(), |parent| {
                                parent.child(
                                    TextView::markdown(
                                        "document-convert-live-preview",
                                        preview_markdown,
                                        window,
                                        cx,
                                    )
                                    .selectable(true),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }
}

impl Render for DocumentConvertPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let markdown_to_html = self.render_mode_button(ConvertMode::MarkdownToHtml, cx);
        let html_to_markdown = self.render_mode_button(ConvertMode::HtmlToMarkdown, cx);
        let status = self.render_status(cx);
        let preview = self.render_preview(window, cx);
        let source_empty = self
            .source_input
            .read(cx)
            .text()
            .to_string()
            .trim()
            .is_empty();
        let app_theme = cx.theme();

        v_flex()
            .size_full()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette::border(app_theme))
                    .bg(app_theme.background)
                    .p_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_semibold()
                                    .text_color(palette::text_strong(app_theme))
                                    .child("文档转换"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette::muted(app_theme))
                                    .child("Markdown 与 HTML 本地互转，结果可复制或保存到文件。"),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("open-document-file")
                                    .outline()
                                    .small()
                                    .icon(Icon::new(IconName::FolderOpen).size_4())
                                    .label("打开文件")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_file(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("convert-document")
                                    .primary()
                                    .small()
                                    .label("转换")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.convert(cx);
                                    })),
                            )
                            .child(
                                Button::new("copy-document-result")
                                    .outline()
                                    .small()
                                    .icon(Icon::new(IconName::Copy).size_4())
                                    .label("复制")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.copy_converted(cx);
                                    })),
                            )
                            .child(
                                Button::new("save-document-result")
                                    .outline()
                                    .small()
                                    .icon(Icon::new(IconName::ArrowDown).size_4())
                                    .label("保存")
                                    .disabled(source_empty)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_converted(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(markdown_to_html)
                            .child(html_to_markdown),
                    )
                    .child(status),
            )
            .child(
                h_flex()
                    .flex_1()
                    .gap_3()
                    .overflow_hidden()
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
                                            .items_center()
                                            .h(px(32.))
                                            .px_3()
                                            .border_b_1()
                                            .border_color(palette::border(app_theme))
                                            .text_size(px(12.))
                                            .text_color(palette::muted(app_theme))
                                            .child("源内容"),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .font_family(app_theme.mono_font_family.clone())
                                            .text_size(px(12.))
                                            .child(
                                                Input::new(&self.source_input)
                                                    .h_full()
                                                    .appearance(false),
                                            ),
                                    ),
                            ),
                    )
                    .child(div().flex_1().h_full().child(preview)),
            )
    }
}

fn mode_for_path(path: &Path) -> ConvertMode {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => ConvertMode::HtmlToMarkdown,
        _ => ConvertMode::MarkdownToHtml,
    }
}

fn default_save_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|path| path.join("Downloads"))
        .filter(|path| path.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn markdown_to_html_document(markdown: &str) -> String {
    let mut body = String::new();
    let parser = pulldown_cmark::Parser::new_ext(markdown, pulldown_cmark::Options::all());
    pulldown_cmark::html::push_html(&mut body, parser);
    format!(
        "<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n  <meta charset=\"utf-8\">\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  <title>Converted Markdown</title>\n</head>\n<body>\n{body}\n</body>\n</html>\n"
    )
}

fn html_to_markdown(html: &str) -> String {
    let mut text = html.to_string();
    let replacements = [
        ("<br>", "\n"),
        ("<br/>", "\n"),
        ("<br />", "\n"),
        ("</p>", "\n\n"),
        ("</div>", "\n"),
        ("</h1>", "\n\n"),
        ("</h2>", "\n\n"),
        ("</h3>", "\n\n"),
        ("</h4>", "\n\n"),
        ("</h5>", "\n\n"),
        ("</h6>", "\n\n"),
        ("<li>", "- "),
        ("</li>", "\n"),
        ("</tr>", "\n"),
        ("</td>", " | "),
        ("</th>", " | "),
        ("&nbsp;", " "),
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
    ];

    for (from, to) in replacements {
        text = replace_ascii_case(&text, from, to);
    }
    text = strip_tags(&text);
    collapse_blank_lines(&text)
}

fn replace_ascii_case(source: &str, needle: &str, replacement: &str) -> String {
    let lower_source = source.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;

    while let Some(relative) = lower_source[cursor..].find(&lower_needle) {
        let start = cursor + relative;
        output.push_str(&source[cursor..start]);
        output.push_str(replacement);
        cursor = start + needle.len();
    }
    output.push_str(&source[cursor..]);
    output
}

fn strip_tags(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut in_tag = false;
    for ch in source.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    output
}

fn collapse_blank_lines(source: &str) -> String {
    let mut output = String::new();
    let mut blank_count = 0;

    for line in source.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                output.push('\n');
            }
        } else {
            blank_count = 0;
            output.push_str(line);
            output.push('\n');
        }
    }

    output.trim().to_string()
}
