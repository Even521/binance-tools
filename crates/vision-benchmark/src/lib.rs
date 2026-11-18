//! Shared template matching helpers used by the benchmark and desktop X assistant.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use image::{RgbaImage, imageops::crop_imm};
use robost_vision::{PreparedTemplate, ScreenPoint, TemplateMatcher};

#[derive(Clone, Copy, Debug)]
pub struct SearchRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct ButtonHit {
    pub name: String,
    pub center: ScreenPoint,
    pub score: f32,
}

/// Scan multiple named templates in a screenshot crop. This is only a prefilter:
/// callers must not use returned coordinates as click targets.
pub fn scan_templates(
    screenshot_path: &Path,
    templates: &[(&str, &Path)],
    threshold: f32,
    scales: &[f32],
    region: Option<SearchRegion>,
) -> Result<Vec<ButtonHit>> {
    ensure!(
        (0.0..=1.0).contains(&threshold),
        "threshold must be between 0 and 1"
    );
    ensure!(
        !scales.is_empty(),
        "at least one template scale is required"
    );
    let screenshot = image::open(screenshot_path)
        .with_context(|| format!("open screenshot: {}", screenshot_path.display()))?
        .into_rgba8();
    let (image, origin) = crop_search_region(&screenshot, region)?;
    let matcher = TemplateMatcher::new(threshold)
        .with_multi_scale(true)
        .with_scales(scales.to_vec());
    let mut hits = Vec::new();
    for (name, path) in templates {
        let template = PreparedTemplate::from_file(path)
            .with_context(|| format!("load template {name}: {}", path.display()))?;
        let found = matcher
            .find_prepared(&image, &template, origin)
            .with_context(|| format!("match template {name}"))?;
        if found.score >= threshold {
            hits.push(ButtonHit {
                name: (*name).to_string(),
                center: found.center,
                score: found.score,
            });
        }
    }
    Ok(hits)
}

fn crop_search_region(
    screenshot: &RgbaImage,
    region: Option<SearchRegion>,
) -> Result<(RgbaImage, ScreenPoint)> {
    let Some(region) = region else {
        return Ok((screenshot.clone(), ScreenPoint { x: 0, y: 0 }));
    };
    ensure!(
        region.width > 0 && region.height > 0,
        "search region is empty"
    );
    ensure!(
        region.x.saturating_add(region.width) <= screenshot.width()
            && region.y.saturating_add(region.height) <= screenshot.height(),
        "search region exceeds screenshot dimensions"
    );
    Ok((
        crop_imm(screenshot, region.x, region.y, region.width, region.height).to_image(),
        ScreenPoint {
            x: region.x as i32,
            y: region.y as i32,
        },
    ))
}
