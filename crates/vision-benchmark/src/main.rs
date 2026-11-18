use anyhow::{Context, Result, bail};
use robost_vision::{PreparedTemplate, ScreenPoint, TemplateMatcher};
use serde::Deserialize;
use std::{collections::HashMap, env, fs, path::{Path, PathBuf}, time::{Duration, Instant}};

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)] settings: Settings,
    templates: HashMap<String, String>,
    cases: Vec<TestCase>,
}
#[derive(Debug, Deserialize)]
#[serde(default)]
struct Settings {
    threshold: f32, iterations: usize, tolerance_px: f64,
    multi_scale: bool, scales: Option<Vec<f32>>, origin_x: i32, origin_y: i32,
}
impl Default for Settings {
    fn default() -> Self {
        Self { threshold: 0.87, iterations: 30, tolerance_px: 8.0,
            multi_scale: true, scales: None, origin_x: 0, origin_y: 0 }
    }
}
#[derive(Debug, Deserialize)]
struct TestCase {
    name: String, screenshot: String, template: String,
    expected_center: Option<Point>,
    #[serde(default)] origin: Option<Point>,
}
#[derive(Debug, Deserialize)]
struct Point { x: i32, y: i32 }
#[derive(Debug)]
struct CaseResult {
    name: String, found: bool, correct: bool, score: Option<f32>,
    center: Option<(i32, i32)>, expected: Option<(i32, i32)>,
    error_px: Option<f64>, timings: Vec<Duration>,
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let manifest_path = args.manifest.canonicalize()
        .with_context(|| format!("找不到测试清单：{}", args.manifest.display()))?;
    let base = manifest_path.parent().unwrap_or(Path::new("."));
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(&manifest_path).context("读取 benchmark.json 失败")?
    ).context("解析 benchmark.json 失败")?;
    if manifest.cases.is_empty() && args.scan_image.is_none() {
        bail!("测试清单 cases 为空。请使用 --scan-image <截图路径> 直接扫描截图，或添加人工标注样本。");
    }
    if manifest.settings.iterations == 0 { bail!("iterations 必须大于 0"); }

    let mut matcher = TemplateMatcher::new(manifest.settings.threshold)
        .with_multi_scale(manifest.settings.multi_scale);
    if let Some(scales) = manifest.settings.scales.clone() {
        matcher = matcher.with_scales(scales);
    }

    // 模板加载和灰度预处理不计入每次匹配耗时。
    let mut templates = HashMap::<String, PreparedTemplate>::new();
    for (name, relative) in &manifest.templates {
        let path = resolve(base, relative);
        let template = PreparedTemplate::from_file(&path)
            .with_context(|| format!("加载模板 {name} 失败：{}", path.display()))?;
        templates.insert(name.clone(), template);
    }

    // 多目标扫描：每次取当前最高分，屏蔽该按钮邻域后继续寻找下一个。
    if let Some(scan_path) = &args.scan_image {
        let scan_path = if scan_path.is_absolute() { scan_path.clone() } else { base.join(scan_path) };
        let screenshot = image::open(&scan_path)
            .with_context(|| format!("加载扫描截图失败：{}", scan_path.display()))?
            .into_rgba8();
        let mut annotated = screenshot.clone();
        let mut csv = String::from("template,index,center_x,center_y,score,elapsed_ms\\n");
        println!("\\n========== X 页面多目标按钮扫描 ==========");
        println!("截图：{} | 尺寸：{} x {}", scan_path.display(), screenshot.width(), screenshot.height());
        println!("阈值：{:.2} | 每种模板最多：{}", manifest.settings.threshold, args.max_results);
        for (name, template) in &templates {
            let relative = manifest.templates.get(name).context("模板路径缺失")?;
            let template_image = image::open(resolve(base, relative))?.into_rgba8();
            let (tw, th) = template_image.dimensions();
            let mut working = screenshot.clone();
            let mut hits = Vec::new();
            let started = Instant::now();
            for _ in 0..args.max_results {
                let found = matcher.find_prepared(&working, template,
                    ScreenPoint { x: manifest.settings.origin_x, y: manifest.settings.origin_y })
                    .with_context(|| format!("模板 {name} 多目标匹配失败"))?;
                if found.score < manifest.settings.threshold { break; }
                let cx = found.center.x;
                let cy = found.center.y;
                if hits.iter().any(|(x, y, _): &(i32, i32, f32)|
                    (x - cx).abs() < (tw as i32 / 2).max(8) &&
                    (y - cy).abs() < (th as i32 / 2).max(8)) { break; }
                hits.push((cx, cy, found.score));
                // 用邻域背景均值覆盖已命中区域，避免下一轮重复命中同一按钮。
                suppress_match(&mut working, cx, cy, tw, th);
            }
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            println!("{name}: {} 个 | 总耗时 {:.3} ms", hits.len(), elapsed);
            for (index, (cx, cy, score)) in hits.iter().enumerate() {
                println!("  #{:02} center=({}, {}) score={:.3}", index + 1, cx, cy, score);
                csv.push_str(&format!("{},{},{},{},{:.4},{:.3}\\n", name, index + 1, cx, cy, score, elapsed));
                draw_box(&mut annotated, *cx, *cy, tw, th);
            }
        }
        let output = scan_path.with_file_name(format!("{}-multi-detect.png",
            scan_path.file_stem().and_then(|s| s.to_str()).unwrap_or("scan")));
        annotated.save(&output).with_context(|| format!("保存标注图失败：{}", output.display()))?;
        let csv_path = output.with_extension("csv");
        fs::write(&csv_path, csv)?;
        println!("标注截图：{}\\nCSV：{}", output.display(), csv_path.display());
        return Ok(());
    }

    let mut results = Vec::new();
    for case in &manifest.cases {
        let template = templates.get(&case.template)
            .with_context(|| format!("样本 {} 引用了不存在的模板 {}", case.name, case.template))?;
        let screenshot_path = resolve(base, &case.screenshot);
        let screenshot = image::open(&screenshot_path)
            .with_context(|| format!("加载截图失败：{}", screenshot_path.display()))?
            .into_rgba8();
        let origin = case.origin.as_ref().map(|p| ScreenPoint { x: p.x, y: p.y })
            .unwrap_or(ScreenPoint { x: manifest.settings.origin_x, y: manifest.settings.origin_y });

        // 预热不计入耗时统计。
        let _ = matcher.find_prepared(&screenshot, template, origin)
            .with_context(|| format!("样本 {} 预热匹配失败", case.name))?;
        let mut timings = Vec::with_capacity(manifest.settings.iterations);
        let mut last_match = None;
        for _ in 0..manifest.settings.iterations {
            let start = Instant::now();
            let found = matcher.find_prepared(&screenshot, template, origin)
                .with_context(|| format!("样本 {} 匹配失败", case.name))?;
            timings.push(start.elapsed());
            last_match = Some(found);
        }

        let found = last_match;
        let expected = case.expected_center.as_ref().map(|p| (p.x, p.y));
        let center = found.as_ref().map(|m| (m.center.x, m.center.y));
        let error_px = match (expected, center) {
            (Some((ex, ey)), Some((x, y))) =>
                Some((((x - ex) as f64).powi(2) + ((y - ey) as f64).powi(2)).sqrt()),
            _ => None,
        };
        let correct = match (expected, found.as_ref(), error_px) {
            (Some(_), Some(_), Some(error)) => error <= manifest.settings.tolerance_px,
            (None, None, _) => true,
            _ => false,
        };
        results.push(CaseResult {
            name: case.name.clone(), found: found.is_some(), correct,
            score: found.as_ref().map(|m| m.score), center, expected, error_px, timings,
        });
    }
    print_report(&results, &manifest.settings, &args, base)?;
    Ok(())
}

struct Args { manifest: PathBuf, csv: Option<PathBuf>, scan_image: Option<PathBuf>, max_results: usize }
fn parse_args() -> Result<Args> {
    let mut manifest = PathBuf::from("crates/vision-benchmark/benchmark.example.json");
    let mut csv = None;
    let mut scan_image = None;\n    let mut max_results = 30usize;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => manifest = args.next().context("--manifest 缺少路径")?.into(),
            "--csv" => csv = Some(args.next().context("--csv 缺少输出路径")?.into()),
            "--scan-image" => scan_image = Some(args.next().context("--scan-image 缺少截图路径")?.into()),\n            "--max-results" => max_results = args.next().context("--max-results 缺少数量")?.parse().context("--max-results 必须是正整数")?,
            "--help" | "-h" => {
                println!("vision-benchmark [--manifest benchmark.json] [--scan-image screenshot.png] [--max-results 30] [--csv report.csv]");
                std::process::exit(0);
            }
            other => bail!("未知参数：{other}。使用 --help 查看用法。"),
        }
    }
    if max_results == 0 { bail!("--max-results 必须大于 0"); }\n    Ok(Args { manifest, csv, scan_image, max_results })
}

fn suppress_match(image: &mut image::RgbaImage, cx: i32, cy: i32, tw: u32, th: u32) {
    let rx = (tw as i32).max(12);
    let ry = (th as i32).max(12);
    let left = (cx - rx).max(0) as u32;
    let top = (cy - ry).max(0) as u32;
    let right = (cx + rx).min(image.width() as i32 - 1) as u32;
    let bottom = (cy + ry).min(image.height() as i32 - 1) as u32;
    let mut sum = [0u64; 3]; let mut count = 0u64;
    for x in left..=right {
        for y in [top.saturating_sub(1), (bottom + 1).min(image.height()-1)] {
            let p = image.get_pixel(x, y); for c in 0..3 { sum[c] += p[c] as u64; } count += 1;
        }
    }
    for y in top..=bottom {
        for x in [left.saturating_sub(1), (right + 1).min(image.width()-1)] {
            let p = image.get_pixel(x, y); for c in 0..3 { sum[c] += p[c] as u64; } count += 1;
        }
    }
    let color = image::Rgba([(sum[0]/count.max(1)) as u8,(sum[1]/count.max(1)) as u8,(sum[2]/count.max(1)) as u8,255]);
    for x in left..=right { for y in top..=bottom { image.put_pixel(x,y,color); } }
}
fn draw_box(image: &mut image::RgbaImage, cx: i32, cy: i32, tw: u32, th: u32) {
    let left=(cx-tw as i32/2).max(0) as u32; let top=(cy-th as i32/2).max(0) as u32;
    let right=(cx+tw as i32/2).min(image.width() as i32-1) as u32;
    let bottom=(cy+th as i32/2).min(image.height() as i32-1) as u32;
    let color=image::Rgba([255,40,40,255]);
    for x in left..=right { image.put_pixel(x,top,color); image.put_pixel(x,bottom,color); }
    for y in top..=bottom { image.put_pixel(left,y,color); image.put_pixel(right,y,color); }
}
\nfn resolve(base: &Path, relative: &str) -> PathBuf {
    let path = PathBuf::from(relative);
    if path.is_absolute() { path } else { base.join(path) }
}
fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() { return 0.0; }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((p * sorted.len() as f64).ceil() as usize).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}
fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 { 0.0 } else { numerator as f64 / denominator as f64 }
}
fn fmt_point(point: Option<(i32, i32)>) -> String {
    point.map(|(x, y)| format!("{x},{y}")).unwrap_or_else(|| "-".into())
}
fn csv_escape(value: &str) -> String { format!("\"{}\"", value.replace('"', "\"\"")) }

fn print_report(results: &[CaseResult], settings: &Settings, args: &Args, base: &Path) -> Result<()> {
    let total = results.len();
    let tp = results.iter().filter(|r| r.expected.is_some() && r.correct).count();
    let fp = results.iter().filter(|r| r.expected.is_none() && r.found).count()
        + results.iter().filter(|r| r.expected.is_some() && r.found && !r.correct).count();
    let fn_count = results.iter().filter(|r| r.expected.is_some() && !r.correct).count();
    let tn = results.iter().filter(|r| r.expected.is_none() && !r.found).count();
    let accuracy = (tp + tn) as f64 / total.max(1) as f64 * 100.0;
    let precision = ratio(tp, tp + fp) * 100.0;
    let recall = ratio(tp, tp + fn_count) * 100.0;
    let all_ms: Vec<f64> = results.iter().flat_map(|r| r.timings.iter())
        .map(Duration::as_secs_f64).map(|s| s * 1000.0).collect();
    let avg = if all_ms.is_empty() { 0.0 } else { all_ms.iter().sum::<f64>() / all_ms.len() as f64 };

    println!("\n========== robost-vision 视觉识别测试 ==========");
    println!("样本数：{total} | 每张截图迭代：{} | 阈值：{:.2} | 多尺度：{}",
        settings.iterations, settings.threshold, settings.multi_scale);
    println!("准确率：{accuracy:.2}% | Precision：{precision:.2}% | Recall：{recall:.2}%");
    println!("TP：{tp} | FP：{fp} | FN：{fn_count} | TN：{tn}");
    println!("匹配耗时（不含截图读取/模板预处理）：平均 {avg:.3} ms | P50 {:.3} ms | P95 {:.3} ms",
        percentile(&all_ms, 0.50), percentile(&all_ms, 0.95));
    println!("坐标容差：{} px", settings.tolerance_px);
    println!("\n样本明细：");
    println!("{:<28} {:<8} {:<9} {:<12} {:<12} {:<10}",
        "样本", "结果", "得分", "识别坐标", "标注坐标", "误差(px)");
    for r in results {
        let state = if r.correct { "正确" } else if r.found { "误识别" } else { "未识别" };
        let score = r.score.map(|v| format!("{v:.3}")).unwrap_or_else(|| "-".into());
        let center = fmt_point(r.center);
        let expected = fmt_point(r.expected);
        let error = r.error_px.map(|v| format!("{v:.1}")).unwrap_or_else(|| "-".into());
        println!("{:<28} {:<8} {:<9} {:<12} {:<12} {:<10}",
            r.name, state, score, center, expected, error);
    }

    if let Some(csv_path) = &args.csv {
        let output = if csv_path.is_absolute() { csv_path.clone() } else { base.join(csv_path) };
        if let Some(parent) = output.parent() { fs::create_dir_all(parent)?; }
        let mut csv = String::from("case,found,correct,score,center_x,center_y,expected_x,expected_y,error_px,p50_ms,p95_ms,avg_ms\n");
        for r in results {
            let values: Vec<f64> = r.timings.iter().map(Duration::as_secs_f64).map(|v| v * 1000.0).collect();
            let avg = if values.is_empty() { 0.0 } else { values.iter().sum::<f64>() / values.len() as f64 };
            let p50 = percentile(&values, 0.50);
            let p95 = percentile(&values, 0.95);
            let (cx, cy) = r.center.unwrap_or((0, 0));
            let (ex, ey) = r.expected.unwrap_or((0, 0));
            csv.push_str(&format!("{},{},{},{},{},{},{},{},{},{:.4},{:.4},{:.4}\n",
                csv_escape(&r.name), r.found, r.correct,
                r.score.map(|v| v.to_string()).unwrap_or_default(),
                if r.center.is_some() { cx.to_string() } else { String::new() },
                if r.center.is_some() { cy.to_string() } else { String::new() },
                if r.expected.is_some() { ex.to_string() } else { String::new() },
                if r.expected.is_some() { ey.to_string() } else { String::new() },
                r.error_px.map(|v| v.to_string()).unwrap_or_default(), p50, p95, avg));
        }
        fs::write(&output, csv).with_context(|| format!("写入 CSV 失败：{}", output.display()))?;
        println!("\nCSV 报告：{}", output.display());
    }
    Ok(())
}
