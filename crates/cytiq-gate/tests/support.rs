use std::path::Path;

use onnx_protobuf::{
    GraphProto, Message, ModelProto, NodeProto, OperatorSetIdProto, StringStringEntryProto,
    TensorProto, TensorShapeProto, TypeProto, ValueInfoProto, tensor_proto, tensor_shape_proto,
    type_proto,
};
use protobuf::{EnumOrUnknown, MessageField};

const GRID_SIZE: i64 = 101;

struct UnitoMetadata<'a> {
    gate_name: &'a str,
    x_axis: &'a str,
    y_axis: &'a str,
    parent_gate: Option<&'a str>,
}

/// Write a minimal ONNX identity model with the custom metadata expected by
/// `cytiq-gate`.
///
/// # Errors
///
/// Returns an error when the ONNX protobuf cannot be serialized or the target
/// path cannot be written.
pub fn write_identity_unito_model(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_identity_unito_model_with_metadata(path, "Single_cell_gate_1", "FSC-A", "SSC-A", None)
}

/// Write a minimal ONNX identity model with caller-provided metadata.
///
/// # Errors
///
/// Returns an error when the ONNX protobuf cannot be serialized or the target
/// path cannot be written.
pub fn write_identity_unito_model_with_metadata(
    path: &Path,
    gate_name: &str,
    x_axis: &str,
    y_axis: &str,
    parent_gate: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    write_unito_model(
        path,
        Some(UnitoMetadata {
            gate_name,
            x_axis,
            y_axis,
            parent_gate,
        }),
        None,
    )
}

/// Write a minimal ONNX model without the custom metadata expected by
/// `cytiq-gate`.
///
/// # Errors
///
/// Returns an error when the ONNX protobuf cannot be serialized or the target
/// path cannot be written.
#[allow(dead_code)]
pub fn write_unito_model_without_metadata(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_unito_model(path, None, None)
}

/// Write a minimal ONNX model that references an external weights file.
///
/// # Errors
///
/// Returns an error when the ONNX protobuf cannot be serialized or the target
/// path cannot be written.
pub fn write_unito_model_with_external_data(
    path: &Path,
    location: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    write_unito_model_with_external_data_and_metadata(
        path,
        location,
        "Single_cell_gate_1",
        "FSC-A",
        "SSC-A",
        None,
    )
}

/// Write a minimal ONNX model with caller-provided metadata and an external
/// weights reference.
///
/// # Errors
///
/// Returns an error when the ONNX protobuf cannot be serialized or the target
/// path cannot be written.
pub fn write_unito_model_with_external_data_and_metadata(
    path: &Path,
    location: &str,
    gate_name: &str,
    x_axis: &str,
    y_axis: &str,
    parent_gate: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    write_unito_model(
        path,
        Some(UnitoMetadata {
            gate_name,
            x_axis,
            y_axis,
            parent_gate,
        }),
        Some(location),
    )
}

fn write_unito_model(
    path: &Path,
    metadata_values: Option<UnitoMetadata<'_>>,
    external_data_location: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = ModelProto::new();
    model.ir_version = 9;
    "cytiq-tests".clone_into(&mut model.producer_name);
    model.opset_import.push(OperatorSetIdProto {
        domain: String::new(),
        version: 13,
        special_fields: protobuf::SpecialFields::default(),
    });
    if let Some(metadata_values) = metadata_values {
        model.metadata_props = vec![
            metadata("cytiq.gate_name", metadata_values.gate_name),
            metadata("cytiq.x_axis", metadata_values.x_axis),
            metadata("cytiq.y_axis", metadata_values.y_axis),
        ];
        if let Some(parent_gate) = metadata_values.parent_gate {
            model
                .metadata_props
                .push(metadata("cytiq.parent_gate", parent_gate));
        }
    }

    let mut graph = GraphProto::new();
    "cytiq_gate_identity".clone_into(&mut graph.name);
    graph.input.push(value_info("input"));
    graph.output.push(value_info("output"));
    match external_data_location {
        Some(location) => {
            graph.initializer.push(external_tensor("bias", location));
            graph.node.push(NodeProto {
                input: vec!["input".to_owned(), "bias".to_owned()],
                output: vec!["output".to_owned()],
                name: "add".to_owned(),
                op_type: "Add".to_owned(),
                ..NodeProto::new()
            });
        }
        None => {
            graph.node.push(NodeProto {
                input: vec!["input".to_owned()],
                output: vec!["output".to_owned()],
                name: "identity".to_owned(),
                op_type: "Identity".to_owned(),
                ..NodeProto::new()
            });
        }
    }
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

fn external_tensor(name: &str, location: &str) -> TensorProto {
    TensorProto {
        dims: vec![1],
        data_type: tensor_proto::DataType::FLOAT as i32,
        name: name.to_owned(),
        external_data: vec![metadata("location", location)],
        data_location: EnumOrUnknown::new(tensor_proto::DataLocation::EXTERNAL),
        ..TensorProto::new()
    }
}
