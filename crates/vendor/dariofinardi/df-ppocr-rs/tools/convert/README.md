# tools/convert — Conversione modelli Paddle-only → ONNX

Alcuni modelli PP-StructureV3 non sono disponibili come ONNX prebuilt su
HuggingFace `PaddlePaddle/`. Questa cartella contiene gli script per
convertirli da PaddlePaddle al formato ONNX richiesto da `ppocr-rs`.

## Mappa di disponibilità

| Modello                      | ONNX prebuilt (HF) | Script di conversione        |
|------------------------------|:------------------:|------------------------------|
| PP-OCRv6 det/rec             | ✅                 | — (usa `ModelHub::ensure`)   |
| PP-LCNet_x1_0_table_cls      | ✅                 | — (usa `ModelHub::ensure_single(TableCls)`) |
| SLANeXt_wired/wireless       | ✅                 | — (usa `ModelHub::ensure_single(TableStructureWired/Wireless)`) |
| RT-DETR-L cell det (wired)   | ✅ (kreuzberg-dev) | — già integrato in `CellDetector` |
| RT-DETR-L cell det (wireless)| ✅                 | — (usa `ModelHub::ensure_single(CellDetWireless)`) |
| PP-LCNet_x1_0_doc_ori        | ✅                 | — (usa `ModelHub::ensure_single(DocOrientation)`) |
| UVDoc (unwarping)            | ✅                 | — (usa `ModelHub::ensure_single(DocUnwarp)`) |
| PP-FormulaNet_plus-L (rec)   | ✅                 | — (usa `ModelHub::ensure_single(FormulaRec)`) |
| **PP-FormulaNet-S/L (det)**  | ❌                 | `convert_formula_det.py`     |
| **ChartRecognition**         | ❌                 | `convert_chart_rec.py`       |

## Setup

```powershell
cd tools/convert
python -m venv .venv
.venv\Scripts\Activate.ps1
pip install -r requirements.txt
```

## ChartRecognition

ChartRecognition non è pubblicato come ONNX. Per convertirlo:

```powershell
# 1. Scarica il modello Paddle (tar ~50 MB)
Invoke-WebRequest `
  -Uri "https://paddleocr.bj.bcebos.com/ppstructure/models/chart/en_PP-Chart_infer.tar" `
  -OutFile "en_PP-Chart_infer.tar"
tar -xf en_PP-Chart_infer.tar

# 2. Converti
python convert_chart_rec.py --model_dir ./en_PP-Chart_infer --lang en

# 3. Copia nella cache ppocr-rs
$dest = "$env:LOCALAPPDATA\ppocr-rs\models\chart_rec"
New-Item -ItemType Directory -Force $dest
Copy-Item en_chart_rec.onnx "$dest\inference.onnx"
```

## PP-FormulaNet detection

Il componente di **recognition** è già disponibile su HuggingFace;
solo il componente di **detection** (individua le regioni formula) richiede
conversione manuale:

```powershell
# Scarica
Invoke-WebRequest `
  -Uri "https://paddleocr.bj.bcebos.com/ppstructure/models/formula/en_PP-FormulaNet-S_det_infer.tar" `
  -OutFile "en_PP-FormulaNet-S_det_infer.tar"
tar -xf en_PP-FormulaNet-S_det_infer.tar

# Converti
python convert_formula_det.py --model_dir ./en_PP-FormulaNet-S_det_infer --size S

# Copia
$dest = "$env:LOCALAPPDATA\ppocr-rs\models\formula_det"
New-Item -ItemType Directory -Force $dest
Copy-Item formula_det_S.onnx "$dest\inference.onnx"
```

## Note

- `paddle2onnx` richiede PaddlePaddle **CPU** installato (non serve GPU).
- I modelli convertiti con opset 11 sono compatibili con `ort 2.0.0-rc.9`.
- La shape statica (`[1, 3, H, W]` fissa) è necessaria per ort su ARM64
  Snapdragon X Elite dove shape dinamiche possono crashare.
- Il modello chart convertito **non è rilevante per PII** (i grafici non
  contengono dati personali identificabili nei documenti legali/medici target).
