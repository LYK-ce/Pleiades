# Presented by KeJi
# Date: 2026-03-23

"""
ONNX 模型输入/输出检查脚本

使用方法:
    python tests/common/check_onnx_input.py
    python tests/common/check_onnx_input.py --model tests/test_models/qwen3-0.6b/onnx/model.onnx

功能:
    1. 列出模型的所有输入及其形状
    2. 列出模型的所有输出及其形状
    3. 生成 Rust 代码片段用于创建输入张量
"""

import sys
import argparse
from pathlib import Path

# 尝试导入 onnx
try:
    import onnx
except ImportError:
    print("[错误] 请先安装 onnx: pip install onnx")
    sys.exit(1)


def Get_Model_Path():
    """获取模型路径"""
    parser = argparse.ArgumentParser(description='检查 ONNX 模型输入/输出')
    parser.add_argument('--model', type=str, 
                        default='tests/test_models/qwen3-0.6b/onnx/model.onnx',
                        help='ONNX 模型路径')
    args = parser.parse_args()
    return Path(args.model)


def Get_Shape_Str(tensor_type):
    """将 ONNX 形状转换为字符串"""
    shape = []
    for dim in tensor_type.shape.dim:
        if dim.HasField('dim_value'):
            shape.append(str(dim.dim_value))
        elif dim.HasField('dim_param'):
            shape.append(f"{dim.dim_param}")
        else:
            shape.append("?")
    return f"[{', '.join(shape)}]"


def Get_Data_Type_Str(tensor_type):
    """将 ONNX 数据类型转换为可读字符串"""
    type_map = {
        1: "Float32",
        2: "UInt8",
        3: "Int8",
        4: "UInt16",
        5: "Int16",
        6: "Int32",
        7: "Int64",
        8: "String",
        9: "Bool",
        10: "Float16",
        11: "Double",
        12: "UInt32",
        13: "UInt64",
        14: "Complex64",
        15: "Complex128",
        16: "BFloat16",
    }
    return type_map.get(tensor_type.elem_type, f"Unknown({tensor_type.elem_type})")


def Check_Model(model_path: Path):
    """检查 ONNX 模型"""
    if not model_path.exists():
        print(f"[错误] 模型文件不存在: {model_path}")
        print(f"[提示] 请确保模型已下载到指定路径")
        sys.exit(1)
    
    print("=" * 60)
    print(f"ONNX 模型检查: {model_path}")
    print("=" * 60)
    
    # 加载模型（大模型使用 load_external_data=False）
    try:
        # 对于大于 2GB 的模型，使用 load_external_data=False
        model = onnx.load(str(model_path), load_external_data=False)
        print(f"[成功] 模型加载成功（仅加载结构，不包含外部数据）")
    except Exception as e:
        print(f"[错误] 模型加载失败: {e}")
        sys.exit(1)
    
    # 打印模型信息
    print(f"\n模型信息:")
    print(f"  IR 版本: {model.ir_version}")
    print(f"  生产方: {model.producer_name} {model.producer_version}")
    print(f"  模型版本: {model.model_version}")
    
    # 打印输入
    print(f"\n{'=' * 60}")
    print(f"模型输入 ({len(model.graph.input)} 个):")
    print(f"{'=' * 60}")
    
    for i, input in enumerate(model.graph.input):
        name = input.name
        tensor_type = input.type.tensor_type
        shape_str = Get_Shape_Str(tensor_type)
        dtype_str = Get_Data_Type_Str(tensor_type)
        
        print(f"\n  [{i}] {name}")
        print(f"      形状: {shape_str}")
        print(f"      类型: {dtype_str}")
    
    # 打印输出
    print(f"\n{'=' * 60}")
    print(f"模型输出 ({len(model.graph.output)} 个):")
    print(f"{'=' * 60}")
    
    for i, output in enumerate(model.graph.output):
        name = output.name
        tensor_type = output.type.tensor_type
        shape_str = Get_Shape_Str(tensor_type)
        dtype_str = Get_Data_Type_Str(tensor_type)
        
        print(f"\n  [{i}] {name}")
        print(f"      形状: {shape_str}")
        print(f"      类型: {dtype_str}")
    
    # 生成 Rust 代码片段
    print(f"\n{'=' * 60}")
    print(f"Rust 代码片段 (用于创建输入张量):")
    print(f"{'=' * 60}")
    
    print("\nuse ort::value::Tensor;")
    print("use std::collections::HashMap;")
    print("\n// 创建输入张量")
    
    for input in model.graph.input:
        name = input.name
        tensor_type = input.type.tensor_type
        dtype_str = Get_Data_Type_Str(tensor_type)
        
        # 提取形状维度
        dims = []
        for dim in tensor_type.shape.dim:
            if dim.HasField('dim_value'):
                dims.append(str(dim.dim_value))
            elif dim.HasField('dim_param'):
                dims.append(f"seq_len")  # 假设动态维度是 seq_len
            else:
                dims.append("1")
        
        shape_str = f"vec![{', '.join(dims)}]"
        
        if dtype_str == "Int64":
            print(f"\n// {name} ({dtype_str})")
            print(f"let {name}_data: Vec<i64> = vec![...]; // 填入实际数据")
            print(f"let {name}_tensor = Tensor::from_array(({shape_str}, {name}_data))?;")
        elif dtype_str == "Float32":
            print(f"\n// {name} ({dtype_str})")
            print(f"let {name}_data: Vec<f32> = vec![...]; // 填入实际数据")
            print(f"let {name}_tensor = Tensor::from_array(({shape_str}, {name}_data))?;")
        else:
            print(f"\n// {name} ({dtype_str}) - 需要相应类型")
    
    print(f"\n// 构建输入 HashMap")
    print(f"let mut inputs: HashMap<String, ort::DynValue> = HashMap::new();")
    for input in model.graph.input:
        name = input.name
        print(f"inputs.insert(\"{name}\".to_string(), {name}_tensor.into_dyn());")
    
    print(f"\n{'=' * 60}")
    print(f"检查完成!")
    print(f"{'=' * 60}")


def main():
    model_path = Get_Model_Path()
    Check_Model(model_path)


if __name__ == "__main__":
    main()
