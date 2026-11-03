# Ollama 并发启动脚本（在运行 Ollama 模型的服务器上执行）
# 使用前先从系统托盘完全退出已运行的 Ollama。
# Ctrl+C 可停止本次启动的 Ollama 服务。

$ErrorActionPreference = "Stop"

$ollama = Get-Command "ollama" -ErrorAction SilentlyContinue
if (-not $ollama) {
    throw "找不到 ollama 命令。请先安装 Ollama，或将 ollama.exe 加入 PATH。"
}

$listener = Get-NetTCPConnection -LocalPort 11434 -State Listen -ErrorAction SilentlyContinue |
    Select-Object -First 1
if ($listener) {
    throw "11434 端口已有进程监听（PID: $($listener.OwningProcess)）。请先从系统托盘完全退出 Ollama，再运行此脚本；不要同时启动两个服务。"
}

# 两台客户端可同时请求同一个模型。并发数提高会增加显存/内存占用。
$env:OLLAMA_HOST = "0.0.0.0:11434"
$env:OLLAMA_NUM_PARALLEL = "2"
$env:OLLAMA_MAX_QUEUE = "64"
$env:OLLAMA_KEEP_ALIVE = "10m"

Write-Host "启动 Ollama 并发服务..." -ForegroundColor Cyan
Write-Host "监听地址：0.0.0.0:11434"
Write-Host "每个模型最大并发请求：$env:OLLAMA_NUM_PARALLEL"
Write-Host "最大排队请求：$env:OLLAMA_MAX_QUEUE"
Write-Host "模型保留时间：$env:OLLAMA_KEEP_ALIVE"
Write-Host "如果显存不足，两个请求仍可能排队；可将 OLLAMA_NUM_PARALLEL 调回 1。" -ForegroundColor Yellow

& $ollama.Source serve
