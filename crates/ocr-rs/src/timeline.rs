use ppocr_rs::TextBlock;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct LocatedText {
    pub text: String,
    pub x: u32,
    pub y: u32,
    pub confidence: f32,
}

#[derive(Debug, Default, Serialize)]
pub struct BrowserRegion {
    pub tabs: Vec<LocatedText>,
    pub current_page: Vec<LocatedText>,
    pub bookmarks: Vec<LocatedText>,
}

#[derive(Debug, Default, Serialize)]
pub struct TimelineRegion {
    pub navigation: Vec<LocatedText>,
    pub composer: Vec<LocatedText>,
    pub new_posts: Vec<LocatedText>,
    pub posts: Vec<TimelinePost>,
    pub unclassified: Vec<LocatedText>,
}

#[derive(Debug, Default, Serialize)]
pub struct RightSidebar {
    pub search: Vec<LocatedText>,
    pub sections: Vec<SidebarSection>,
}

#[derive(Debug, Default, Serialize)]
pub struct SidebarSection {
    pub title: Option<String>,
    pub items: Vec<LocatedText>,
}

#[derive(Debug, Serialize)]
pub struct PostMetrics {
    pub comments: Option<String>,
    pub reposts: Option<String>,
    pub likes: Option<String>,
    pub views: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ActionPoint {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Serialize)]
pub struct PostActionCoordinates {
    pub reply: Option<ActionPoint>,
    pub repost: Option<ActionPoint>,
    pub like: Option<ActionPoint>,
    pub views: Option<ActionPoint>,
    pub bookmark: Option<ActionPoint>,
    pub share: Option<ActionPoint>,
    pub more: Option<ActionPoint>,
}

#[derive(Debug, Serialize)]
pub struct TimelinePost {
    pub author_name: Option<String>,
    pub account: Option<String>,
    pub verified: Option<bool>,
    pub timestamp: Option<String>,
    pub is_ad: bool,
    pub grok: Option<bool>,
    pub text: String,
    pub has_image: bool,
    pub has_media: bool,
    pub image_coordinates: Option<(u32, u32)>,
    pub metrics: PostMetrics,
    pub action_coordinates: PostActionCoordinates,
}

#[derive(Debug, Serialize)]
pub struct TimelineOcrResult {
    pub browser: BrowserRegion,
    pub left_navigation: Vec<LocatedText>,
    pub timeline: TimelineRegion,
    pub right_sidebar: RightSidebar,
}

#[derive(Default)]
struct PostBuilder {
    author_name: Option<String>,
    account: Option<String>,
    verified: Option<bool>,
    timestamp: Option<String>,
    is_ad: bool,
    grok: Option<bool>,
    header_y: u32,
    header_left: u32,   // 新增：作者头部文字的左边界
    inside_quote: bool, // 新增：是否处于引用卡片内部
    media_start_y: Option<u32>,
    text: Vec<LocatedText>,
    metric_candidates: Vec<LocatedText>,
}

/// Splits recognized text into the four visible regions of a desktop X screenshot.
pub fn parse_timeline(blocks: &[TextBlock], image: &image::RgbImage) -> TimelineOcrResult {
    let width = image.width();
    let height = image.height();
    let browser_bottom = height * 11 / 100;
    let mut positioned = blocks
        .iter()
        .filter_map(|block| {
            let (x, y) = block_position(block)?;
            Some((y, x, block))
        })
        .collect::<Vec<_>>();
    positioned.sort_by_key(|(y, x, _)| (*y, *x));

    let mut browser = BrowserRegion::default();
    let left_navigation = Vec::new();
    let mut timeline = TimelineRegion::default();
    let mut right_sidebar = RightSidebar::default();
    let mut post_builders = Vec::<PostBuilder>::new();
    let mut current_post: Option<PostBuilder> = None;
    let mut pending_ad_label = false;

    for (y, x, block) in positioned {
        let text = block.text.trim();
        if text.is_empty() {
            continue;
        }
        let item = LocatedText {
            text: text.to_string(),
            x,
            y,
            confidence: block.text_score,
        };

        if y <= browser_bottom {
            if y < height * 4 / 100 {
                browser.tabs.push(item);
            } else if y < height * 7 / 100 {
                browser.current_page.push(item);
            } else {
                browser.bookmarks.push(item);
            }
            continue;
        }

        // 必须先识别帖子作者栏，再判断左导航和右侧栏。
        // 否则靠近区域边界的作者栏可能被错误分类。
        if let Some((name, account, timestamp, is_ad)) = parse_post_header(text) {
            // 取 OCR 文本框的真实左边界，而不是文字中心点。
            let header_left = block
                .box_points
                .iter()
                .map(|point| point.x)
                .min()
                .unwrap_or(x);

            // 缩进阈值先设为截图宽度的 0.6%，最低 10 像素。
            // 这是用于当前截图的初步阈值，需要用多张截图验证。
            let indent_threshold = (width * 6 / 1000).max(10);

            // 判断当前作者栏是否可能属于前一条帖子的引用卡片。
            let is_nested_header = current_post.as_ref().is_some_and(|post| {
                let is_indented = header_left > post.header_left.saturating_add(indent_threshold);

                let is_below_current_header = y > post.header_y;

                is_indented && is_below_current_header
            });

            if is_nested_header {
                // 引用帖不作为新的顶层帖子。
                // 只跳过引用作者栏，不要永久忽略主帖后续的互动栏。
                continue;
            }

            // 当前作者栏被视为新的顶层帖子，先保存前一条帖子。
            if let Some(post) = current_post.take() {
                post_builders.push(post);
            }

            current_post = Some(PostBuilder {
                author_name: Some(name),
                account: Some(account),
                verified: text.contains('✓').then_some(true),
                timestamp,
                is_ad: is_ad || pending_ad_label,
                grok: text.contains("Grok").then_some(true),
                header_y: y,
                header_left,
                inside_quote: false,
                ..PostBuilder::default()
            });

            pending_ad_label = false;
            continue;
        }

        if x >= width * 64 / 100 {
            if y < height * 19 / 100 || text.contains("搜索") {
                right_sidebar.search.push(item);
            } else {
                push_sidebar_item(&mut right_sidebar.sections, item);
            }
            continue;
        }
        if y < height * 15 / 100 {
            timeline.navigation.push(item);
            continue;
        }
        if y < height * 25 / 100 {
            timeline.composer.push(item);
            continue;
        }
        if y < height * 31 / 100 {
            timeline.new_posts.push(item);
            continue;
        }
        if text.starts_with("广告") && text.chars().count() <= 8 {
            if let Some(post) = current_post.as_mut() {
                post.is_ad = true;
            } else {
                pending_ad_label = true;
            }
            continue;
        }
        if text.contains("显示更多") {
            if let Some(post) = current_post.as_mut() {
                post.media_start_y = Some(y.saturating_add(12));
            }
            continue;
        }

        if let Some(post) = current_post.as_mut() {
            if let Some(count) = extract_numeric_count(text) {
                post.metric_candidates.push(LocatedText {
                    text: count,
                    x,
                    y,
                    confidence: block.text_score,
                });
            } else if !is_ui_control(text) {
                post.text.push(item);
            }
        } else if let Some(count) = extract_numeric_count(text)
            && (0.25..=0.62).contains(&(x as f32 / width.max(1) as f32))
        {
            let mut post = PostBuilder {
                header_y: y,
                ..PostBuilder::default()
            };
            post.metric_candidates.push(LocatedText {
                text: count,
                x,
                y,
                confidence: block.text_score,
            });
            current_post = Some(post);
        } else if !is_ui_control(text) {
            timeline.unclassified.push(item);
        }
    }
    if let Some(post) = current_post {
        post_builders.push(post);
    }

    for (index, post) in post_builders.iter().enumerate() {
        let post_bottom = post_builders
            .get(index + 1)
            .map(|next| next.header_y)
            .unwrap_or(height);
        let media = has_media(image, post.header_y, post_bottom).or_else(|| {
            post.media_start_y
                .filter(|&media_y| media_y < post_bottom)
                .map(|media_y| (width * 36 / 100, media_y))
        });
        let mut metrics = PostMetrics {
            comments: None,
            reposts: None,
            likes: None,
            views: None,
        };
        let mut action_coordinates = PostActionCoordinates {
            reply: None,
            repost: None,
            like: None,
            views: None,
            bookmark: None,
            share: None,
            more: None,
        };
        // 先把互动数字分成不同的水平行。
        // 由于每条 PostBuilder 理应只属于一条顶层帖子，
        // 这里再从该帖子的候选数字中选择最完整的一行。
        let action_rows = group_metric_rows(&post.metric_candidates, (height * 12 / 1032).max(8));

        // 优先选择包含数字最多的一行。
        // 如果数量相同，保留排序后更靠上的一行。
        // 这比让后续候选数字无条件覆盖之前的数据更可靠。
        // 选择最靠下且至少包含两个数字的互动行。
        // 这样可以减少把引用卡片上方的数字混入主帖互动栏的情况。
        let selected_row = action_rows
            .iter()
            .filter(|row| row.len() >= 2)
            .max_by_key(|row| row.iter().map(|candidate| candidate.y).max().unwrap_or(0));

        let selected_action_y = selected_row
            .map(|row| row.iter().map(|candidate| candidate.y).sum::<u32>() / row.len() as u32);
        if let Some(row) = selected_row {
            for candidate in row {
                let relative_x = candidate.x as f32 / width.max(1) as f32;

                // 注意：这是基于数字位置的估算坐标，不是实际图标中心。
                let point = ActionPoint {
                    x: candidate
                        .x
                        .saturating_sub((width as f32 * 0.005).round() as u32),
                    y: candidate.y,
                };

                if relative_x < 0.40 {
                    metrics.comments = Some(candidate.text.clone());
                    action_coordinates.reply = Some(point);
                } else if relative_x < 0.45 {
                    metrics.reposts = Some(candidate.text.clone());
                    action_coordinates.repost = Some(point);
                } else if relative_x < 0.52 {
                    metrics.likes = Some(candidate.text.clone());
                    action_coordinates.like = Some(point);
                } else if relative_x < 0.62 {
                    metrics.views = Some(candidate.text.clone());
                    action_coordinates.views = Some(point);
                }
            }
        }

        if metrics.comments.is_some() && metrics.likes.is_some() && metrics.views.is_some() {
            metrics.reposts.get_or_insert_with(|| "0".to_string());
        }

        timeline.posts.push(TimelinePost {
            author_name: post.author_name.clone(),
            account: post.account.clone(),
            verified: post.verified,
            timestamp: post.timestamp.clone(),
            is_ad: post.is_ad,
            grok: post.grok,
            text: post
                .text
                .iter()
                .filter(|item| {
                    // 排除媒体区域内的文字
                    let outside_media = media
                        .is_none_or(|(media_x, media_y)| !(item.x >= media_x && item.y >= media_y));

                    // 排除与操作栏数字处于同一行的图标 OCR 结果
                    let action_row_tolerance = (height * 12 / 1032).max(8);

                    let outside_action_row = selected_action_y
                        .is_none_or(|action_y| item.y.abs_diff(action_y) > action_row_tolerance);

                    outside_media && outside_action_row
                })
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            has_image: media.is_some(),
            has_media: media.is_some(),
            image_coordinates: media,
            metrics,
            action_coordinates,
        });
    }

    TimelineOcrResult {
        browser,
        left_navigation,
        timeline,
        right_sidebar,
    }
}

fn push_sidebar_item(sections: &mut Vec<SidebarSection>, item: LocatedText) {
    let starts_section = is_sidebar_heading(&item.text)
        || sections
            .last()
            .and_then(|section| section.items.last())
            .is_some_and(|last| item.y.saturating_sub(last.y) > 90);

    if starts_section || sections.is_empty() {
        sections.push(SidebarSection {
            title: is_sidebar_heading(&item.text).then(|| item.text.clone()),
            items: Vec::new(),
        });
    }
    if let Some(section) = sections.last_mut() {
        section.items.push(item);
    }
}

fn is_sidebar_heading(text: &str) -> bool {
    [
        "热点投票",
        "实时热门",
        "趋势",
        "你可能喜欢",
        "Who to follow",
    ]
    .iter()
    .any(|heading| text.contains(heading))
}

fn has_media(image: &image::RgbImage, post_top: u32, post_bottom: u32) -> Option<(u32, u32)> {
    let left = image.width() / 4;
    let right = image.width() * 9 / 16;
    let top = post_top.saturating_add(32).min(image.height());
    let bottom = post_bottom.saturating_sub(45).min(image.height());
    if right <= left || bottom.saturating_sub(top) < 70 {
        return None;
    }

    let sampled_width = ((right - left) / 3) as usize;
    let row_threshold = (sampled_width as f32 * 0.01).ceil().max(2.0) as usize;
    let mut active_rows = 0usize;

    for y in top..bottom {
        let active_pixels = (left..right)
            .step_by(3)
            .filter(|&x| {
                let pixel = image.get_pixel(x, y).0;
                let darkest = pixel[0].min(pixel[1]).min(pixel[2]);
                let brightest = pixel[0].max(pixel[1]).max(pixel[2]);
                brightest < 250 && (brightest.saturating_sub(darkest) > 25 || darkest < 190)
            })
            .count();

        if active_pixels >= row_threshold {
            active_rows += 1;
            if active_rows >= 35 {
                return Some((image.width() * 36 / 100, y.saturating_sub(35)));
            }
        } else {
            active_rows = 0;
        }
    }
    None
}

fn block_position(block: &TextBlock) -> Option<(u32, u32)> {
    if block.box_points.is_empty() {
        return None;
    }
    let min_x = block.box_points.iter().map(|point| point.x).min()?;
    let max_x = block.box_points.iter().map(|point| point.x).max()?;
    let min_y = block.box_points.iter().map(|point| point.y).min()?;
    Some(((min_x + max_x) / 2, min_y))
}

pub(super) fn parse_post_header(text: &str) -> Option<(String, String, Option<String>, bool)> {
    let at = text.find('@')?;
    let raw_name = text[..at].trim();
    let is_ad = raw_name.starts_with("广告");
    let name = raw_name
        .strip_prefix("广告")
        .unwrap_or(raw_name)
        .trim()
        .to_string();
    let account_and_time = text[at + 1..].trim();
    let account_end = account_and_time
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_alphanumeric() && *ch != '_')
        .map(|(index, _)| index)
        .unwrap_or(account_and_time.len());
    let account = &account_and_time[..account_end];
    let time = account_and_time[account_end..]
        .trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric() && !ch.is_alphanumeric());
    if account.is_empty() || name.is_empty() || text.chars().count() > 80 {
        return None;
    }
    let timestamp = looks_like_time(time).then(|| time.to_string());
    Some((name, format!("@{account}"), timestamp, is_ad))
}

fn looks_like_time(text: &str) -> bool {
    [
        "小时",
        "分钟",
        "秒",
        "天",
        "周",
        "个月",
        "刚刚",
        "yesterday",
        "Yesterday",
    ]
    .iter()
    .any(|unit| text.contains(unit))
        || (text.len() <= 8
            && text.chars().any(|ch| ch.is_ascii_digit())
            && text.chars().any(|ch| matches!(ch, 'h' | 'm' | 'd' | 's')))
}

fn extract_numeric_count(text: &str) -> Option<String> {
    let token = text.split_whitespace().last()?;
    if token.chars().any(|ch| ch.is_ascii_digit())
        && token.chars().all(|ch| ch.is_ascii_digit() || ch == ',')
    {
        Some(token.to_string())
    } else {
        None
    }
}

fn is_ui_control(text: &str) -> bool {
    let text = text.trim();
    text.is_empty()
        || text.chars().all(|ch| ch.is_ascii_digit())
        || (text.len() <= 3
            && text.bytes().any(|byte| byte.is_ascii_digit())
            && text.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        || (text.chars().count() <= 2 && text.chars().any(|ch| ch.is_ascii_alphabetic()))
        || [
            "显示更多",
            "回复",
            "转帖",
            "喜欢",
            "分享",
            "书签",
            "更多",
            "发帖",
        ]
        .iter()
        .any(|control| text == *control)
}

/// 将互动数字按垂直位置分组。
///
/// 同一互动行的数字通常具有接近的 Y 坐标。
/// 这里允许 12 像素的垂直差异；不同截图分辨率下，
/// 后续可以将这个固定值改成基于图像高度的比例。
fn group_metric_rows(candidates: &[LocatedText], tolerance: u32) -> Vec<Vec<&LocatedText>> {
    let mut sorted = candidates.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|candidate| (candidate.y, candidate.x));

    let mut rows: Vec<Vec<&LocatedText>> = Vec::new();

    for candidate in sorted {
        if let Some(row) = rows.last_mut() {
            // 使用当前行第一个数字的 Y 坐标作为行基准，
            // 防止多个小偏差累积后把相邻两行合并。
            if candidate.y.abs_diff(row[0].y) <= tolerance {
                row.push(candidate);
                continue;
            }
        }

        rows.push(vec![candidate]);
    }

    rows
}
