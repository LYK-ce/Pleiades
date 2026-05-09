1. node.rs中idle_connection_timeout设为60秒，空闲连接会自动断开。Pleiades为少量设备持久协作场景，应改为长超时或引入心跳机制替代。位置：node.rs:232 `with_idle_connection_timeout(Duration::from_secs(60))`
已解决
2. CUDA 版本分发与 DLL 依赖问题

   **问题**: 使用 `candle-core[cuda]` 编译的二进制文件，PE 导入表中硬性引用了 `nvcuda.dll`、`cublas64_12.dll` 等 CUDA DLL。在没有 CUDA 环境的机器上运行时，Windows 直接弹窗"找不到 DLL"，程序无法启动。

   **根因**: candle → cudarc 编译时链接了 CUDA 的 import library（.lib），产生了链接时 DLL 依赖。CUDA 不支持静态链接（NVIDIA EULA 禁止将 cuBLAS/cuDNN 静态嵌入后重新分发）。

   **PyTorch 为什么不需要装 CUDA Toolkit**: PyTorch 的 pip wheel 包（~2.5GB）内嵌了所有 CUDA 运行时 DLL（cublas、cublasLt、cudart、cudnn 等），安装时直接释放到 `torch/lib/` 目录。PyTorch 真正需要的系统级依赖只有 `nvcuda.dll`（NVIDIA 显卡驱动自带）。

   **解决方案 — 三版本分发策略**:

   | 版本 | 包含内容 | 大小（估计） | 用户需要 |
   |------|---------|-------------|---------|
   | `pleiades-cpu` | exe + .config | ~15-20 MB | 无要求 |
   | `pleiades-cuda` | exe + .config | ~15-20 MB | 安装 CUDA Toolkit |
   | `pleiades-cuda-bundled` | exe + .config + CUDA DLL | 压缩后 ~200 MB | 仅 NVIDIA 显卡驱动 |

   - 三个版本用同一份代码，只是编译 feature 和打包策略不同
   - `nvcuda.dll` 是唯一无法打包的 DLL（驱动级），但只要安装了 NVIDIA 显卡驱动就自动存在
   - NVIDIA EULA 允许将 cudart/cublas 等运行时 DLL 随应用程序重新分发
   - 确定需要打包哪些 DLL：`dumpbin /imports pleiades.exe | findstr /i cuda`
已解决

3. 针对张量传输低频、竞争小的场景，可通过预分配大内存池（如启动时 mmap 或预留数个百 MB 的 Vec）来规避动态分配与零初始化开销；利用 Mutex 保护池即可（竞争少，锁开销可忽略），读取时用 Vec::with_capacity + unsafe { set_len } 避免 vec![0; n] 的无效写操作，传完归还复用，实现“固定内存、按需取用”的零拷贝或低拷贝传输。
   
4. 当前的config.toml里面配置的默认设备是cpu，因此即便是有cuda环境也是默认cpu
   已解决

5. 张量传输与命令共享通道问题：当前张量数据(DataType::Data)与控制命令(DataType::Command)共用同一个request-response协议，pipeline推理时每个张量都要走完整的request→ACK流程，增加了不必要的往返延迟，且大张量可能阻塞后续控制命令。解决方案是参照文件传输(stream_protocol.rs)的设计，为张量引入专用的libp2p_stream通道(/pleiades/tensor/1.0.0)，pipeline建立时打开一条持久流，推理期间张量直接写入流中（fire-and-forget，无需等ACK），借助TCP自然背压控制流速，命令通道仅用于控制消息，实现数据面与控制面的完全隔离。

6. 缺少心跳机制，设备意外离线检测慢：当前没有应用层心跳，设备正常退出时TCP FIN能即时检测，但断电/拔网线等异常离线只能依赖mDNS过期（约10-15分钟）或下次发送数据时的30秒request超时来发现，检测延迟极高。解决方案是启用libp2p内置的ping行为（每15秒自动ping一次，3次失败触发ConnectionClosed，约45秒检测到异常离线），同时配置TCP层keepalive（约10秒间隔），双重保障将异常离线检测时间从分钟级降到30-45秒，且ping返回的RTT数据可填入PeerInfo.latency_ms用于后续任务调度。具体实现方式如下：libp2p 0.56 已内置 `libp2p::ping::Behaviour`，无需额外 Cargo 依赖。实现步骤：(1) 在 `network_service.rs` 的 `PleiadesNetworkBehaviour` 中添加 `pub ping: libp2p::ping::Behaviour` 字段；(2) 在 `with_behaviour` 闭包中创建 `libp2p::ping::Behaviour::new(libp2p::ping::Config::new().with_interval(Duration::from_secs(15)))`；(3) 在 `Handle_Swarm_Event` 中添加 `PleiadesNetworkBehaviourEvent::Ping(event)` 匹配分支，从 `event.result` 中提取 RTT 并更新 `connected_peers` 中对应 `PeerInfo.latency_ms`；ping 失败时 Swarm 会自动触发 `ConnectionClosed` 事件，无需额外处理。该方案在应用层检测进程假死，配合 TCP keepalive 在传输层检测连接断裂，双重保障覆盖所有异常离线场景。

7. Android 部署问题与解决方案：当前项目在 Android 上部署面临四个问题——(1) TUI 模块依赖终端环境（ratatui+crossterm），Android 没有终端，需要将 TUI 替换为 WebUI（内嵌轻量 HTTP Server 如 axum + WebSocket 推送 Ui_Message），Android 端用 WebView 加载 localhost 页面，该方案同时也适用于桌面平台；(2) GPU 加速不可用，Android 不支持 CUDA，candle-core 也未提供 Vulkan/NNAPI 后端，只能使用 CPU 推理（编译时 --no-default-features 去掉 cuda feature），ARM64 上 candle 可利用 NEON 指令集；(3) mDNS 局域网发现受限，Android 默认过滤组播包，需要在 Kotlin 层通过 WifiManager.MulticastLock 获取组播锁后再启动 Rust 网络层，否则 libp2p 的 mDNS 行为无法收到邻居节点的广播；(4) 文件系统与入口点差异，Android 应用只能写入 app data 目录，需要将 Pleiades_Workspace 和 .config 的路径从硬编码改为由 JNI 从 Kotlin 层传入的 Android 数据目录，同时 main.rs 的入口需要改为 `#[no_mangle] pub extern` 的 JNI 导出函数，由 Android Activity 通过 JNI 调用启动。建议的实施路径是先在桌面实现 WebUI 替换 TUI（可在浏览器中测试验证），再交叉编译到 aarch64-linux-android 并编写 Kotlin 壳应用（WebView + JNI + 权限声明），最后处理 mDNS 和文件路径等平台适配细节。

8. connected_peers（节点连接管理）当前直接以 HashMap<PeerId, PeerInfo> 字段嵌在 Network_Service 中，查询操作（Get_Peers/Get_Peer_Info）通过 NodeCommand 管道往返实现，存在不必要的延迟——每次查询需经历 cmd_tx.send → select! 调度 → oneshot 回传的完整往返。应将其提取为独立的 PeerStore 组件，采用 `Arc<std::sync::RwLock<HashMap<PeerId, PeerInfo>>>` 共享锁方案：Network_Service 持有写锁（连接建立/断开时更新，极低频），NodeHandle 和 Control/TUI 层持有读锁（查询节点列表/RTT 等，高频），读写互不阻塞且无 async channel 开销。好处包括：(1) 读取从微秒级管道往返降至纳秒级内存访问；(2) Get_Peers/Get_Peer_Info 从 async 方法变为同步方法，简化调用链；(3) 可移除 NodeCommand::GetPeers/GetPeerInfo 两个命令变体，精简命令通道；(4) 为未来高频场景（TUI 每帧刷新节点列表、ping 心跳每 15 秒更新 RTT、基于延迟的任务调度）提供性能保障。使用 std::sync::RwLock 而非 tokio::sync::RwLock，因为锁内操作均为纳秒级 HashMap 访问，无需 async 开销。

9.  Inbound_Request_Manager 的 pending_responses 和 pending_replies 两个 HashMap 缺少超时清理机制：出站请求发出后若对方永不响应，虽然 NodeHandle.Send_Bytes 的 30 秒 tokio::timeout 会让调用方超时返回，但 oneshot::Sender 被 drop 后对应的 HashMap 条目不会被清理，造成内存泄漏；入站请求若被 Control 层忽略不回复，ResponseChannel 会永久持有在 pending_replies 中。当前低频场景下泄漏量极小不构成实际问题，但长期运行或高频交互时可能累积。解决方案是为每个 pending 条目附带 `created_at: Instant` 时间戳，在每次 Register/Route 操作时顺带扫描并清理超过 TTL（如 60 秒）的过期条目，对过期的 pending_responses 通过 oneshot 发送超时错误，对过期的 pending_replies 直接丢弃 ResponseChannel（libp2p 会自动处理未回复的请求）。

10. Outbound_Manager 当前仅负责响应追踪与路由（纯状态管理），实际的请求发送操作（swarm.behaviour_mut().request_response.send_request()）仍在 Network_Service 中执行，不应将发送逻辑合并到 Outbound_Manager 中。原因有三：(1) 合并会引入 libp2p request_response::Behaviour 类型耦合，当前 Outbound_Manager 仅依赖轻量的 OutboundRequestId；(2) Rust 借用检查器不允许在同一方法调用中同时可变借用 self.swarm 和 self.outbound_manager；(3) 当前 Facade+State Manager 模式是 actor 架构的惯用设计——Network_Service 负责 I/O 交互，Manager 只管状态。若未来需要自动重传功能，应通过给 Outbound_Manager 一个 mpsc::Sender<NodeCommand> 来间接发送请求，而非直接持有 Swarm 引用。

11. ML Engine 推理路径优化：数据平面直通 + 零拷贝张量传输

    **问题**：当前 pipeline 推理中，每一步自回归生成的 tensor 数据都需要经过 Network→Control→ML_Service(channel)→Worker→ML_Service(oneshot)→Control→Network 的完整链条，共约 4 次 channel 跨越。此外，tensor 从网络接收到 GPU 推理再到网络发送，存在约 10 次内存拷贝：(1) socket→payload (2) payload.to_vec() 传给 Service_Command (3) GGUF_Tensor_Deserialize 中 data_slice.to_vec() (4) bytes→Vec<f32> 类型转换 (5) Tensor::new 上传到 GPU (6) 推理结果 to_vec1<f32> 读回 CPU (7) f32→le_bytes 转换 (8) GGUF_Tensor_Serialize 打包 (9) offset 前缀拼接 (10) write_all 写入 socket。以 Qwen3-8B hidden_state [1,1,4096] 为例，decode 阶段每步仅 16KB 数据量，拷贝开销可忽略；但 prefill 阶段（如 seq_len=512）数据量达 8MB，10 次拷贝意味着 ~80MB 内存带宽开销。

    **优化方向一：数据平面与控制平面分离**。Control 层在 pipeline 建立阶段（WORK/LOAD/PIPELINE_FLOW 等低频命令）仍发挥中心协调作用，但 tensor 数据的高频传输应绕过 Control 直达 ML Engine。具体方案有两种：(A) Control 内部 fast-path——进入 Busy 状态后启动专用 inference pipeline task，该 task 直接持有 inbound_rx(仅 Data 类型)、ml_service、node_handle，在紧凑循环内执行 recv→inference→send，避免经过 Control 主循环的 match 分发；(B) 真正的 Network→ML Engine 直连——ML Engine 层直接持有 Network 的发送句柄和接收通道，pipeline 建立后 tensor 数据完全不经过 Control，但这会增加模块间耦合且使状态管理和错误处理分散化。推荐方案 A 作为折中。

    **优化方向二：专用 Tensor Stream**。当前 tensor 走 data_protocol.rs 的 request-response 模式，每次传输需完整的 TLV 帧打包和 ACK 往返。参照 stream_protocol.rs 的设计，为 tensor 引入专用的持久化 libp2p::Stream 通道（/pleiades/tensor-stream/1.0.0），pipeline 建立时打开一条持久双向流，推理期间每步仅发送 [8B offset][raw tensor bytes]，无需重新建立请求、无需等 ACK，借助 TCP 背压控制流速。这与 problem 5 中提到的张量专用通道方案是一致的。

    **优化方向三：零拷贝/低拷贝传输**。理论最低拷贝次数为 4 次（socket→CPU buffer、CPU→GPU、GPU→CPU buffer、CPU buffer→socket），当前 10 次可优化掉 6 次。具体手段：(a) 使用 `bytes::Bytes` 引用计数共享替代 Vec<u8> 拷贝，Network/Control/Worker 间传递同一块内存的引用（省去 Copy 2、3）；(b) 使用 `bytemuck::cast_slice` 将 &[u8] 直接 reinterpret 为 &[f32]，跳过中间 Vec<f32> 分配（省去 Copy 4）；(c) 输出时预分配 send_buffer，从 GPU Tensor 直接读入发送缓冲区，避免 Vec<f32>→bytes→serialize 的多次转换（省去 Copy 7、8）；(d) 对于 GPU 场景，可使用 CUDA pinned memory（页锁定内存）加速 CPU↔GPU 传输。需要引入 `bytes` 和 `bytemuck` 两个 crate 作为新依赖。

    **优先级与实施建议**：对于当前 decode 阶段 16KB/step 的场景，channel 开销和拷贝开销相对于 GPU 推理时间（10-50ms）和网络传输时间（1-10ms LAN）而言不到 0.1%，暂不构成瓶颈。但随着模型规模增大、context 变长、batch inference 引入，该优化将变得有意义。建议实施路径：先实现方向二（专用 Tensor Stream，与 problem 5 合并实施），再实现方向一的方案 A（Control 内部 fast-path），最后按需引入方向三的零拷贝优化。

12. ML Engine 指令驱动架构迁移后的多节点推理性能与正确性问题

    **背景**：将 ML Service 从独立命令 API（Load_Model/Inference/Encode/Decode 等）重构为指令驱动架构（Run_Program + Instruction 序列），同时将多节点张量传输从 Control 中转的 request-response 方案迁移到 tensor stream 直通方案。

    **已修复的正确性问题**：
    - **(a) TakeTensorStreams 破坏性 Take**：network_service.rs 的 TakeTensorStreams 先 Take_Stream() 再检查，一方 None 时另一方的 stream 在 Err 路径被 drop 导致永久丢失。修复：先 Has_Stream() 检查两端就绪再 Take。
    - **(b) Tensor stream 建流时序**：协调者在 PIPELINE_FLOW 之后才创建 Manager/打开出站流，Worker 的入站流到达时 Manager 不存在被丢弃；且模型加载（耗时数秒）阻塞了出站流打开，Worker 的 5 秒 retry 窗口过期。修复：Create_Tensor_Stream 提前到 PIPELINE_FLOW 之前，Open_Tensor_Stream 提前到模型加载之前。
    - **(c) Send offset 语义错误**：Handle_Inference 自增 ctx.offset 后，Handle_Send 发送递增后的值，Worker 收到错误的位置编码 offset → shape mismatch。修复：新增 inference_offset 字段，Inference 前保存，Send 使用保存值。
    - **(d) Receive 覆盖 Coordinator offset**：Handle_Receive 无条件设 ctx.offset = received_offset，破坏 Coordinator 自管理的序列位置 → decode 阶段 KV cache 位置冲突 → 输出不连贯。修复：拆分为 offset（Coordinator 自增）和 received_offset（Worker 使用），Handle_Inference 根据 has_input_head 选择。
    - **(e) Receive 不更新 ctx.output**：Handle_Receive 只设 ctx.input_bytes 不设 ctx.output，Coordinator 的 Sample 读到旧的 hidden state 而非 Worker 返回的 logits → 每轮采样相同垃圾 token。修复：Handle_Receive 同时 Deserialize_Tensor 到 ctx.output。

    **待解决的性能问题**：tensor stream 直通方案理论上应比 Control 中转 request-response 更快（无 ACK 往返、持久连接），但实测反而更慢。疑似原因：(1) Worker 的 Handle_Receive 冗余调用 Deserialize_Tensor（Worker 的 Inference 读 ctx.input_bytes 不需要 ctx.output，但每帧都做了全量 bytes→Tensor 反序列化）；(2) Tensor_IO_Handle 的 block_on(async_io) 与 tokio runtime 主线程池竞争；(3) 相比旧方案多了一层 GGUF packet 封装。优化方向：Handle_Receive 去掉自动反序列化，改为 Sample 内部按需反序列化。

    **mDNS 发现范围问题**：当前使用默认 mDNS 服务名 `_p2p._udp.local`，会发现局域网内所有 libp2p 应用。若同一网络有不相关的 Pleiades 实例，会被 Get_Peers() 视为可用 Worker 并尝试分配推理任务。需要自定义 mDNS 服务名或引入集群标识机制。

13. GGUF_Model_Inference 接口签名与实际输入类型不匹配

    **问题**：`GGUF_Model_Inference` 函数签名接受 `&[u8]` 字节切片，但引擎层调用时实际持有的输入要么是 `Vec<u32>`（token IDs），要么是 `Tensor`（hidden state）。当前实现在两条路径上都做了无意义的序列化→反序列化往返：(1) Tokens 路径：Engine 将 `Vec<u32>` 通过 `Token_Ids_To_Bytes()` 转为 `Vec<u8>`，传入 `GGUF_Model_Inference` 后又将 `&[u8]` 转回 `Vec<u32>` 再创建 `Tensor`；(2) Tensor 路径：Engine 将 `Tensor` 通过 `Tensor_To_Bytes()` 序列化为 `Vec<u8>`，传入后又 `GGUF_Tensor_Deserialize` 反序列化回 `Tensor`。两条路径的数据最终都是 `Tensor` 进入 `model.Forward()`。

    **解决方案**：将 `GGUF_Model_Inference` 签名从 `(model, &[u8], offset)` 改为 `(model, &Tensor, offset)`，由引擎层负责在调用前将 token IDs 直接构造为 `Tensor`（`Tensor::new(&token_ids, device)?.unsqueeze(0)?`），或直接传入寄存器中的 `Tensor`。同时移除不再需要的 `Token_Ids_To_Bytes()` 辅助函数和 `Inference_With_Backend` 的 bytes 中间层，更新 `ml_thread_engine.rs` 中 Prefill/Inference 指令执行逻辑及相关测试文件。

14. GGUF 模型加载性能问题：From_Extracted 方法 vs New 方法

    **问题**：当前 `gguf_model.rs` 使用 `Layer_Weights::From_Extracted()` 方法从 `HashMap<String, QTensor>` 构建模型层，而 `qwen3.rs` 提供了更高效的 `Layer_Weights::New()` 方法直接从 `Gguf` 读取器加载。当前架构存在性能问题：(1) 重复文件 I/O：`GGUF_Load_Layer` 每层都重新打开和解析 GGUF 文件；(2) 内存碎片化：张量存储在 `HashMap` 中导致内存不连续，缓存不友好；(3) 额外数据复制：需要中间 `HashMap` 存储，增加内存开销和构建时间。

    **解决方案**：改用 `Layer_Weights::New()` 方法，创建全局 `Gguf` 读取器，一次打开文件顺序读取所有层。优势包括：(1) 减少文件 I/O 操作（单次打开 vs 每层重新打开）；(2) 改善内存局部性（直接流式加载到最终数据结构）；(3) 减少中间数据结构和复制操作；(4) 可能获得更好的 CPU 缓存命中率。需要修改 `gguf_model.rs` 中的 `GGUF_Load_Model` 函数，创建全局 `Gguf` 读取器并直接调用 `Layer_Weights::New()`。

15. 心跳（Ping）故障处理待实现

    **背景**：已使用 libp2p 内置 ping 协议实现基本心跳检测（`network_service.rs` 中 `Handle_Ping_Event`），收到 Pong 时通过 `peer_handle.update_heartbeat()` 更新延迟，超时时通过 `peer_handle.update_status(Disconnected)` 标记断开。但当前仅做状态标记，未进行任何故障恢复操作。

    **待实现的故障处理**：

    **(a) 临时网络波动容忍**：当前实现单次 Ping 超时就立即标记为 Disconnected，过于激进。应引入滑动窗口机制（如连续 3/5 次失败才标记断开），避免因瞬间网络抖动误判。需要在 PeerInfo 中增加 `ping_fail_count: u32` 字段，Handle_Ping_Event 中超时时累加计数，成功时重置为 0，仅当计数达到阈值时才标记 Disconnected。

    **(b) 超时后自动重连**：节点被标记为 Disconnected 后，当前没有任何重连尝试。应实现指数退避重试机制（1s, 2s, 4s, 8s...最大 60s），在 Network 层启动重连任务，尝试重新 Dial 该节点。需要在 PeerInfo 中增加 `reconnect_attempts: u32` 字段用于计算退避时间。

    **(c) 推理任务影响处理**：如果在 pipeline 推理过程中某个 Worker 心跳超时，当前没有通知 Control 层中断或重分配任务。应在标记 Disconnected 时检查该节点是否处于 Busy 状态，若是则需通知 Control 层进行任务故障处理（如中断当前推理、重新分配计算图）。

    **(d) 心跳成功率监控**：应记录每个节点的心跳成功率和平均延迟，用于后续的节点选择和负载均衡。可在 PeerInfo 中增加 `ping_success_count: u64`、`ping_total_count: u64`、`avg_latency_ms: f64` 字段，Handle_Ping_Event 中持续更新统计数据。

    **(e) 与 ConnectionClosed 事件的协调**：Ping 超时后 libp2p 可能随后触发 ConnectionClosed 事件，导致重复处理（先 update_status(Disconnected) 再 remove_peer）。需要确保两者不冲突，可在 ConnectionClosed 处理中检查节点是否已被标记为 Disconnected。