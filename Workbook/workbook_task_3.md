# Task 3: CPU/GPU 混合推理（单脚本顺序流水线）

> start: 2026-05-21 | end: 2026-05-21 | status: ✅ done

## 执行记录

### 3.1 LuaTensor::to_device
- Src/ML_Engine/lua_tensor.rs: UserData 新增 `to_device(device_str)` 方法
- 调用 `Tensor::to_device(&Device)` 做直接设备间拷贝

### 3.2 cpu_gpu_run.lua
- programs/user/cpu_gpu_run.lua (新建, 105行)
- COMMAND="cpu_gpu_run"
- 用法: exec cpu_gpu_run model=xxx.pgguf split=16
- 流: cpu:forward → to_device("cuda") → gpu:forward → to_device("cpu") → sample

### 3.3 文档
- docs/capabilities.md: LuaTensor 方法表 +to_device + 混合推理示例

### 3.4 测试
- cargo test --lib: 103 passed, 0 failed
