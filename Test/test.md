# Test目录说明文档

## 目录用途

Test目录是Pleiades项目的测试框架，用于对项目各个模块（特别是运行时层）进行功能验证和性能测试。

## 目录结构

```
Test/
├── test.md              # 本说明文档
├── Test.py              # 统一测试入口
├── Runtime_Test.py      # 运行时层测试脚本
├── Model/               # 测试模型目录
│   ├── CNN_Model.py    # CNN模型定义和生成脚本
│   └── ViT_Model.py    # ViT模型定义和生成脚本
└── Weights/            # 模型权重存储目录
    ├── cnn_model.pth   # CNN模型权重
    └── vit_model.pth   # ViT模型权重
```

## 使用方法

### 1. 运行所有测试

```bash
python Test/Test.py all
```

执行所有测试套件，包括Runtime层测试。

### 2. 运行Runtime测试

```bash
python Test/Test.py runtime
```

仅执行Runtime层的测试，包括：
- 系统快照功能测试
- 模型加载功能测试
- 模型推理功能测试

### 3. 创建/重新生成模型

```bash
python Test/Test.py models
```

重新生成CNN和ViT测试模型并保存到Weights目录。

### 4. 列出可用测试

```bash
python Test/Test.py list
```

显示所有可用的测试选项。

### 5. 直接运行测试脚本

也可以直接运行各个测试脚本：

```bash
# 运行Runtime测试
python Test/Runtime_Test.py

# 创建CNN模型
python Test/Model/CNN_Model.py

# 创建ViT模型
python Test/Model/ViT_Model.py
```

## 测试内容说明

### Runtime_Test.py

测试运行时层的核心功能：

#### Test_Get_System_Snapshot
测试系统快照功能，验证能否正确获取：
- CPU使用率、核心数、频率
- 内存总量、可用量、使用率
- 网络接口、IO统计、带宽、延迟
- GPU可用性、设备信息

#### Test_Load_Model
测试模型加载功能，验证：
- 能否成功加载CNN模型
- 能否成功加载ViT模型
- 能否正确处理加载失败的情况
- 模型是否正确设置为评估模式

#### Test_Execute
测试模型推理功能，验证：
- 未加载模型时正确抛出异常
- CNN模型推理输出形状正确
- ViT模型推理输出形状正确
- 支持批量推理

## 测试模型说明

### CNN模型
- 架构：3层卷积神经网络（Conv + BatchNorm + MaxPool）
- 参数量：约1.1M
- 输入：(batch_size, 3, 32, 32)
- 输出：(batch_size, 10)

### ViT模型
- 架构：Vision Transformer（Patch Embedding + 6层Transformer）
- 参数量：约2.7M
- 输入：(batch_size, 3, 32, 32)
- 输出：(batch_size, 10)

两个模型均使用torch.jit.trace保存，避免类定义依赖问题。

## 测试输出示例

运行Runtime测试时，会输出详细的系统信息：

```
测试 Get_System_Snapshot...
  CPU使用率: 15.0%
  内存使用率: 72.1%
  GPU可用: True
  网络接口数量: 6
  网络发送字节: 1,234,567,890
  网络接收字节: 9,876,543,210
  发送带宽: 0.52 Mbps
  接收带宽: 1.24 Mbps
  网络延迟: 0.15 ms
  [OK] Get_System_Snapshot 测试通过
```

## 注意事项

1. **首次运行**：首次运行测试前，需要先生成测试模型：
   ```bash
   python Test/Test.py models
   ```

2. **GPU支持**：测试会自动检测GPU可用性，如果有GPU会自动使用GPU进行推理

3. **网络测试**：网络带宽和延迟测试可能需要一定时间（约0.1秒），这是正常的

4. **权重文件**：Weights目录下的.pth文件是torch.jit traced模型，可以直接加载使用

## 扩展测试

如需添加新的测试用例：

1. 在Runtime_Test.py中添加新的测试函数
2. 在函数名前加上`Test_`前缀
3. 在`Run_All_Tests()`函数中调用新的测试函数
4. 使用assert进行验证，输出使用`[OK]`标记通过状态

## 故障排除

### 模型加载失败
- 确保已运行`python Test/Test.py models`生成模型
- 检查Weights目录是否存在且包含模型文件

### Unicode编码错误
- 已修复，所有输出使用ASCII字符
- 如仍有问题，检查终端编码设置

### GPU相关错误
- 测试会自动处理GPU不可用的情况
- 如遇到CUDA错误，检查PyTorch和CUDA版本兼容性
