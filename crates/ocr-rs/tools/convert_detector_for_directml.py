"""Replace PP-OCRv6 Tiny detector integer nearest Resize ops for DirectML.

The source detector exports six NCHW Resize ops that DirectML fails to execute
with ORT 2.0.0-rc.13. Each uses nearest/asymmetric/floor with integral H/W
scale, which is exactly a depthwise ConvTranspose with an all-ones kernel and
matching stride. This script creates the checked-in DirectML-specific model.

Usage:
  python convert_detector_for_directml.py <source-det.onnx> <output-det.onnx>
Requires the ``onnx`` and ``numpy`` Python packages.
"""

import argparse

import numpy as np
import onnx
from onnx import helper, numpy_helper, shape_inference


def convert(source: str, destination: str) -> None:
    model = onnx.load(source)
    inferred = shape_inference.infer_shapes(model)
    shapes = {
        value.name: [dim.dim_value or dim.dim_param for dim in value.type.tensor_type.shape.dim]
        for value in [*inferred.graph.value_info, *inferred.graph.input]
    }
    initializers = {tensor.name: numpy_helper.to_array(tensor) for tensor in model.graph.initializer}
    converted = 0

    for node in model.graph.node:
        if node.op_type != "Resize":
            continue

        if len(node.input) < 3 or node.input[2] not in initializers:
            raise ValueError(f"{node.name}: Resize scales must be constant")
        scales = initializers[node.input[2]]
        if not (
            scales.shape == (4,)
            and scales[0] == 1
            and scales[1] == 1
            and scales[2] == scales[3]
            and float(scales[2]).is_integer()
        ):
            raise ValueError(f"{node.name}: unsupported Resize scales {scales}")

        attributes = {attribute.name: helper.get_attribute_value(attribute) for attribute in node.attribute}
        if not (
            attributes.get("mode") == b"nearest"
            and attributes.get("coordinate_transformation_mode") == b"asymmetric"
            and attributes.get("nearest_mode") == b"floor"
        ):
            raise ValueError(f"{node.name}: unsupported Resize attributes {attributes}")

        input_name, output_name = node.input[0], node.output[0]
        input_shape = shapes.get(input_name)
        if input_shape is None or len(input_shape) != 4 or not isinstance(input_shape[1], int):
            raise ValueError(f"{node.name}: cannot determine NCHW channel count for {input_name}")

        factor = int(scales[2])
        channels = input_shape[1]
        weight_name = f"{node.name}.directml_depthwise_upsample_weight"
        weight = np.ones((channels, 1, factor, factor), dtype=np.float32)
        model.graph.initializer.append(numpy_helper.from_array(weight, name=weight_name))

        node.op_type = "ConvTranspose"
        del node.input[:]
        node.input.extend([input_name, weight_name])
        del node.attribute[:]
        for name, value in (
            ("strides", [factor, factor]),
            ("kernel_shape", [factor, factor]),
            ("pads", [0, 0, 0, 0]),
            ("group", channels),
        ):
            node.attribute.append(helper.make_attribute(name, value))
        node.output[0] = output_name
        converted += 1

    if converted == 0:
        raise ValueError("No supported Resize nodes found; source model may have changed")
    onnx.checker.check_model(model)
    onnx.save(model, destination)
    print(f"Converted {converted} Resize nodes: {destination}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("source")
    parser.add_argument("destination")
    arguments = parser.parse_args()
    convert(arguments.source, arguments.destination)
