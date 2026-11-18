//! Reusable screenshot template matching for UI button prefilters.

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

#[derive(Clone, Copy, Debug)]
pub struct TemplateMatch {
    pub center: ScreenPoint,
    pub score: f32,
}

/// Find a visual template in a screenshot or an optional crop.
///
/// A low threshold is useful for prefilters: false positives only trigger the
/// slower OCR path, while false negatives can skip a valid post.
pub fn find_template(
    screenshot_path: &Path,
    template_path: &Path,
    threshold: f32,
    scales: &[f32],
    region: Option<SearchRegion>,
) -> Result<Option<TemplateMatch>> {
    ensure!(
        (0.0..=1.0).contains(&threshold),
        "template threshold must be between 0 and 1"
    );
    ensure!(
        !scales.is_empty(),
        "at least one template scale is required"
    );

    let screenshot = image::open(screenshot_path)
        .with_context(|| {
            format!(
                "open screenshot for template matching: {}",
                screenshot_path.display()
            )
        })?
        .into_rgba8();
    let (image, origin) = crop_search_region(&screenshot, region)?;
    let template = PreparedTemplate::from_file(template_path)
        .with_context(|| format!("load button template: {}", template_path.display()))?;
    let matcher = TemplateMatcher::new(threshold)
        .with_multi_scale(true)
        .with_scales(scales.to_vec());
    let found = matcher
        .find_prepared(&image, &template, origin)
        .context("match screenshot against button template")?;

    if found.score < threshold {
        return Ok(None);
    }

    Ok(Some(TemplateMatch {
        center: found.center,
        score: found.score,
    }))
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
        "template search region is empty"
    );
    ensure!(
        region.x.saturating_add(region.width) <= screenshot.width()
            && region.y.saturating_add(region.height) <= screenshot.height(),
        "template search region exceeds screenshot dimensions"
    );

    Ok((
        crop_imm(screenshot, region.x, region.y, region.width, region.height).to_image(),
        ScreenPoint {
            x: region.x as i32,
            y: region.y as i32,
        },
    ))
}
