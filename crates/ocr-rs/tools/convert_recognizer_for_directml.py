"""Decompose PP-OCRv6 Tiny's opset-11 Softmax for ONNX Runtime DirectML.

Usage:
  python convert_recognizer_for_directml.py <source-rec.onnx> <output-rec.onnx>
Requires the ``onnx`` Python package.
"""

import argparse

import onnx
from onnx import helper


def convert(source: str, destination: str) -> None:
    model = onnx.load(source)
    rewritten = []
    converted = 0

    for node in model.graph.node:
        if node.op_type != "Softmax":
            rewritten.append(node)
            continue

        axis = next((helper.get_attribute_value(attribute) for attribute in node.attribute if attribute.name == "axis"), -1)
        input_name, output_name = node.input[0], node.output[0]
        prefix = node.name or f"softmax_{converted}"
        max_name = f"{prefix}.directml_max"
        shifted_name = f"{prefix}.directml_shifted"
        exp_name = f"{prefix}.directml_exp"
        sum_name = f"{prefix}.directml_sum"
        rewritten.extend(
            [
                helper.make_node("ReduceMax", [input_name], [max_name], name=f"{prefix}.ReduceMax", axes=[axis], keepdims=1),
                helper.make_node("Sub", [input_name, max_name], [shifted_name], name=f"{prefix}.Sub"),
                helper.make_node("Exp", [shifted_name], [exp_name], name=f"{prefix}.Exp"),
                helper.make_node("ReduceSum", [exp_name], [sum_name], name=f"{prefix}.ReduceSum", axes=[axis], keepdims=1),
                helper.make_node("Div", [exp_name, sum_name], [output_name], name=f"{prefix}.Div"),
            ]
        )
        converted += 1

    if converted != 1:
        raise ValueError(f"Expected one Softmax node, converted {converted}; source model may have changed")
    model.graph.ClearField("node")
    model.graph.node.extend(rewritten)
    onnx.checker.check_model(model)
    onnx.save(model, destination)
    print(f"Decomposed {converted} Softmax node: {destination}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("source")
    parser.add_argument("destination")
    arguments = parser.parse_args()
    convert(arguments.source, arguments.destination)
