use anyhow::{Result, ensure};
use image::imageops::crop_imm;

/// 目标帖子互动栏的局部截图。
pub(super) struct PostActionCrop {
    pub image: image::RgbImage,
    /// 裁剪区域顶部在原图中的 Y 坐标。
    pub top: u32,
}

/// 根据目标帖子的头部和下一条帖子的头部，裁剪当前帖子区域。
///
/// 保留完整图片宽度，是因为 timeline.rs 使用横向位置判断回复、
/// 转发、点赞数量的位置；不能随意缩窄图片。
pub(super) fn crop_post_action_region(
    source: &image::RgbImage,
    header_y: u32,
    next_header_y: u32,
) -> Result<PostActionCrop> {
    ensure!(source.width() > 0 && source.height() > 0, "原图尺寸无效");

    let bottom = next_header_y.min(source.height());

    ensure!(header_y < bottom, "目标帖子边界异常，无法裁剪互动栏");

    // 留出顶部空间，让局部 OCR 仍能识别目标帖子的作者栏。
    // timeline.rs 需要作者栏来关联后面的互动数量。
    let post_height = bottom - header_y;
    let padding = (post_height * 15 / 100).max(32).min(header_y);
    let top = header_y - padding;

    let crop = crop_imm(source, 0, top, source.width(), bottom - top).to_image();

    ensure!(!crop.is_empty(), "互动栏裁剪结果为空");

    Ok(PostActionCrop { image: crop, top })
}

/// 将局部截图中的 Y 坐标还原为原图坐标。
pub(super) fn restore_y(local_y: u32, crop_top: u32) -> Result<u32> {
    crop_top
        .checked_add(local_y)
        .ok_or_else(|| anyhow::anyhow!("互动栏 Y 坐标换算溢出"))
}
