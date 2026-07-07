//Presented by KeJi
//Date : 2026-03-30

// gguf model manager类
// 此类的作用包括
// - 读取目标gguf文件，便于上层分析传入的gguf文件具体对应什么模型，以及当前gguf文件包含哪些层（我们之后会修改gguf文件格式，让它仅包含部分层）
// - 加载模型，提取gguf文件当中的特定层的权重并将其返回
// - 切分gguf模型，按照需要，对gguf文件模型进行切分。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use anyhow::Result;
use candle_core::quantized::gguf_file;
use candle_core::quantized::QTensor;
use candle_core::Device;
use std::collections::HashMap;
use std::io::{BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::hash::Hasher;
use twox_hash::XxHash32;

// ============================================================
// 数据结构定义
// ============================================================

/// 模型整体架构信息
pub struct Model_Arch_Info {
    pub architecture: String,
    pub num_layers: usize,
    pub embedding_length: usize,
    pub head_count: usize,
    pub head_count_kv: usize,
    pub head_dim: usize,
    pub feed_forward_length: usize,
    pub context_length: usize,
    pub rms_norm_eps: f64,
    pub rope_freq_base: f64,
    pub vocab_size: usize,
    /// EOS token ID，从 GGUF metadata 的 tokenizer.ggml.eos_token_id 读取
    pub eos_token_id: u32,
    /// chat template，从 GGUF metadata 的 tokenizer.chat_template 读取
    pub chat_template: Option<String>,
    pub layers: Vec<Layer_Info>,
    pub non_layer_tensors: Vec<Tensor_Detail>,
    pub metadata_raw: HashMap<String, String>,
    /// 此模型是否是切分过的模型
    pub is_split: bool,
    /// 切分的开始层（仅当 is_split 为 true 时有效）
    pub split_start: usize,
    /// 切分的结束层（仅当 is_split 为 true 时有效）
    pub split_end: usize,
    /// 模型唯一标识 — xxhash32(原始文件字节)，PGGUF 从 metadata 读取
    pub model_id: Option<u32>,
    /// 256 位层位图，bit N=1 表示文件含第 N 层
    pub layer_bitmap: Option<[u8; 32]>,
    /// 权重格式标记: None = 标准 GGUF 量化, Some("safetensors") = safetensors 原样打包
    /// （由 Python --wrap-native 模式写入，DeepSeek V4 使用）
    pub weight_format: Option<String>,
    /// safetensors 分片数量（仅 weight_format == "safetensors" 时有效）
    pub num_shards: Option<usize>,
}

/// 每层的信息
pub struct Layer_Info {
    pub layer_index: usize,
    pub tensors: Vec<Tensor_Detail>,
    pub total_size_bytes: usize,
}

/// 单个 tensor 的详细信息
pub struct Tensor_Detail {
    pub name: String,
    pub shape: Vec<usize>,
    pub dtype: String,
    pub size_bytes: usize,
}

/// 提取出的层权重
pub struct GGUF_Layer_Weights {
    pub layer_index: usize,
    pub tensors: HashMap<String, QTensor>,
}

// ============================================================
// 辅助函数
// ============================================================

/// 从 metadata HashMap 中安全读取 u32/u64 类值并返回 usize（公开 API）
pub fn Get_Metadata_Usize_From_Map(metadata: &HashMap<String, gguf_file::Value>, key: &str) -> Option<usize> {
    Get_Metadata_Usize(metadata, key)
}

/// 从 metadata 中安全读取 u32/u64 类值并返回 usize
fn Get_Metadata_Usize(metadata: &HashMap<String, gguf_file::Value>, key: &str) -> Option<usize> {
    if let Some(val) = metadata.get(key) {
        if let Ok(v) = val.to_u32() {
            return Some(v as usize);
        }
        if let Ok(v) = val.to_u64() {
            return Some(v as usize);
        }
    }
    None
}

/// 从 metadata 中安全读取字符串
fn Get_Metadata_String(metadata: &HashMap<String, gguf_file::Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|v| v.to_string().ok().map(|s| s.clone()))
}

/// 从 metadata 中安全读取 f32/f64 类值并返回 f64
fn Get_Metadata_F64(metadata: &HashMap<String, gguf_file::Value>, key: &str) -> Option<f64> {
    if let Some(val) = metadata.get(key) {
        if let Ok(v) = val.to_f32() {
            return Some(v as f64);
        }
        if let Ok(v) = val.to_f64() {
            return Some(v);
        }
    }
    None
}

/// 将 Value 转为可阅读的字符串（用于原始 metadata 记录）
fn Value_To_Display_String(val: &gguf_file::Value) -> String {
    match val {
        gguf_file::Value::U8(v) => format!("{}", v),
        gguf_file::Value::I8(v) => format!("{}", v),
        gguf_file::Value::U16(v) => format!("{}", v),
        gguf_file::Value::I16(v) => format!("{}", v),
        gguf_file::Value::U32(v) => format!("{}", v),
        gguf_file::Value::I32(v) => format!("{}", v),
        gguf_file::Value::U64(v) => format!("{}", v),
        gguf_file::Value::I64(v) => format!("{}", v),
        gguf_file::Value::F32(v) => format!("{}", v),
        gguf_file::Value::F64(v) => format!("{}", v),
        gguf_file::Value::Bool(v) => format!("{}", v),
        gguf_file::Value::String(v) => v.clone(),
        gguf_file::Value::Array(arr) => format!("[array, len={}]", arr.len()),
    }
}

/// 计算单个 tensor 的字节大小
fn Calc_Tensor_Size_Bytes(info: &gguf_file::TensorInfo) -> usize {
    let elem_count = info.shape.elem_count();
    let block_size = info.ggml_dtype.block_size();
    if block_size == 0 {
        return 0;
    }
    elem_count * info.ggml_dtype.type_size() / block_size
}

// ============================================================
// 核心公开函数
// ============================================================


/// 从已加载的 GGUF Content 中提取模型架构信息（零 I/O）。
///
/// 与 `GGUF_Analyze` 功能相同，但不打开文件也不解析 header——
/// 直接使用调用方已加载的 Content，避免重复打开文件。
pub fn GGUF_Analyze_From_Content(content: &gguf_file::Content) -> Result<Model_Arch_Info> {
    let architecture = Get_Metadata_String(&content.metadata, "general.architecture")
        .unwrap_or_else(|| "unknown".to_string());

    let num_layers = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.block_count", architecture),
    )
    .unwrap_or(0);

    let embedding_length = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.embedding_length", architecture),
    )
    .unwrap_or(0);

    let head_count = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.attention.head_count", architecture),
    )
    .unwrap_or(0);

    let head_count_kv = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.attention.head_count_kv", architecture),
    )
    .unwrap_or(0);

    let feed_forward_length = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.feed_forward_length", architecture),
    )
    .unwrap_or(0);

    let head_dim = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.attention.key_length", architecture),
    )
    .unwrap_or_else(|| {
        if head_count > 0 {
            embedding_length / head_count
        } else {
            0
        }
    });

    let context_length = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.context_length", architecture),
    )
    .unwrap_or(2048);

    let rms_norm_eps = Get_Metadata_F64(
        &content.metadata,
        &format!("{}.attention.layer_norm_rms_epsilon", architecture),
    )
    .unwrap_or(1e-6);

    let rope_freq_base = Get_Metadata_F64(
        &content.metadata,
        &format!("{}.rope.freq_base", architecture),
    )
    .unwrap_or(10000.0);

    let vocab_size = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.vocab_size", architecture),
    )
    .unwrap_or_else(|| {
        Get_Metadata_Usize(&content.metadata, "tokenizer.ggml.tokens").unwrap_or(0)
    });

    let eos_token_id = Get_Metadata_Usize(&content.metadata, "tokenizer.ggml.eos_token_id")
        .map(|v| v as u32)
        .unwrap_or(0);

    let chat_template = Get_Metadata_String(&content.metadata, "tokenizer.chat_template");

    let mut layer_tensors_map: HashMap<usize, Vec<Tensor_Detail>> = HashMap::new();
    let mut non_layer_tensors: Vec<Tensor_Detail> = Vec::new();

    for (tensor_name, tensor_info) in &content.tensor_infos {
        let detail = Tensor_Detail {
            name: tensor_name.clone(),
            shape: tensor_info.shape.dims().to_vec(),
            dtype: format!("{:?}", tensor_info.ggml_dtype),
            size_bytes: Calc_Tensor_Size_Bytes(tensor_info),
        };

        if tensor_name.starts_with("blk.") {
            let parts: Vec<&str> = tensor_name.splitn(3, '.').collect();
            if parts.len() >= 2 {
                if let Ok(layer_idx) = parts[1].parse::<usize>() {
                    layer_tensors_map
                        .entry(layer_idx)
                        .or_default()
                        .push(detail);
                    continue;
                }
            }
        }

        non_layer_tensors.push(detail);
    }

    let split_start_val = Get_Metadata_Usize(&content.metadata, "pleiades.split.start");
    let split_end_val = Get_Metadata_Usize(&content.metadata, "pleiades.split.end");
    let is_split = split_start_val.is_some() && split_end_val.is_some();
    let split_start = split_start_val.unwrap_or(0);
    let split_end = split_end_val.unwrap_or(0);

    let mut layers: Vec<Layer_Info> = Vec::with_capacity(num_layers);
    for i in 0..num_layers {
        let layer_idx = i;
        let mut tensors = layer_tensors_map.remove(&layer_idx).unwrap_or_default();
        tensors.sort_by(|a, b| a.name.cmp(&b.name));
        let total_size_bytes = tensors.iter().map(|t| t.size_bytes).sum();

        layers.push(Layer_Info {
            layer_index: layer_idx,
            tensors,
            total_size_bytes,
        });
    }

    non_layer_tensors.sort_by(|a, b| a.name.cmp(&b.name));

    let mut metadata_raw: HashMap<String, String> = HashMap::new();
    for (key, val) in &content.metadata {
        metadata_raw.insert(key.clone(), Value_To_Display_String(val));
    }

    Ok(Model_Arch_Info {
        architecture,
        num_layers,
        embedding_length,
        head_count,
        head_count_kv,
        head_dim,
        feed_forward_length,
        context_length,
        rms_norm_eps,
        rope_freq_base,
        vocab_size,
        eos_token_id,
        chat_template: chat_template.clone(),
        layers,
        non_layer_tensors,
        metadata_raw,
        is_split,
        split_start,
        split_end,
        model_id: None,
        layer_bitmap: None,
        weight_format: Get_Metadata_String(&content.metadata, "pleiades.weight_format"),
        num_shards: Get_Metadata_Usize(&content.metadata, "pleiades.num_shards"),
    })
}

// ============================================================
// GGUF_Analyze_And_Convert — 分析 + GGUF→PGGUF 自动转换
// ============================================================

/// 从 layer_tensors_map 构建 256 位层位图。
///
/// layer_tensors_map: key=blk 索引 (0..num_layers-1)，value=该层的 tensor 列表。
/// 只要 key 存在即表示该层存在 → bit 置 1。
fn Build_Layer_Bitmap(
    layer_tensors_map: &HashMap<usize, Vec<Tensor_Detail>>,
    num_layers: usize,
) -> [u8; 32] {
    let mut bitmap = [0u8; 32];
    for i in 0..num_layers {
        if layer_tensors_map.contains_key(&i) {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            if byte_idx < 32 {
                bitmap[byte_idx] |= 1 << bit_idx;
            }
        }
    }
    bitmap
}

/// 解析 GGUF / PGGUF 文件，返回架构元信息。
///
/// 如果是原始 GGUF（metadata 无 pleiades.model_id）：
///   1. 读全量字节 → xxhash32 → model_id
///   2. 从 tensor 信息构建 layer_bitmap
///   3. 写入 .pgguf 文件（metadata 追加 model_id + layer_bitmap）
///   4. 删除原始 .gguf → 重命名 .pgguf 覆盖原后缀
///
/// 如果已是 PGGUF（metadata 有 pleiades.model_id）：
///   直接读取 model_id + layer_bitmap，零额外 I/O。
pub fn GGUF_Analyze_And_Convert(gguf_file_path: &Path) -> Result<(Model_Arch_Info, PathBuf)> {
    // 1. 打开文件并解析 GGUF Content
    let mut file = std::fs::File::open(gguf_file_path)?;
    let content = gguf_file::Content::read(&mut file)
        .map_err(|e| anyhow::anyhow!("Failed to read GGUF content: {}", e))?;

    // 2. 从 Content 提取架构信息（零 I/O）
    let mut arch_info = GGUF_Analyze_From_Content(&content)?;

    // 2.5 如果是 safetensors 原样打包模式，跳过 GGUF→PGGUF 转换
    if arch_info.weight_format.as_deref() == Some("safetensors") {
        arch_info.model_id = Get_Metadata_Usize(&content.metadata, "pleiades.model_id")
            .map(|id| id as u32);
        arch_info.layer_bitmap = Get_Metadata_String(&content.metadata, "pleiades.layer_bitmap")
            .map(|hex| Parse_Bitmap_Hex(&hex));
        return Ok((arch_info, gguf_file_path.to_path_buf()));
    }

    // 3. 检查是否已是 PGGUF（metadata 中有 pleiades.model_id）
    let existing_model_id = Get_Metadata_Usize(&content.metadata, "pleiades.model_id");
    let existing_bitmap = Get_Metadata_String(&content.metadata, "pleiades.layer_bitmap");

    if let (Some(id), Some(hex_bitmap)) = (existing_model_id, existing_bitmap) {
        // 已是 PGGUF → 直接读取，零 I/O
        arch_info.model_id = Some(id as u32);
        arch_info.layer_bitmap = Some(Parse_Bitmap_Hex(&hex_bitmap));
        return Ok((arch_info, gguf_file_path.to_path_buf()));
    }

    // 4. 原始 GGUF → 计算 model_id (从 tensor 元数据, 不读全文件)
    let mut hasher = XxHash32::with_seed(0);
    let mut sorted_names: Vec<&String> = content.tensor_infos.keys().collect();
    sorted_names.sort();
    for name in &sorted_names {
        hasher.write(name.as_bytes());
        let info = &content.tensor_infos[*name];
        // Hash shape dims to make model_id unique per architecture
        for dim in info.shape.dims() {
            hasher.write(&(*dim as u64).to_le_bytes());
        }
    }
    let model_id = hasher.finish() as u32;

    // 5. 构建 layer_bitmap
    let mut layer_tensors_map: HashMap<usize, Vec<Tensor_Detail>> = HashMap::new();
    for (tensor_name, _tensor_info) in &content.tensor_infos {
        if tensor_name.starts_with("blk.") {
            let parts: Vec<&str> = tensor_name.splitn(3, '.').collect();
            if parts.len() >= 2 {
                if let Ok(layer_idx) = parts[1].parse::<usize>() {
                    layer_tensors_map.entry(layer_idx).or_default();
                }
            }
        }
    }
    let layer_bitmap = Build_Layer_Bitmap(&layer_tensors_map, arch_info.num_layers);

    // 6. 写入 PGGUF：加载全部 tensor → gguf_file::write()
    let pgguf_path = gguf_file_path.with_extension("pgguf");
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    tracing::info!(
        "GGUF → PGGUF 转换开始: {} ({:.1} GB)",
        gguf_file_path.display(),
        file_size as f64 / 1e9
    );

    let device = Device::Cpu;
    let mut loaded_tensors: Vec<(String, QTensor)> = Vec::new();
    for tensor_name in content.tensor_infos.keys() {
        let qtensor = content
            .tensor(&mut file, tensor_name, &device)
            .map_err(|e| anyhow::anyhow!("Failed to load tensor '{}': {}", tensor_name, e))?;
        loaded_tensors.push((tensor_name.clone(), qtensor));
    }

    let mut metadata_pairs: Vec<(String, gguf_file::Value)> = content
        .metadata
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    metadata_pairs.push((
        "pleiades.model_id".to_string(),
        gguf_file::Value::U32(model_id),
    ));
    metadata_pairs.push((
        "pleiades.layer_bitmap".to_string(),
        gguf_file::Value::String(Bitmap_To_Hex(&layer_bitmap)),
    ));
    metadata_pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let metadata_refs: Vec<(&str, &gguf_file::Value)> = metadata_pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v))
        .collect();
    let tensor_refs: Vec<(&str, &QTensor)> = loaded_tensors
        .iter()
        .map(|(n, t)| (n.as_str(), t))
        .collect();

    let out_file = std::fs::File::create(&pgguf_path)?;
    let mut writer = BufWriter::new(out_file);
    gguf_file::write(&mut writer, &metadata_refs, &tensor_refs)
        .map_err(|e| anyhow::anyhow!("Failed to write PGGUF file: {}", e))?;
    drop(writer);

    let output_size = std::fs::metadata(&pgguf_path).map(|m| m.len()).unwrap_or(0);
    tracing::info!(
        "GGUF → PGGUF 转换完成: {} ({:.1} GB)",
        pgguf_path.display(),
        output_size as f64 / 1e9
    );

    // 7. 删除原始 .gguf
    drop(file);
    std::fs::remove_file(gguf_file_path)
        .map_err(|e| anyhow::anyhow!("Failed to remove original GGUF: {}", e))?;

    arch_info.model_id = Some(model_id);
    arch_info.layer_bitmap = Some(layer_bitmap);
    Ok((arch_info, pgguf_path))
}


/// 根据模型名称在 workspace 中查找模型文件。
///
/// 优先 `.pgguf`，回退 `.gguf`。
pub fn Resolve_Model_Path(workspace: &Path, model_name: &str) -> Result<PathBuf> {
    let pgguf = workspace.join(format!("{}.pgguf", model_name));
    if pgguf.exists() {
        return Ok(pgguf);
    }
    let gguf = workspace.join(format!("{}.gguf", model_name));
    if gguf.exists() {
        return Ok(gguf);
    }
    anyhow::bail!(
        "Model file not found: '{}.pgguf' or '{}.gguf' in {}",
        model_name, model_name, workspace.display()
    )
}

/// 将 32 字节位图编码为 64 字符 hex 字符串
fn Bitmap_To_Hex(bitmap: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bitmap {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 从 64 字符 hex 字符串解码为 32 字节位图
fn Parse_Bitmap_Hex(hex: &str) -> [u8; 32] {
    let mut bitmap = [0u8; 32];
    let hex = hex.trim();
    for i in 0..32.min(hex.len() / 2) {
        if let Ok(b) = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16) {
            bitmap[i] = b;
        }
    }
    bitmap
}

// GGUF_Load_Layer(gguf_file_path, layer_index, device)
// 输入：gguf文件路径，层索引（单个），设备信息
// 输出：GGUF_Layer_Weights，包含指定层的权重
// 层的定义：
//   第0层表示输入层（embedding），即 token_embd.weight
//   第1层到第N层表示中间层（transformer block），即 blk.0.* ~ blk.(N-1).*
//   第N+1层表示输出层，即 output_norm.weight + output.weight
// 按照如下的顺序执行：
// 1. 打开gguf文件，读取文件头信息，确认文件格式正确
// 2. 读取架构信息，获取总层数 N，确定层索引映射关系
// 3. 根据层索引映射找到属于目标层的 tensor
// 4. 读取tensor数据，按照gguf文件中存储的格式进行解析
// 5. 将解析后的数据转换为 GGUF_Layer_Weights 对象
// 6. 返回 GGUF_Layer_Weights 对象
pub fn GGUF_Load_Layer(
    content: &gguf_file::Content,
    file: &mut std::fs::File,
    layer_index: usize,
    device: &Device,
) -> Result<GGUF_Layer_Weights> {

    // 2. 读取架构信息，获取总层数 N
    let architecture = Get_Metadata_String(&content.metadata, "general.architecture")
        .unwrap_or_else(|| "unknown".to_string());
    let num_layers = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.block_count", architecture),
    )
    .unwrap_or(0);

    // 层编号规则:
    //   0           = embedding 层 (token_embd.weight)
    //   1..=N       = transformer block 层 (blk.0.* ~ blk.(N-1).*)
    //   N+1         = output 层 (output_norm.weight + output.weight)
    let max_layer_index = num_layers + 1;

    if layer_index > max_layer_index {
        anyhow::bail!(
            "Invalid layer_index: {} exceeds max layer index ({}, total {} transformer blocks)",
            layer_index,
            max_layer_index,
            num_layers
        );
    }

    let mut tensors: HashMap<String, QTensor> = HashMap::new();

    // 3. 根据层索引映射找到属于目标层的 tensor，并加载
    if layer_index == 0 {
        // 第0层：输入层（embedding）
        let tensor_name = "token_embd.weight";
        if content.tensor_infos.contains_key(tensor_name) {
            let qtensor = content
                .tensor(file, tensor_name, device)
                .map_err(|e| {
                    anyhow::anyhow!("Failed to load tensor '{}': {}", tensor_name, e)
                })?;
            tensors.insert(tensor_name.to_string(), qtensor);
        } else {
            anyhow::bail!(
                "No tensors found for layer 0 (embedding): '{}' not found in GGUF file",
                tensor_name
            );
        }
    } else if layer_index <= num_layers {
        // 第1层到第N层：transformer block 层
        // layer_index 映射到 blk.(layer_index - 1)
        let blk_idx = layer_index - 1;
        let prefix = format!("blk.{}.", blk_idx);

        // 4. 读取 tensor 数据，按照 gguf 文件中存储的格式进行解析
        for tensor_name in content.tensor_infos.keys() {
            if tensor_name.starts_with(&prefix) {
                let qtensor = content
                    .tensor(file, tensor_name, device)
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to load tensor '{}': {}", tensor_name, e)
                    })?;
                tensors.insert(tensor_name.clone(), qtensor);
            }
        }

        if tensors.is_empty() {
            anyhow::bail!(
                "No tensors found for layer {} (blk.{}, prefix '{}')",
                layer_index,
                blk_idx,
                prefix
            );
        }
    } else {
        // 第N+1层：输出层（output_norm + output.weight）
        for tensor_name in &["output_norm.weight", "output.weight"] {
            if content.tensor_infos.contains_key(*tensor_name) {
                let qtensor = content
                    .tensor(file, *tensor_name, device)
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to load tensor '{}': {}", tensor_name, e)
                    })?;
                tensors.insert(tensor_name.to_string(), qtensor);
            }
        }

        if tensors.is_empty() {
            anyhow::bail!(
                "No tensors found for layer {} (output layer): output_norm.weight / output.weight not found",
                layer_index
            );
        }
    }

    // 5. 将解析后的数据转换为 GGUF_Layer_Weights 对象
    // 6. 返回 GGUF_Layer_Weights 对象
    Ok(GGUF_Layer_Weights {
        layer_index,
        tensors,
    })
}

// GGUF_Split_Model(gguf_file_path, split_start, split_end， output_gguf_file_path)
// 从给定的gguf file path当中读取gguf文件
// 按照split_start和split_end当中的配置对gguf文件进行切分
// 我们这里需要做出这样明确的规定，以Qwen3 0.6B模型为例
// 0 表示输入层，也就是embedding层
// 1-28表示中间层，也就是transformer block层
// 29表示输出层，也就是output norm和lm head层
// 因此如果我们需要切分出前4层，我们就应该传入split_start=0, split_end=3
// 切分函数按照这样的步骤执行
// 1. 打开gguf文件，读取文件头信息，确认文件格式正确
// 2. 根据split_start和split_end的配置，确定需要切分的层范围
// 3. 直接复制gguf的文件头
// 4. 读取gguf文件当中的metadata信息，然后对其进行修改，修改n_tensors 等等核心参数，确保它们与切分后的模型保持一致
// 5. 将其中的chat_template的内容设置为空，将tokennizer的内容设置为空
// 6. 在metadata中增加两个字段，分别是split_start和split_end，记录切分的层范围
// 7. 读取gguf文件当中的tensor信息，然后按照split_start和split_end的配置进行筛选，保留需要切分的层的tensor信息
// 8. 将筛选后的tensor信息写入新的gguf文件当中
// 9. 进行对齐填充
// 10. 从原始的gguf文件对应位置读取tensor数据，并将需要切分的层的tensor数据写入新的gguf文件当中
// 11. 将新的gguf文件保存到磁盘output_gguf_file_path位置上
// 12. 文件名称为原始gguf文件名称+_split_{split_start}_{split_end}.pgguf(这里使用我们自己定义的文件格式，避免与原始格式冲突)
pub fn GGUF_Split_Model(
    gguf_file_path: &Path,
    split_start: usize,
    split_end: usize,
    output_gguf_file_path: &Path,
    keep_tokenizer: bool,
) -> Result<()> {
    // 1. 打开 gguf 文件，读取文件头信息，确认文件格式正确
    let mut src_file = std::fs::File::open(gguf_file_path)?;
    let content = gguf_file::Content::read(&mut src_file)
        .map_err(|e| anyhow::anyhow!("Failed to read GGUF content: {}", e))?;

    // 获取架构和层数信息
    let architecture = Get_Metadata_String(&content.metadata, "general.architecture")
        .unwrap_or_else(|| "unknown".to_string());
    let num_layers = Get_Metadata_Usize(
        &content.metadata,
        &format!("{}.block_count", architecture),
    )
    .unwrap_or(0);

    // 层编号规则:
    //   0           = embedding 层 (token_embd.weight)
    //   1..=N       = transformer block 层 (blk.0.* ~ blk.(N-1).*)
    //   N+1         = output 层 (output_norm.weight + output.weight)
    let max_layer_index = num_layers + 1;

    // 2. 根据 split_start 和 split_end 的配置，确定需要切分的层范围
    if split_start > split_end {
        anyhow::bail!(
            "Invalid split range: split_start ({}) > split_end ({})",
            split_start,
            split_end
        );
    }
    if split_end > max_layer_index {
        anyhow::bail!(
            "Invalid split range: split_end ({}) exceeds max layer index ({}, total {} transformer blocks)",
            split_end,
            max_layer_index,
            num_layers
        );
    }

    // 收集需要保留的 tensor 名称
    let mut selected_tensor_names: Vec<String> = Vec::new();

    for layer_idx in split_start..=split_end {
        if layer_idx == 0 {
            // Embedding 层
            if content.tensor_infos.contains_key("token_embd.weight") {
                selected_tensor_names.push("token_embd.weight".to_string());
            }
        } else if layer_idx <= num_layers {
            // Transformer block 层: layer_idx 映射到 blk.(layer_idx - 1)
            let blk_idx = layer_idx - 1;
            let prefix = format!("blk.{}.", blk_idx);
            for tensor_name in content.tensor_infos.keys() {
                if tensor_name.starts_with(&prefix) {
                    selected_tensor_names.push(tensor_name.clone());
                }
            }
        } else if layer_idx == max_layer_index {
            // Output 层 (output_norm + lm_head)
            for name in &["output_norm.weight", "output.weight"] {
                if content.tensor_infos.contains_key(*name) {
                    selected_tensor_names.push(name.to_string());
                }
            }
            // Weight tying: 如果 output.weight 不存在，说明模型使用了权重共享，
            // lm_head 复用 token_embd.weight。此时即使切分范围不包含输入层（layer 0），
            // 也需要将 token_embd.weight 包含在切分文件中，以确保输出层能正常工作。
            if !content.tensor_infos.contains_key("output.weight") {
                if content.tensor_infos.contains_key("token_embd.weight")
                    && !selected_tensor_names.contains(&"token_embd.weight".to_string())
                {
                    selected_tensor_names.push("token_embd.weight".to_string());
                }
            }
        }
    }

    // 对 tensor 名称排序，确保写入顺序确定
    selected_tensor_names.sort();

    if selected_tensor_names.is_empty() {
        anyhow::bail!(
            "No tensors selected for split range [{}, {}]. Check layer range.",
            split_start,
            split_end
        );
    }

    // 7. 加载筛选后的 tensor 数据（使用 CPU 设备，仅用于读取原始数据）
    let device = Device::Cpu;
    let mut loaded_tensors: Vec<(String, QTensor)> = Vec::with_capacity(selected_tensor_names.len());

    for tensor_name in &selected_tensor_names {
        let qtensor = content
            .tensor(&mut src_file, tensor_name, &device)
            .map_err(|e| anyhow::anyhow!("Failed to load tensor '{}': {}", tensor_name, e))?;
        loaded_tensors.push((tensor_name.clone(), qtensor));
    }

    // 3~6. 构建修改后的 metadata
    //   - 复制原始 metadata（跳过 tokenizer 和 chat_template 相关的 key）
    //   - 保留原始 block_count（不修改），使切分模型保留完整的层信息，
    //     不包含的层在 GGUF_Analyze / GGUF_Load_Layer 中自然为空
    //   - 追加 pleiades.split.start 和 pleiades.split.end 字段
    let mut metadata_pairs: Vec<(String, gguf_file::Value)> = Vec::new();

    // 需要跳过的 metadata key（旧的 layer_bitmap 必须跳过以重新计算）
    let mut skip_keys: Vec<&str> = vec!["pleiades.layer_bitmap"];
    if !keep_tokenizer {
        skip_keys.push("tokenizer.");
        skip_keys.push("chat_template");
    }

    for (key, value) in &content.metadata {
        let should_skip = skip_keys
            .iter()
            .any(|prefix| key.starts_with(prefix));
        if should_skip {
            continue;
        }
        // 保留所有其他 metadata（包括 block_count）原样复制
        metadata_pairs.push((key.clone(), value.clone()));
    }

    // 5.5 根据已筛选的 tensor 名称重新构建 layer_bitmap（仅标记 split 范围内实际存在的层）
    let mut new_bitmap = [0u8; 32];
    for tensor_name in &selected_tensor_names {
        if tensor_name.starts_with("blk.") {
            // tensor 名称格式: blk.{idx}.{rest}
            let parts: Vec<&str> = tensor_name.splitn(3, '.').collect();
            if parts.len() >= 2 {
                if let Ok(blk_idx) = parts[1].parse::<usize>() {
                    if blk_idx < 256 {
                        new_bitmap[blk_idx / 8] |= 1 << (blk_idx % 8);
                    }
                }
            }
        }
    }
    metadata_pairs.push((
        "pleiades.layer_bitmap".to_string(),
        gguf_file::Value::String(Bitmap_To_Hex(&new_bitmap)),
    ));

    // 6. 追加切分范围标记
    metadata_pairs.push((
        "pleiades.split.start".to_string(),
        gguf_file::Value::U32(split_start as u32),
    ));
    metadata_pairs.push((
        "pleiades.split.end".to_string(),
        gguf_file::Value::U32(split_end as u32),
    ));

    // 对 metadata 按 key 排序，确保写入顺序确定
    metadata_pairs.sort_by(|a, b| a.0.cmp(&b.0));

    // 12. 构建输出文件路径
    //     文件名称为: {原始文件名}_split_{start}_{end}.pgguf
    let original_stem = gguf_file_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let output_filename = format!(
        "{}_split_{}_{}.pgguf",
        original_stem, split_start, split_end
    );
    let output_path = output_gguf_file_path.join(&output_filename);

    // 确保输出目录存在
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // 8~11. 使用 candle 的 gguf_file::write 将 metadata + tensor 数据写入新文件
    //       该函数内部处理:
    //       - GGUF header 写入 (magic, version, n_tensors, n_kv)
    //       - metadata KV pairs 序列化
    //       - tensor info 写入（含 offset 重算）
    //       - 对齐填充
    //       - tensor data 写入
    let out_file = std::fs::File::create(&output_path)?;
    let mut writer = BufWriter::new(out_file);

    // 构建引用切片供 gguf_file::write 使用
    let metadata_refs: Vec<(&str, &gguf_file::Value)> = metadata_pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v))
        .collect();

    let tensor_refs: Vec<(&str, &QTensor)> = loaded_tensors
        .iter()
        .map(|(n, t)| (n.as_str(), t))
        .collect();

    gguf_file::write(&mut writer, &metadata_refs, &tensor_refs)
        .map_err(|e| anyhow::anyhow!("Failed to write split GGUF file: {}", e))?;

    // 确保所有数据刷入磁盘
    drop(writer);

    Ok(())
}
