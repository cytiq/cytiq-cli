use std::path::Path;

use onnx_protobuf::{
    GraphProto, Message, ModelProto, NodeProto, OperatorSetIdProto, StringStringEntryProto,
    TensorShapeProto, TypeProto, ValueInfoProto, tensor_proto, tensor_shape_proto, type_proto,
};
use protobuf::MessageField;

const GRID_SIZE: i64 = 101;

/// Write a minimal ONNX identity model with the custom metadata expected by
/// `cytiq-gate`.
///
/// # Errors
///
/// Returns an error when the ONNX protobuf cannot be serialized or the target
/// path cannot be written.
pub fn write_identity_unito_model(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = ModelProto::new();
    model.ir_version = 9;
    "cytiq-tests".clone_into(&mut model.producer_name);
    model.opset_import.push(OperatorSetIdProto {
        domain: String::new(),
        version: 13,
        special_fields: protobuf::SpecialFields::default(),
    });
    model.metadata_props = vec![
        metadata("cytiq.gate_name", "Single_cell_gate_1"),
        metadata("cytiq.x_axis", "FSC-A"),
        metadata("cytiq.y_axis", "SSC-A"),
    ];

    let mut graph = GraphProto::new();
    "cytiq_gate_identity".clone_into(&mut graph.name);
    graph.input.push(value_info("input"));
    graph.output.push(value_info("output"));
    graph.node.push(NodeProto {
        input: vec!["input".to_owned()],
        output: vec!["output".to_owned()],
        name: "identity".to_owned(),
        op_type: "Identity".to_owned(),
        ..NodeProto::new()
    });
    model.graph = MessageField::some(graph);

    std::fs::write(path, model.write_to_bytes()?)?;
    Ok(())
}

fn value_info(name: &str) -> ValueInfoProto {
    let mut tensor = type_proto::Tensor::new();
    tensor.elem_type = tensor_proto::DataType::FLOAT as i32;
    tensor.shape = MessageField::some(TensorShapeProto {
        dim: vec![dim(1), dim(1), dim(GRID_SIZE), dim(GRID_SIZE)],
        ..TensorShapeProto::new()
    });

    let mut type_proto = TypeProto::new();
    type_proto.set_tensor_type(tensor);

    ValueInfoProto {
        name: name.to_owned(),
        type_: MessageField::some(type_proto),
        ..ValueInfoProto::new()
    }
}

fn dim(value: i64) -> tensor_shape_proto::Dimension {
    let mut dimension = tensor_shape_proto::Dimension::new();
    dimension.set_dim_value(value);
    dimension
}

fn metadata(key: &str, value: &str) -> StringStringEntryProto {
    StringStringEntryProto {
        key: key.to_owned(),
        value: value.to_owned(),
        ..StringStringEntryProto::new()
    }
}
