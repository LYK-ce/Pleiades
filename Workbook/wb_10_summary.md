# wb_10_summary — 2026-05-28

## 事务 1: Architecture 文档更新
- 完成: 2026-05-28
- 更新内容: 日期; B5 标注未实现; inference_id/u64; Network 新增 API (send_response, put_record, get_record, open_file_stream, send_file_data, receive_file_data); MlSession 新增 get_offset/set_seed; LuaTensor 方法 (dims/to_bytes/to_device); storage handle:release(); network.send_response/put_record/get_record/open_file_stream; Session_Manager 3.9 节; Scheduler config 备注; ProgramRegistry 脚本发现; legacy 目录

## 事务 2: README 更新
- 完成: 2026-05-28
- 内容: 项目介绍、核心特性、快速开始(构建/运行)、项目结构、架构概览图

## 事务 3: 清理
- 完成: 2026-05-28
- 删除: /workspace/Src/__pycache__/
- 移动至 legacy: programs/builtin/{run,pipeline,list}.lua (引用不存在的 API: caps.create_session, caps.allocate_io, caps.get_available_peers, caps.plan_weighted/uniform, caps.split_model/wrong-table, caps.send_file/wrong-table, caps.establish_streams, caps.send_tensor/wrong-sig, caps.receive_tensor/wrong-sig, sess:get_output_tensor)

## 事务 4: Code Review 结果

### 严重问题
|#|等级|问题|位置|
|-|-|-|-|
|1|🔴中|algo.unwrap() panic风险 |capability_binding.rs:160|
|2|🔴中|B2 入站路由全TODO |branch_command.rs|
|3|🟡低|B3 TensorStreamArrived TODO |branch_stream.rs:61|
|4|🟡低|B5 EventBus路由缺失 |Core主循环|
|5|🟡低|analyze_model闭包违反Lua绑定规范(15行table构造) |capability_binding.rs:220-237|
|6|🟡低|Scheduler_Config缺失 |config.rs|
|7|🟡低|Has_Active_Session()始终false |app.rs:354|
|8|🟡低|SessionManager::Take_Frontend未实现 |manager.rs:31|
|9|🟢低|PeerId::random() TODO |network_service.rs:312|

### 代码质量
- ✅ 模块化trait pattern贯穿 (Network/Storage/Peer/Session)
- ✅ 并发安全: RwLock+oneshot+RendezvousMap
- ✅ 路径安全检查 (file_id防目录遍历)
- ✅ MlSession Lua绑定完全符合规范 (薄胶水模式)
- ✅ 测试覆盖: Storage 30+, VM 15+, EventBus 4
- ⚠️ try_into().unwrap() 5处 (gguf_tensor.rs) — 应expect
- ⚠️ Mutex poison .unwrap_or_else(|e|e.into_inner()) 多处 — 统一处理
- ⚠️ spawn_lua_script用std::thread而非tokio::task — 资源效率
- ⚠️ Box<dyn Error> 丢失类型信息

### 规范符合度
- 归属注释: ✅ 全部有 Present by KeJi + Date
- Pascal_Snake_Case函数: ✅
- #![allow(non_snake_case)]: ⚠️ 几乎所有文件，暗示命名未充分规范化
- Lua绑定规范: ⚠️ 1处违反 (analyze_model)

### 模块耦合度
- 低耦合: trait object依赖倒置
- Session_Manager: 几乎未被Core使用
- EventBus: 仅TUI消费，Core不订阅
