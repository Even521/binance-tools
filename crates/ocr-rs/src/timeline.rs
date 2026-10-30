use ppocr_rs::TextBlock;
use serde::Serialize;
use std::io::Write;

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
    header_left: u32,
    media_start_y: Option<u32>,
    text: Vec<LocatedText>,
    metric_candidates: Vec<LocatedText>,
}

fn ocr_debug(message: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(r"D:\rust\binance-tools\ocr-debug.log")
    {
        let _ = writeln!(file, "{message}");
    }
}

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
    let mut left_navigation = Vec::new();
    let mut timeline = TimelineRegion::default();
    let mut right_sidebar = RightSidebar::default();

    let mut post_builders: Vec<PostBuilder> = Vec::new();
    let mut current_post: Option<PostBuilder> = None;
    let mut pending_ad_label = false;

    // 引用作者栏出现后，跳过中央时间线的引用内容。
    // 下一条顶层作者栏出现时恢复。
    let mut skip_nested_content = false;

    // 是否已经识别到帖子详情弹窗中的引用作者
    let mut nested_quote_seen = false;

    // 是否忽略弹窗后方的时间线帖子
    let mut ignore_following_posts = false;

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

        // 忽略弹窗后方中央时间线的内容，但不影响浏览器顶部和右侧栏解析。
        if ignore_following_posts && x >= width * 28 / 100 && x < width * 64 / 100 {
            continue;
        }

        // 1. 浏览器顶部
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

        // 2. 识别作者栏
        let parsed_header = parse_post_header(text);

        if text.contains('@') {
            ocr_debug(&format!(
                "[OCR-HEADER] y={}, x={}, text={:?}, parsed={:?}",
                y, x, text, parsed_header
            ));
        }

        ocr_debug(&format!("[OCR-BLOCK] y={}, x={}, text={:?}", y, x, text));

        if let Some((name, account, timestamp, is_ad)) = parsed_header {
            let header_left = x;
            let indent_threshold = (width * 4 / 100).max(40);

            let is_nested_header = current_post.as_ref().is_some_and(|post| {
                let is_indented = header_left > post.header_left.saturating_add(indent_threshold);
                let is_below_current_header = y > post.header_y;

                is_indented && is_below_current_header
            });

            ocr_debug(&format!(
                "[OCR-NESTED-CHECK] name={:?}, account={:?}, y={}, x={}, nested={}",
                name, account, y, header_left, is_nested_header
            ));

            if is_nested_header {
                nested_quote_seen = true;
                // 引用卡片中的作者也需要作为评论目标。
                // 先保存外层帖子，再为引用作者创建独立帖子。
                if let Some(post) = current_post.take() {
                    post_builders.push(post);
                }

                current_post = Some(PostBuilder {
                    author_name: Some(name),
                    account: Some(account),
                    verified: text.contains('✓').then_some(true),
                    timestamp,
                    is_ad,
                    grok: text.contains("Grok").then_some(true),
                    header_y: y,
                    header_left,
                    ..PostBuilder::default()
                });

                // 引用卡片正文不属于当前评论目标，跳过后续内容。
                skip_nested_content = true;

                ocr_debug(&format!(
                    "[OCR-QUOTE-AUTHOR-KEEP] 保留引用作者作为评论目标：text={:?}, y={}, x={}",
                    text, y, header_left
                ));

                continue;
            }

            // 新的顶层作者栏：恢复采集。
            // 已经识别到引用作者时，下一位顶层作者属于弹窗背后的时间线。
            // 保存当前引用作者，但不把背景帖子加入本次结果。
            if nested_quote_seen {
                if let Some(post) = current_post.take() {
                    ocr_debug(&format!(
                        "[OCR-POST-SAVE-QUOTE] name={:?}, account={:?}, y={}",
                        post.author_name, post.account, post.header_y
                    ));

                    post_builders.push(post);
                }

                ignore_following_posts = true;
                skip_nested_content = false;

                ocr_debug(&format!(
                    "[OCR-BACKGROUND-POST-SKIP] name={:?}, account={:?}, y={}",
                    name, account, y
                ));

                continue;
            }

            // 普通顶层帖子：恢复正文采集。
            skip_nested_content = false;

            if let Some(post) = current_post.take() {
                ocr_debug(&format!(
                    "[OCR-POST-SAVE] name={:?}, account={:?}, y={}",
                    post.author_name, post.account, post.header_y
                ));

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
                ..PostBuilder::default()
            });

            pending_ad_label = false;
            continue;
        }

        // 3. 左侧导航栏
        if x < width * 28 / 100 {
            left_navigation.push(item);
            continue;
        }

        // 4. 右侧栏
        if x >= width * 64 / 100 {
            if y < height * 19 / 100 || text.contains("搜索") {
                right_sidebar.search.push(item);
            } else {
                push_sidebar_item(&mut right_sidebar.sections, item);
            }
            continue;
        }

        // 5. 时间线顶部导航和发帖区域
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

        // 6. 引用区域跳过判断。
        //
        // 必须在广告、媒体标记和正文采集之前，
        // 避免引用中的内容修改外层帖子的状态。
        if skip_nested_content {
            ocr_debug(&format!(
                "[OCR-QUOTE-CONTENT-SKIP] y={}, x={}, text={:?}",
                y, x, text
            ));
            continue;
        }

        // 7. 广告标记
        if text.starts_with("广告") && text.chars().count() <= 8 {
            if let Some(post) = current_post.as_mut() {
                post.is_ad = true;
            } else {
                pending_ad_label = true;
            }
            continue;
        }

        // 8. 媒体区域标记
        if text.contains("显示更多") {
            if let Some(post) = current_post.as_mut() {
                post.media_start_y = Some(y.saturating_add(12));
            }
            continue;
        }

        // 9. 正文和互动数字
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
        } else if !is_ui_control(text) {
            timeline.unclassified.push(item);
        }
    }

    // 保存最后一条帖子
    if let Some(post) = current_post {
        post_builders.push(post);
    }

    // 10. 构建帖子结果
    for (index, post) in post_builders.iter().enumerate() {
        ocr_debug(&format!(
            "[OCR-POST-OUTPUT] index={}, name={:?}, account={:?}, y={}",
            index, post.author_name, post.account, post.header_y
        ));

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

        let action_rows = group_metric_rows(&post.metric_candidates, (height * 12 / 1032).max(8));

        let selected_row = action_rows
            .iter()
            .filter(|row| {
                if row.len() < 3 {
                    return false;
                }

                let row_y = row.iter().map(|candidate| candidate.y).sum::<u32>() / row.len() as u32;

                row_y > post.header_y
                    && row_y < post_bottom
                    && row_y.saturating_sub(post.header_y) < height * 55 / 100
            })
            .min_by_key(|row| {
                row.iter().map(|candidate| candidate.y).sum::<u32>() / row.len() as u32
            });

        let selected_action_y = selected_row
            .map(|row| row.iter().map(|candidate| candidate.y).sum::<u32>() / row.len() as u32);

        if let Some(row) = selected_row {
            for candidate in row {
                let relative_x = candidate.x as f32 / width.max(1) as f32;

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

        let post_text = post
            .text
            .iter()
            .filter(|item| {
                let outside_media = media
                    .is_none_or(|(media_x, media_y)| !(item.x >= media_x && item.y >= media_y));

                let action_row_tolerance = (height * 12 / 1032).max(8);

                let outside_action_row = selected_action_y
                    .is_none_or(|action_y| item.y.abs_diff(action_y) > action_row_tolerance);

                outside_media && outside_action_row
            })
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        timeline.posts.push(TimelinePost {
            author_name: post.author_name.clone(),
            account: post.account.clone(),
            verified: post.verified,
            timestamp: post.timestamp.clone(),
            is_ad: post.is_ad,
            grok: post.grok,
            text: post_text,
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

fn push_sidebar_item(sections: &mut Vec<SidebarSection>, item: LocatedText) {
    if is_sidebar_heading(&item.text) || sections.is_empty() {
        sections.push(SidebarSection {
            title: Some(item.text.clone()),
            items: Vec::new(),
        });
    } else if let Some(section) = sections.last_mut() {
        section.items.push(item);
    }
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

fn group_metric_rows(candidates: &[LocatedText], tolerance: u32) -> Vec<Vec<&LocatedText>> {
    let mut sorted = candidates.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|candidate| (candidate.y, candidate.x));

    let mut rows: Vec<Vec<&LocatedText>> = Vec::new();

    for candidate in sorted {
        if let Some(row) = rows.last_mut() {
            if candidate.y.abs_diff(row[0].y) <= tolerance {
                row.push(candidate);
                continue;
            }
        }

        rows.push(vec![candidate]);
    }

    rows
}
