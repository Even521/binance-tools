use gpui::Hsla;
use gpui_component::Theme;

pub fn text(theme: &Theme) -> Hsla {
    theme.foreground.opacity(0.96)
}

pub fn text_strong(theme: &Theme) -> Hsla {
    theme.foreground
}

pub fn muted(theme: &Theme) -> Hsla {
    theme.foreground.opacity(0.74)
}

pub fn muted_soft(theme: &Theme) -> Hsla {
    theme.foreground.opacity(0.62)
}

pub fn border(theme: &Theme) -> Hsla {
    theme.border.opacity(0.62)
}

pub fn border_soft(theme: &Theme) -> Hsla {
    theme.border.opacity(0.42)
}

pub fn surface(theme: &Theme) -> Hsla {
    theme.group_box.opacity(0.24)
}

pub fn surface_strong(theme: &Theme) -> Hsla {
    theme.group_box.opacity(0.38)
}

pub fn hover(theme: &Theme) -> Hsla {
    theme.muted.opacity(0.18)
}

pub fn error_background() -> Hsla {
    gpui::hsla(0.0, 0.72, 0.94, 1.0)
}

pub fn error_border() -> Hsla {
    gpui::hsla(0.0, 0.62, 0.76, 1.0)
}

pub fn error_text() -> Hsla {
    gpui::hsla(0.0, 0.18, 0.18, 1.0)
}
