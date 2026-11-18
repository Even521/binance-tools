# Vision Benchmark

独立测试 `robost-vision` 的 X 页面按钮模板匹配能力。该 crate 不接入 `desktop-gpui`，不修改现有 X 助手和 PP-OCRv6。

## 1. 准备测试素材

截图可能包含账号和个人信息，测试素材默认放在仓库根目录 `data/vision-benchmark/`（该目录已被根 `.gitignore` 忽略，不会误提交）。

目录结构：

```text
data/vision-benchmark/
  screenshots/
    home-01.png
    home-02.png
  templates/
    reply.png
    like.png
    repost.png
```

仓库自带的三个 PNG 是用于先跑通流程的近似风格演示模板，并非从 X 官网提取的官方 SVG。实际测试时，建议再从你使用的 X 页面截图中裁剪真实按钮图标，替换这三个 PNG；模板只保留图标本身和少量边缘背景，不要包含整条互动栏。

## 2. 标注截图

将示例清单和内置模板复制到测试目录：

```powershell
New-Item -ItemType Directory -Force data/vision-benchmark/templates
Copy-Item crates/vision-benchmark/benchmark.example.json data/vision-benchmark/benchmark.json
Copy-Item crates/vision-benchmark/templates/*.png data/vision-benchmark/templates/
```

每个正样本填写按钮中心点 `expected_center`，坐标使用完整截图的像素坐标；截图左上角不是屏幕 (0,0) 时设置 `origin`。负样本省略 `expected_center`，用于测量误报。

示例：

```json
{
  "name": "home-reply-01",
  "screenshot": "screenshots/home-01.png",
  "template": "reply",
  "expected_center": { "x": 612, "y": 742 }
}
```

每种按钮准备多张截图，覆盖不同帖子位置、滚动位置、亮/暗主题、浏览器缩放和 DPI。建议至少 20 张正样本 + 10 张负样本后再判断可靠性。

## 3. 直接扫描一张现有截图

无需先填写 `cases` 和人工标注坐标。可以直接把 X 页面截图路径传给程序，程序会加载清单里的 reply / like / repost 模板并扫描整张截图：

```powershell
cargo run -p vision-benchmark --release -- --manifest crates/vision-benchmark/benchmark.example.json --scan-image "D:\rust\binance-tools\data\x_assistant\screenshots\round-098\20261111-032625-660-01-feed.png"
```

输出每个模板是否找到、匹配中心坐标、匹配分数以及平均/P95耗时。坐标以截图左上角为原点；如果截图对应屏幕非 (0,0) 起点，可通过清单中的 `origin_x` / `origin_y` 设置屏幕原点。

## 4. 批量准确率测试

在仓库根目录执行：

```powershell
cargo run -p vision-benchmark --release -- --manifest data/vision-benchmark/benchmark.json --csv data/vision-benchmark/report.csv
```

默认阈值 0.87，多尺度匹配，迭代 30 次。每个样本先预热一次；耗时统计不包含截图解码、模板加载和模板灰度预处理，只统计匹配调用本身。

报告包括 Accuracy、Precision、Recall、FP/FN、P50/P95/平均耗时、NCC 得分、识别中心点与人工标注中心点的像素误差。CSV 可用 Excel 打开。

## 5. 注意事项

模板匹配不是语义识别。按钮图标、主题、缩放、动画和布局变化都会影响结果。不要只凭 NCC 分数就点击；先评估坐标误差和负样本误报率。

本测试第一阶段只测 `robost-vision`；后续可在同一批截图上增加 PP-OCRv6 坐标识别耗时对照。测试结果不是线上自动点击安全保证。
