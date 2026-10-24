"""
Conversione PP-FormulaNet detection (rilevamento formule) da Paddle → ONNX.

La pipeline formule ha DUE componenti:
  1. Formula DETECTION — individua le regioni formula nell'immagine
     (non disponibile come ONNX su HuggingFace; da convertire con questo script)
  2. Formula RECOGNITION — converte il crop formula in LaTeX
     (disponibile come ONNX: `PaddlePaddle/PP-FormulaNet_plus-L_onnx`)

Questo script si occupa SOLO del componente detection (step 1).

## Modelli detection disponibili su PaddlePaddle

  - PP-FormulaNet-S (detection): leggero, < 5M params
  - PP-FormulaNet-L (detection): più accurato, ~20M params

Il componente di recognition (step 2) è già scaricabile via:
  ModelHub::ensure_single(PpStructureModel::FormulaRec)

## Prerequisiti

    pip install -r requirements.txt

## Scarica il modello

    # Modello detection - incluso nel bundle PP-StructureV3:
    # https://paddleocr.bj.bcebos.com/ppstructure/models/formula/en_PP-FormulaNet-S_det_infer.tar

## Esecuzione

    python convert_formula_det.py --model_dir ./en_PP-FormulaNet-S_det_infer --size S
    python convert_formula_det.py --model_dir ./en_PP-FormulaNet-L_det_infer --size L

Output: formula_det_<size>.onnx
"""

import argparse
import os
import numpy as np


def main():
    parser = argparse.ArgumentParser(description="Converti PP-FormulaNet detection in ONNX")
    parser.add_argument("--model_dir", required=True,
                        help="Directory con inference.pdmodel + inference.pdiparams")
    parser.add_argument("--size", default="S", choices=["S", "L"],
                        help="Variante modello S (leggero) o L (accurato)")
    parser.add_argument("--output", default=None,
                        help="Path output ONNX (default: formula_det_<size>.onnx)")
    parser.add_argument("--opset", type=int, default=11,
                        help="ONNX opset version (default: 11)")
    args = parser.parse_args()

    output = args.output or f"formula_det_{args.size}.onnx"

    pdmodel = os.path.join(args.model_dir, "inference.pdmodel")
    if not os.path.isfile(pdmodel):
        print(f"[ERRORE] inference.pdmodel non trovato in: {args.model_dir}")
        print("  Scarica da: https://paddleocr.bj.bcebos.com/ppstructure/models/formula/")
        print("  en_PP-FormulaNet-S_det_infer.tar oppure en_PP-FormulaNet-L_det_infer.tar")
        return 1

    try:
        import paddle2onnx
    except ImportError:
        print("[ERRORE] paddle2onnx non installato. Esegui: pip install -r requirements.txt")
        return 1

    pdiparams = os.path.join(args.model_dir, "inference.pdiparams")
    print(f"[INFO] Conversione {pdmodel} → {output} (opset {args.opset})")

    # PP-FormulaNet-det è tipicamente un detector RT-DETR / DETR-based.
    # Input: immagine [1, 3, H, W]. Per ONNX statico usiamo 800×800.
    ret = paddle2onnx.command.c_paddle_to_onnx(
        model_file     = pdmodel,
        params_file    = pdiparams,
        save_file      = output,
        opset_version  = args.opset,
        enable_dev_version = False,
        input_shape_dict = '{"x": [1, 3, 800, 800]}',
    )
    if ret != 0:
        print(f"[ERRORE] paddle2onnx ha restituito codice {ret}")
        return ret

    print(f"[OK] Modello salvato: {output}")

    try:
        import onnx
        model = onnx.load(output)
        onnx.checker.check_model(model)
        inputs  = [i.name for i in model.graph.input]
        outputs = [o.name for o in model.graph.output]
        print(f"[OK] ONNX checker OK — inputs: {inputs}, outputs: {outputs}")
    except Exception as e:
        print(f"[WARN] ONNX checker: {e}")

    try:
        import onnxruntime as ort
        sess = ort.InferenceSession(output, providers=["CPUExecutionProvider"])
        dummy = np.zeros((1, 3, 800, 800), dtype=np.float32)
        inp = sess.get_inputs()[0].name
        result = sess.run(None, {inp: dummy})
        print(f"[OK] Smoke test OK — output shapes: {[r.shape for r in result]}")
    except Exception as e:
        print(f"[WARN] Smoke test: {e}")

    print()
    print("Integrazione in ppocr-rs:")
    print("  Copia in cache: %LOCALAPPDATA%\\ppocr-rs\\models\\formula_det\\inference.onnx")
    print("  Poi implementa FormulaDetector in src/formula_det.rs.")
    print("  Il riconoscimento (recognition) è già disponibile via PpStructureModel::FormulaRec.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
