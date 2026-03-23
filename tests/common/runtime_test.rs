//Presented by KeJi
//Date: 2026-03-21

//! Runtime 模块测试

use pleiades::{Runtime, RuntimeConfig, Init_ONNX, RuntimeInitConfig, TensorPacket, DataType};
use std::collections::HashMap;
use ndarray::ArrayD;
use ndarray_npy::NpzReader;
use std::fs::File;

// 导入 ort 类型用于直接调用 Execute
use ort::value::{Tensor, DynValue};

/// 测试加载模型并打印 execution provider
#[test]
pub fn Test_Load_Model_And_Print_EP() {
    // 配置的 device 类型（从环境变量或配置读取）
    let configured_device = "cuda"; // 这里改为 cuda，但实际可能回退到 CPU
    
    // 初始化 ONNX Runtime
    let init_config = RuntimeInitConfig {
        device: configured_device.to_string(),
    };
    Init_ONNX(init_config).expect("Init_ONNX 失败");

    // 创建 Runtime
    let config = RuntimeConfig::default();
    let mut runtime = Runtime::new(config).expect("创建 Runtime 失败");

    // 加载模型
    let model_path = "tests/test_models/deit_tiny.onnx";
    let model_info = runtime.Load_Model(model_path).expect("加载模型失败");

    println!("模型加载成功: {}", model_info.path);
    
    // 打印配置的 device 和可能的回退信息
    println!("配置的 Execution Provider: {}", configured_device);
    println!("实际 Execution Provider: 取决于 ort 内部实现，如果 {} 不可用会自动回退到 CPU", configured_device);
    
    // 检查 CUDA 为什么可能不工作
    println!("\n注意：如果配置了 CUDA 但仍运行在 CPU 上，可能原因：");
    println!("1. CUDA 驱动未安装或版本不匹配");
    println!("2. CUDA Toolkit 未安装");
    println!("3. onnxruntime 未编译 CUDA 支持（需要 cargo build --features cuda）");
    println!("4. 显卡不支持 CUDA");
    
    // 卸载模型
    runtime.Unload_Model();
    println!("模型已卸载");
}

/// 从 NPZ 文件读取数组数据
fn Read_Npz_Array(npz: &mut NpzReader<File>, key: &str) -> Result<ArrayD<f32>, String> {
    // ndarray-npy 使用索引访问数组
    // 首先找到对应的数组索引
    let names = npz.names().map_err(|e| format!("无法读取数组列表: {}", e))?;
    
    let idx = names.iter()
        .position(|n| n == key)
        .ok_or_else(|| format!("无法找到 {} 数组", key))?;
    
    // 读取数组数据
    let array: ArrayD<f32> = npz.by_index(idx)
        .map_err(|e| format!("无法读取 {} 数据: {}", key, e))?;
    
    Ok(array)
}

/// 测试完整模型推理 - 加载模型，读取测试数据，执行推理并对比结果
#[test]
pub fn Test_Full_Model() {
    // 初始化 ONNX Runtime
    let init_config = RuntimeInitConfig {
        device: "cuda".to_string(),
    };
    Init_ONNX(init_config).expect("Init_ONNX 失败");

    // 读取测试数据
    let test_data_path = "tests/test_models/test_data.npz";
    let file = File::open(test_data_path).expect("无法打开 test_data.npz");
    let mut npz = NpzReader::new(file).expect("无法读取 NPZ 文件");

    // 打印 NPZ 文件中所有数组名称
    let names = npz.names().expect("无法读取数组名称列表");
    println!("NPZ 文件中的数组: {:?}", names);

    // 定义要测试的模型 (键名对应 NPZ 文件中的数组名称)
    let models = vec![
        ("mlp", "mlp", "tests/test_models/mlp.onnx"),
        ("vgg11", "vgg11", "tests/test_models/vgg11.onnx"),
        ("deit_tiny", "deit", "tests/test_models/deit_tiny.onnx"),
    ];

    for (model_name, key_prefix, model_path) in models {
        println!("\n========================================");
        println!("测试模型: {}", model_name);
        println!("========================================");

        // 创建 Runtime
        let config = RuntimeConfig::default();
        let mut runtime = Runtime::new(config.clone()).expect("创建 Runtime 失败");

        // 加载模型
        let model_info = runtime.Load_Model(model_path).expect(&format!("加载 {} 失败", model_name));
        println!("模型加载成功: {}", model_info.path);

        // 读取输入数据
        let input_key = format!("{}_input", key_prefix);
        let input_array = Read_Npz_Array(&mut npz, &input_key)
            .expect(&format!("无法读取 {} 输入数据", input_key));
        println!("输入数据形状: {:?}", input_array.shape());

        // 创建 TensorPacket
        let input_shape: Vec<usize> = input_array.shape().to_vec();
        let input_data: Vec<f32> = input_array.iter().copied().collect();
        let input_packet = TensorPacket::From_F32_Slice(
            "input".to_string(),
            input_shape,
            &input_data,
        ).expect("创建输入 TensorPacket 失败");

        // 执行推理
        let mut inputs = HashMap::new();
        inputs.insert("input".to_string(), input_packet);
        
        let outputs = runtime.Execute_From_Packets(inputs)
            .expect(&format!("{} 推理失败", model_name));
        
        println!("推理完成，输出数量: {}", outputs.len());

        // 读取预期输出数据
        let output_key = format!("{}_output", key_prefix);
        let expected_output = Read_Npz_Array(&mut npz, &output_key)
            .expect(&format!("无法读取 {} 预期输出数据", output_key));
        println!("预期输出形状: {:?}", expected_output.shape());

        // 对比输出结果
        if let Some(output_packet) = outputs.get("output") {
            let output_data = output_packet.To_F32_Vec()
                .expect("转换输出数据失败");
            let expected_data: Vec<f32> = expected_output.iter().copied().collect();
            
            // 计算误差
            let mut max_error: f32 = 0.0;
            let mut mean_error: f32 = 0.0;
            let len = output_data.len();
            
            for (i, (out, exp)) in output_data.iter().zip(expected_data.iter()).enumerate() {
                let error: f32 = (out - exp).abs();
                mean_error += error;
                if error > max_error {
                    max_error = error;
                }
                
                // 打印前5个元素的对比
                if i < 5 {
                    println!("  [{}]: 输出={:.6}, 预期={:.6}, 误差={:.6}", i, out, exp, error);
                }
            }
            mean_error /= len as f32;
            
            println!("输出对比结果:");
            println!("  输出元素数: {}", len);
            println!("  最大误差: {:.6}", max_error);
            println!("  平均误差: {:.6}", mean_error);
            
            // 断言误差在可接受范围内（考虑到浮点精度）
            assert!(max_error < 1e-3, "{} 最大误差过大: {}", model_name, max_error);
            println!("  ✓ 输出验证通过");
        } else {
            panic!("{} 未找到输出数据", model_name);
        }

        // 卸载模型
        runtime.Unload_Model();
        println!("{} 模型已卸载", model_name);
    }

    println!("\n========================================");
    println!("所有模型测试完成");
    println!("========================================");
}

/// 测试切分模型推理 - 分别加载 part1 和 part2，验证中间结果和最终结果
#[test]
pub fn Test_Split_Model() {
    // 初始化 ONNX Runtime
    let init_config = RuntimeInitConfig {
        device: "cuda".to_string(),
    };
    Init_ONNX(init_config).expect("Init_ONNX 失败");

    // 读取测试数据
    let test_data_path = "tests/test_models/test_data.npz";
    let file = File::open(test_data_path).expect("无法打开 test_data.npz");
    let mut npz = NpzReader::new(file).expect("无法读取 NPZ 文件");

    // 定义要测试的切分模型 (模型名, 键前缀, part1路径, part2路径)
    let models = vec![
        ("mlp", "mlp", "tests/test_models/mlp_part1.onnx", "tests/test_models/mlp_part2.onnx"),
        ("vgg11", "vgg11", "tests/test_models/vgg11_part1.onnx", "tests/test_models/vgg11_part2.onnx"),
        ("deit_tiny", "deit", "tests/test_models/deit_tiny_part1.onnx", "tests/test_models/deit_tiny_part2.onnx"),
    ];

    for (model_name, key_prefix, part1_path, part2_path) in models {
        println!("\n========================================");
        println!("测试切分模型: {}", model_name);
        println!("========================================");

        // 创建两个 Runtime
        let config = RuntimeConfig::default();
        let mut runtime_part1 = Runtime::new(config.clone()).expect("创建 Runtime Part1 失败");
        let mut runtime_part2 = Runtime::new(config.clone()).expect("创建 Runtime Part2 失败");

        // 加载 Part1 模型
        let model_info1 = runtime_part1.Load_Model(part1_path).expect(&format!("加载 {} Part1 失败", model_name));
        println!("Part1 模型加载成功: {}", model_info1.path);

        // 加载 Part2 模型
        let model_info2 = runtime_part2.Load_Model(part2_path).expect(&format!("加载 {} Part2 失败", model_name));
        println!("Part2 模型加载成功: {}", model_info2.path);

        // 读取输入数据
        let input_key = format!("{}_input", key_prefix);
        let input_array = Read_Npz_Array(&mut npz, &input_key)
            .expect(&format!("无法读取 {} 输入数据", input_key));
        println!("输入数据形状: {:?}", input_array.shape());

        // 创建输入 TensorPacket
        let input_shape: Vec<usize> = input_array.shape().to_vec();
        let input_data: Vec<f32> = input_array.iter().copied().collect();
        let input_packet = TensorPacket::From_F32_Slice(
            "input".to_string(),
            input_shape,
            &input_data,
        ).expect("创建输入 TensorPacket 失败");

        // 执行 Part1 推理
        let mut inputs1 = HashMap::new();
        inputs1.insert("input".to_string(), input_packet);
        
        let intermediate_outputs = runtime_part1.Execute_From_Packets(inputs1)
            .expect(&format!("{} Part1 推理失败", model_name));
        
        println!("Part1 推理完成，中间输出数量: {}", intermediate_outputs.len());

        // 读取预期中间结果
        let intermediate_key = format!("{}_intermediate", key_prefix);
        let expected_intermediate = Read_Npz_Array(&mut npz, &intermediate_key)
            .expect(&format!("无法读取 {} 预期中间结果", intermediate_key));
        println!("预期中间结果形状: {:?}", expected_intermediate.shape());

        // 验证中间结果
        let mut intermediate_packet = None;
        if let Some(output_packet) = intermediate_outputs.get("output") {
            let intermediate_data = output_packet.To_F32_Vec().expect("转换中间结果失败");
            let expected_data: Vec<f32> = expected_intermediate.iter().copied().collect();
            
            // 计算中间结果误差
            let mut max_error: f32 = 0.0;
            for (out, exp) in intermediate_data.iter().zip(expected_data.iter()) {
                let error: f32 = (out - exp).abs();
                if error > max_error {
                    max_error = error;
                }
            }
            
            println!("中间结果验证:");
            println!("  最大误差: {:.6}", max_error);
            assert!(max_error < 1e-3, "{} 中间结果最大误差过大: {}", model_name, max_error);
            println!("  ✓ 中间结果验证通过");
            
            // 保存中间结果用于 Part2
            intermediate_packet = Some(output_packet.clone());
        } else {
            panic!("{} Part1 未找到输出数据", model_name);
        }

        // 使用中间结果作为 Part2 的输入
        if let Some(packet) = intermediate_packet {
            let mut inputs2 = HashMap::new();
            // 注意：part2 的输入名称可能是 "input" 或 "intermediate"，根据模型而定
            inputs2.insert("input".to_string(), packet);
            
            let final_outputs = runtime_part2.Execute_From_Packets(inputs2)
                .expect(&format!("{} Part2 推理失败", model_name));
            
            println!("Part2 推理完成，最终输出数量: {}", final_outputs.len());

            // 读取预期最终结果
            let split_output_key = format!("{}_split_output", key_prefix);
            let expected_final = Read_Npz_Array(&mut npz, &split_output_key)
                .expect(&format!("无法读取 {} 预期切分输出", split_output_key));
            println!("预期最终结果形状: {:?}", expected_final.shape());

            // 验证最终结果
            if let Some(output_packet) = final_outputs.get("output") {
                let final_data = output_packet.To_F32_Vec().expect("转换最终结果失败");
                let expected_data: Vec<f32> = expected_final.iter().copied().collect();
                
                // 计算最终结果误差
                let mut max_error: f32 = 0.0;
                let mut mean_error: f32 = 0.0;
                let len = final_data.len();
                
                for (i, (out, exp)) in final_data.iter().zip(expected_data.iter()).enumerate() {
                    let error: f32 = (out - exp).abs();
                    mean_error += error;
                    if error > max_error {
                        max_error = error;
                    }
                    
                    // 打印前5个元素的对比
                    if i < 5 {
                        println!("  [{}]: 输出={:.6}, 预期={:.6}, 误差={:.6}", i, out, exp, error);
                    }
                }
                mean_error /= len as f32;
                
                println!("最终结果验证:");
                println!("  输出元素数: {}", len);
                println!("  最大误差: {:.6}", max_error);
                println!("  平均误差: {:.6}", mean_error);
                
                assert!(max_error < 1e-3, "{} 最终结果最大误差过大: {}", model_name, max_error);
                println!("  ✓ 最终结果验证通过");
            } else {
                panic!("{} Part2 未找到输出数据", model_name);
            }
        }

        // 卸载模型
        runtime_part1.Unload_Model();
        runtime_part2.Unload_Model();
        println!("{} 切分模型已卸载", model_name);
    }

    println!("\n========================================");
    println!("所有切分模型测试完成");
    println!("========================================");
}

/// 测试 CUDA GPU 推理性能 - 循环运行 1000 次推理
#[test]
pub fn Test_CUDA() {
    use std::time::Instant;
    
    println!("========================================");
    println!("CUDA GPU 推理测试");
    println!("========================================");
    
    // 配置 CUDA 环境
    let init_config = RuntimeInitConfig {
        device: "cpu".to_string(),
    };
    Init_ONNX(init_config).expect("Init_ONNX 失败");
    

    
    // 创建 Runtime
    let config = RuntimeConfig::default();
    let mut runtime = Runtime::new(config).expect("创建 Runtime 失败");
    
    // 加载 deit_tiny 模型
    let model_path = "tests/test_models/vgg16.onnx";
    let model_info = runtime.Load_Model(model_path).expect("加载 deit_tiny 模型失败");
    println!("[加载] 模型: {}", model_info.path);
    
    // 初始化输入数据 (1, 3, 224, 224)
    let input_shape = vec![1, 3, 224, 224];
    let input_data: Vec<f32> = vec![0.5; 1 * 3 * 224 * 224]; // 使用固定值填充
    let input_packet = TensorPacket::From_F32_Slice(
        "input".to_string(),
        input_shape,
        &input_data,
    ).expect("创建输入 TensorPacket 失败");
    
    println!("[输入] 形状: {:?}", input_data.len());
    println!("[开始] 循环推理 1000 次...");
    
    // 预热 (前10次不计时)
    let mut inputs = HashMap::new();
    inputs.insert("input".to_string(), input_packet.clone());
    for _ in 0..10 {
        let _ = runtime.Execute_From_Packets(inputs.clone()).expect("预热推理失败");
    }
    
    // 正式测试：循环 1000 次
    let start_time = Instant::now();
    let mut total_iterations: usize = 0;
    
    for i in 0..1000 {
        let iter_start = Instant::now();
        let outputs = runtime.Execute_From_Packets(inputs.clone()).expect(&format!("第 {} 次推理失败", i));
        let iter_duration = iter_start.elapsed();
        
        total_iterations += 1;
        
        // 每 100 次打印一次进度
        if (i + 1) % 100 == 0 {
            println!("[进度] 已完成 {}/1000 次推理, 本次耗时: {:?}", i + 1, iter_duration);
        }
        
        // 验证输出
        if let Some(output_packet) = outputs.get("output") {
            let _output_data = output_packet.To_F32_Vec().expect("转换输出数据失败");
            // 可以在这里添加输出验证
        }
    }
    
    let total_duration = start_time.elapsed();
    let avg_duration = total_duration / 1000;
    
    println!("\n========================================");
    println!("CUDA 推理测试完成");
    println!("========================================");
    println!("总推理次数: {}", total_iterations);
    println!("总耗时: {:?}", total_duration);
    println!("平均每次推理耗时: {:?}", avg_duration);
    println!("吞吐量: {:.2} 次/秒", 1000.0 / total_duration.as_secs_f64());
    
    // 卸载模型
    runtime.Unload_Model();
    println!("[卸载] 模型已卸载");
    
    println!("========================================");
}

/// 测试 Qwen3 完整模型加载和输入输出信息展示
///
/// 此函数完成如下测试：
/// 1. 加载Qwen3模型
/// 2. 输出输入包含的内容
/// 3. 硬编码token id作为输入，然后运行模型得到输出结果
/// 4. 将输出结果打印出来
/// 5. 卸载模型
#[test]
// pub fn Test_Qwen3_Full_Model() {
//     use std::path::Path;
    
//     println!("========================================");
//     println!("Qwen3 完整模型测试");
//     println!("========================================");
    
//     // 检查模型文件是否存在（实际路径: tests/test_models/qwen3-0.6b/onnx/）
//     let model_path = "tests/test_models/qwen3-0.6b/onnx/model.onnx";
//     let fp16_model_path = "tests/test_models/qwen3-0.6b/onnx/model_fp16.onnx";
    
//     let actual_path = if Path::new(model_path).exists() {
//         model_path
//     } else if Path::new(fp16_model_path).exists() {
//         fp16_model_path
//     } else {
//         println!("[跳过] Qwen3 模型文件不存在");
//         println!("预期路径: {}", model_path);
//         println!("请从 https://www.modelscope.cn/models/onnx-community/Qwen3-0.6B-ONNX 下载模型");
//         return;
//     };
    
//     println!("[信息] 找到模型文件: {}", actual_path);
    
//     // 初始化 ONNX Runtime
//     let init_config = RuntimeInitConfig {
//         device: "cpu".to_string(),
//     };
//     Init_ONNX(init_config).expect("Init_ONNX 失败");
//     println!("[初始化] ONNX Runtime 初始化成功");
    
//     // 创建 Runtime
//     let config = RuntimeConfig::default();
//     let mut runtime = Runtime::new(config).expect("创建 Runtime 失败");
    
//     // 1. 加载 Qwen3 模型
//     println!("[加载] 开始加载 Qwen3 模型...");
//     let model_info = runtime.Load_Model(actual_path).expect("加载 Qwen3 模型失败");
//     println!("[成功] 模型加载成功: {}", model_info.path);
    
//     // 2. 输出输入包含的内容
//     println!("\n[输入信息]");
//     println!("  模型类型: Qwen3-0.6B (Decoder-only Transformer)");
//     println!("  预期输入: input_ids (Int64), attention_mask (Int64)");
//     println!("  预期输出: logits (Float32)");
    
//     // 3. 硬编码 token id 作为输入（"你好！"的 token IDs）
//     // 通过 token_extract.py 获取: [108386, 6313]
//     let input_ids: Vec<i64> = vec![108386, 6313];
//     let seq_len: usize = input_ids.len();
//     println!("\n[硬编码输入]");
//     println!("  文本: '你好！'");
//     println!("  Token IDs: {:?}", input_ids);
//     println!("  序列长度: {}", seq_len);
    
//     // 4. 运行模型得到输出结果
//     // 直接使用 Execute 方法，创建所有必需的 Int64 输入张量
//     println!("\n[推理] 开始执行模型推理...");
//     println!("[信息] Qwen3 模型需要 3 个必需输入: input_ids, attention_mask, position_ids");
    
//     let input_shape: Vec<i64> = vec![1, seq_len as i64];
    
//     // input_ids: [108386, 6313] ("你好！")
//     let input_ids_data: Vec<i64> = input_ids.clone();
//     let input_ids_tensor = Tensor::from_array((input_shape.clone(), input_ids_data))
//         .expect("创建 input_ids 张量失败");
    
//     // attention_mask: [1, 1] (全部有效)
//     let attention_mask_data: Vec<i64> = vec![1; seq_len];
//     let attention_mask_tensor = Tensor::from_array((input_shape.clone(), attention_mask_data))
//         .expect("创建 attention_mask 张量失败");
    
//     // position_ids: [0, 1] (位置编码)
//     let position_ids_data: Vec<i64> = (0..seq_len as i64).collect();
//     let position_ids_tensor = Tensor::from_array((input_shape.clone(), position_ids_data))
//         .expect("创建 position_ids 张量失败");
    
//     println!("[输入] 创建张量成功:");
//     println!("       input_ids:    形状 [1, {}], 数据 {:?}", seq_len, input_ids);
//     println!("       attention_mask: 形状 [1, {}], 数据 {:?}", seq_len, vec![1; seq_len]);
//     println!("       position_ids:   形状 [1, {}], 数据 {:?}", seq_len, (0..seq_len as i64).collect::<Vec<i64>>());
    
//     // 构建输入 HashMap
//     let mut inputs: HashMap<String, DynValue> = HashMap::new();
//     inputs.insert("input_ids".to_string(), input_ids_tensor.into_dyn());
//     inputs.insert("attention_mask".to_string(), attention_mask_tensor.into_dyn());
//     inputs.insert("position_ids".to_string(), position_ids_tensor.into_dyn());
    
//     // 注意: ONNX Runtime 不允许创建维度为 0 的张量
//     // 对于第一次推理，不提供 past_key_values，让模型使用默认初始化
//     println!("[输入] 已添加 3 个必需输入 (input_ids, attention_mask, position_ids)");
//     println!("[注意] 不提供 past_key_values，模型将使用默认初始化");
    
//     // 执行推理
//     match runtime.Execute(inputs) {
//         Ok(outputs) => {
//             println!("[成功] 推理完成，输出数量: {}", outputs.len());
            
//             // 打印输出结果
//             println!("\n[输出结果]");
//             for (name, dyn_value) in &outputs {
//                 println!("  输出名: {}", name);
                
//                 // 尝试提取为 Float32 张量
//                 match dyn_value.try_extract_tensor::<f32>() {
//                     Ok((shape_ref, data_slice)) => {
//                         let shape: Vec<i64> = shape_ref.iter().map(|&s| s).collect();
//                         let data_len = data_slice.len();
//                         let preview_len = data_len.min(10);
                        
//                         println!("    形状: {:?}", shape);
//                         println!("    数据类型: Float32");
//                         println!("    总元素数: {}", data_len);
//                         println!("    数据预览 (前{}个): {:?}", preview_len, &data_slice[..preview_len]);
//                     }
//                     Err(e) => {
//                         println!("    无法提取为 Float32: {}", e);
//                     }
//                 }
//             }
//         }
//         Err(e) => {
//             println!("[错误] 推理失败: {:?}", e);
//         }
//     }
    
//     // 5. 卸载模型
//     println!("\n[卸载] 正在卸载模型...");
//     runtime.Unload_Model();
//     println!("[成功] 模型已卸载");
    
//     println!("\n========================================");
//     println!("Qwen3 完整模型测试完成");
//     println!("========================================");
// }


pub fn Test_Qwen3_Full_Model() {
    use std::path::Path;
    use ndarray::Array4;  // 用于创建 4 维数组
    
    println!("========================================");
    println!("Qwen3 完整模型测试");
    println!("========================================");
    
    let model_path = "tests/test_models/qwen3-0.6b/onnx/model.onnx";
    let fp16_model_path = "tests/test_models/qwen3-0.6b/onnx/model_fp16.onnx";
    
    let actual_path = if Path::new(model_path).exists() {
        model_path
    } else if Path::new(fp16_model_path).exists() {
        fp16_model_path
    } else {
        println!("[跳过] Qwen3 模型文件不存在");
        return;
    };
    
    println!("[信息] 找到模型文件: {}", actual_path);
    
    let init_config = RuntimeInitConfig {
        device: "cpu".to_string(),
    };
    Init_ONNX(init_config).expect("Init_ONNX 失败");
    println!("[初始化] ONNX Runtime 初始化成功");
    
    let config = RuntimeConfig::default();
    let mut runtime = Runtime::new(config).expect("创建 Runtime 失败");
    
    println!("[加载] 开始加载 Qwen3 模型...");
    let model_info = runtime.Load_Model(actual_path).expect("加载 Qwen3 模型失败");
    println!("[成功] 模型加载成功: {}", model_info.path);
    
    // 硬编码 token id 作为输入（"你好！"的 token IDs）
    let input_ids: Vec<i64> = vec![108386, 6313];
    let seq_len: usize = input_ids.len();
    println!("\n[硬编码输入] 文本: '你好！' Token IDs: {:?}", input_ids);
    
    // 创建基础输入
    let input_shape: Vec<i64> = vec![1, seq_len as i64];
    
    let input_ids_tensor = Tensor::from_array((input_shape.clone(), input_ids.clone()))
        .expect("创建 input_ids 张量失败");
    
    let attention_mask_data: Vec<i64> = vec![1; seq_len];
    let attention_mask_tensor = Tensor::from_array((input_shape.clone(), attention_mask_data))
        .expect("创建 attention_mask 张量失败");
    
    let position_ids_data: Vec<i64> = (0..seq_len as i64).collect();
    let position_ids_tensor = Tensor::from_array((input_shape.clone(), position_ids_data))
        .expect("创建 position_ids 张量失败");
    
    let mut inputs: HashMap<String, DynValue> = HashMap::new();
    inputs.insert("input_ids".to_string(), input_ids_tensor.into_dyn());
    inputs.insert("attention_mask".to_string(), attention_mask_tensor.into_dyn());
    inputs.insert("position_ids".to_string(), position_ids_tensor.into_dyn());
    
    // 关键修改：使用 ndarray 创建空的 past_key_values
    let num_layers = 28;
    let batch = 1usize;
    let num_kv_heads = 4usize;
    let head_dim = 64usize;
    let seq_len_zero = 0usize;  // 空的序列长度
    
    println!("[输入] 创建 {} 层空的 KV-Cache...", num_layers);
    
    for layer in 0..num_layers {
        // 创建 key: 形状 [1, 4, 0, 64] 的零数组
        // Array4::zeros 接受 (batch, num_kv_heads, seq_len, head_dim)
        let key_array = Array4::<f32>::zeros((batch, num_kv_heads, seq_len_zero, head_dim));
        let key_value = ort::Value::from_array(key_array)
            .expect(&format!("创建 past_key_values.{}.key 失败", layer));
        
        // 创建 value: 同样形状
        let val_array = Array4::<f32>::zeros((batch, num_kv_heads, seq_len_zero, head_dim));
        let val_value = ort::Value::from_array(val_array)
            .expect(&format!("创建 past_key_values.{}.value 失败", layer));
        
        inputs.insert(format!("past_key_values.{}.key", layer), key_value);
        inputs.insert(format!("past_key_values.{}.value", layer), val_value);
    }
    
    println!("[输入] 总计 {} 个输入张量", inputs.len());
    
    // 执行推理
    match runtime.Execute(inputs) {
        Ok(outputs) => {
            println!("[成功] 推理完成，输出数量: {}", outputs.len());
            for (name, dyn_value) in &outputs {
                println!("  输出名: {}", name);
                if let Ok((shape_ref, data)) = dyn_value.try_extract_tensor::<f32>() {
                    let shape: Vec<i64> = shape_ref.iter().collect();
                    println!("    形状: {:?}, 元素数: {}", shape, data.len());
                }
            }
        }
        Err(e) => {
            println!("[错误] 推理失败: {:?}", e);
        }
    }
    
    runtime.Unload_Model();
    println!("\n========================================");
    println!("Qwen3 完整模型测试完成");
    println!("========================================");
}