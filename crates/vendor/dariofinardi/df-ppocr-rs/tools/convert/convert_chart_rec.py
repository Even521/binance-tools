"""
Conversione ChartRecognition (PP-Chart) da PaddlePaddle → ONNX.

ChartRecognition NON è disponibile come ONNX prebuilt su HuggingFace
PaddlePaddle; occorre convertirlo manualmente da questo script.

Il modello PP-Chart converte un'immagine di grafico in una tabella dati
strutturata. Non contiene PII, quindi non è un requisito core per
ppocr-rs; viene incluso per completezza della pipeline PP-StructureV3.

## Prerequisiti

    pip install -r requirements.txt

## Scarica il modello Paddle

    python -m paddle.utils.download download_models chart_rec
    # oppure manualmente da:
    # https://paddleocr.bj.bcebos.com/ppstructure/models/chart/en_PP-Chart_infer.tar
    # https://paddleocr.bj.bcebos.com/ppstructure/models/chart/ch_PP-Chart_infer.tar

## Esecuzione

    python convert_chart_rec.py --model_dir ./en_PP-Chart_infer --lang en
    python convert_chart_rec.py --model_dir ./ch_PP-Chart_infer --lang ch

Output: <lang>_chart_rec.onnx  (input "x", output "fetch_name_0")
"""

import argparse
import os
import numpy as np

def main():
    parser = argparse.ArgumentParser(description="Converti PP-Chart in ONNX")
    parser.add_argument("--model_dir", required=True,
                        help="Directory con il modello Paddle (inference.pdmodel + inference.pdiparams)")
    parser.add_argument("--lang", default="en", choices=["en", "ch"],
                        help="Lingua del modello (default: en)")
    parser.add_argument("--output", default=None,
                        help="Path output ONNX (default: <lang>_chart_rec.onnx)")
    parser.add_argument("--opset", type=int, default=11,
                        help="ONNX opset version (default: 11)")
    args = parser.parse_args()

    output = args.output or f"{args.lang}_chart_rec.onnx"

    # Verifica che il modello Paddle esista
    pdmodel = os.path.join(args.model_dir, "inference.pdmodel")
    pdiparams = os.path.join(args.model_dir, "inference.pdiparams")
    if not os.path.isfile(pdmodel):
        print(f"[ERRORE] inference.pdmodel non trovato in: {args.model_dir}")
        print("  Scarica il modello da:")
        print("  https://paddleocr.bj.bcebos.com/ppstructure/models/chart/en_PP-Chart_infer.tar")
        return 1

    try:
        import paddle2onnx
    except ImportError:
        print("[ERRORE] paddle2onnx non installato. Esegui: pip install -r requirements.txt")
        return 1

    print(f"[INFO] Conversione {pdmodel} → {output} (opset {args.opset})")

    # PP-Chart input: immagine RGB [1, 3, H, W] con shape variabile.
    # L'architettura è tipicamente un encoder visivo + decoder tabulare.
    # Input shape suggerito: [1, 3, 512, 512] (adattato via padding nell'inference).
    ret = paddle2onnx.command.c_paddle_to_onnx(
        model_file     = pdmodel,
        params_file    = pdiparams,
        save_file      = output,
        opset_version  = args.opset,
        enable_dev_version = False,
        # Dimensione statica per massima compatibilità con ort
        input_shape_dict = '{"x": [1, 3, 512, 512]}',
    )
    if ret != 0:
        print(f"[ERRORE] paddle2onnx ha restituito codice {ret}")
        return ret

    print(f"[OK] Modello salvato: {output}")

    # Verifica del grafo ONNX
    try:
        import onnx
        model = onnx.load(output)
        onnx.checker.check_model(model)
        inputs  = [i.name for i in model.graph.input]
        outputs = [o.name for o in model.graph.output]
        print(f"[OK] ONNX checker OK — inputs: {inputs}, outputs: {outputs}")
    except ImportError:
        print("[WARN] onnx non installato, skip checker")
    except Exception as e:
        print(f"[WARN] ONNX checker fallito: {e}")

    # Smoke test con onnxruntime
    try:
        import onnxruntime as ort
        sess = ort.InferenceSession(output, providers=["CPUExecutionProvider"])
        dummy = np.zeros((1, 3, 512, 512), dtype=np.float32)
        input_name = sess.get_inputs()[0].name
        result = sess.run(None, {input_name: dummy})
        print(f"[OK] Smoke test OK — output shapes: {[r.shape for r in result]}")
    except ImportError:
        print("[WARN] onnxruntime non installato, skip smoke test")
    except Exception as e:
        print(f"[WARN] Smoke test fallito: {e}")

    print()
    print("Integrazione in ppocr-rs:")
    print("  Copia il file ONNX nella cache dir:")
    print(f"    %LOCALAPPDATA%\\ppocr-rs\\models\\chart_rec\\inference.onnx")
    print("  Poi implementa ChartRecognizer in src/chart_rec.rs usando lo stesso")
    print("  preprocessing ImageNet (512×512) di SLANeXt/DocOrientation.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
