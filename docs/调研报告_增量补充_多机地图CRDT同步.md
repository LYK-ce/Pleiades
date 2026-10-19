# 多机器人分布式地图一致性 / Occupancy Grid CRDT 同步 —— 增量补充调研报告

> 对已有两份报告（`调研报告_分布式地图一致性方案.md`、`调研报告_方案对照检查_工程实现细节.md`）的**增量补充**，聚焦本轮 6 个问题。既有报告已覆盖的内容（Kimera-Multi、δ-CRDT、stigmergic 论文、OctoMap clamp 参数、gossipsub 64KB 上限、OrbitDB/Filecoin 同步模式等）不重复。

---

## 1. 成熟方案盘点表

| 方案/项目 | 类型 | 关键机制 | 成熟度/可借鉴性 | 与我们方案的对应 |
|---|---|---|---|---|
| **mrgs**（gondsm/mrgs） | **分布式 P2P 栅格共享**（ROS1） | 每车跑完整系统；multimaster_fkie 跨 master；LZ4 压缩；`mrgs_complete_map` **增量融合树**（只对新增信息重新融合） | M.Sc. 级（2014），ROS1，25★，README 自述 unstable；**检索到的最接近"分布式共享整张栅格"的开源工程** | 证明"每车本地融合 + 压缩 + 增量"可落地；与 FULL/DELTA 分 topic 同构 |
| **mrg_slam**（aserbremen/Multi-Robot-Graph-SLAM） | 去中心化图 SLAM（非栅格） | 每车独立 SLAM，经 **ROS2 topic 共享位姿图**；作者实测强烈推荐 **rmw_zenoh**（DDS 多机间不稳定） | 活跃维护（Humble/Jazzy），171★，BSD-2 | 通信层佐证：多机可靠传输选 zenoh 类；共享位姿图而非栅格 |
| **Terrain-aware semantic mapping**（CU Boulder / Team MARBLE，DARPA SubT 第三） | 栅格分发（低带宽） | occupancy+traversability+stairs 编码进栅格，**带宽约束下分发至全车队** | 实战验证（SubT 2021） | "编码栅格 + 车队分发"与本场景同构，含带宽实测 |
| **Rogers et al. Swarm 栅格建图**（Robotics 2023） | 分布式 | 每车存 own 图，**与邻居交换、把邻居图并入本地图** | 学术（资源受限 swarm） | "每车 merged = own+Σ他人"本地融合模式的学术先例 |
| **QSP 四叉树同步协议**（Moll et al., Computer Networks 2021） | 游戏世界同步（跨服务器） | 世界按四叉树分块；**叶子=块版本，内部节点=哈希**追踪变化；区域化命名+组播 | 论文+工件（phylib/QSPArtifacts）；**"块哈希摘要追踪变化"最佳先例** | 支持"瓦片哈希摘要 + 有差异才传块"的轻量反熵 |
| **Geo-CRDT / Geometry-aware CRDT**（IJGI 2025）· **mCRDT** | 空间 CRDT（矢量） | 点/多边形/拓扑保持的协作空间编辑 CRDT | 2025 新兴方向；**是矢量非栅格**；再次确认栅格 CRDT 是空白 | 我们做成栅格版即原创 |
| **CRDT Game State Sync in P2P VR**（Dantas & Baquero, PaPoC'25） | P2P 游戏状态 CRDT | δ-CRDT 作者 Baquero 参与；BrickSync，op/state 混合，WebRTC 持续广播 | 2025 论文；作者明言持续广播缓解网络问题，但**未处理长分区/大量丢包** | 佐证"增量+持续广播"工程可用，也划出"不做反熵"的边界 |
| **Gaffer On Games 状态同步** | 游戏网络经典 | **priority accumulator**（按重要性选对象、带宽自适应）+ 序号检丢失/重复 + jitter buffer | 工业级游戏经验 | "广播预算不足时优先发哪些格"直接可抄 |
| **Intelligent Huffman**（Sensors 2024） | 栅格传输压缩 | CNN + **Huffman + RLE** 压缩整图，两机通信量降 99% | 学术（2024） | 全量 64KB 帧超限的现成解法（RLE/Huffman/LZ4） |
| **SYNAOS 地图管理**（工业博客 2025） | 工业产品（VDA 5050） | **三层地图**：定位层（SLAM 图，车自持，"车检测到变化即与车队共享，中央 FMS 不介入"）；导航层（节点/边，中央）；集成层（元模型，中央） | 产品级；VDA 5050 支持机间分发定位数据、**不支持导航图标准化** | **工业界验证"车间 P2P 共享定位栅格"是真实产品模式** |
| **NODE.maps / SyncroBot** | 工业产品 | 中央 "Live Maps"：收集各车更新→合并全局图→下发车队 | 产品级；**中心化**对照 | 反范式：工业默认中央合并，P2P 只在定位层 |
| **gossipsub 丢包实证**（go-libp2p-pubsub #197） | 分布式系统 | ~10 节点全向广播即出现丢失 | **gossipsub best-effort 无投递保证**，小规模也丢 | "不做反熵"不能建立在"网络不丢"上 |

## 2. 开源实现细节

- **mrgs** — https://github.com/gondsm/mrgs（ROS1；25★）。分布式模式每车 `distributed_node.launch`；`mrgs_data_interface` 跨 master 收发地图/变换；`mrgs_complete_map` 增量建"融合树"，收到新图只重熔增量部分；`mrgs_alignment` 输出两图融合+置信系数；**LZ4 压缩栅格**。借鉴：增量融合树、LZ4、分布式/集中式双模。
- **m-explore-ros2 map_merge** — https://github.com/robo-friends/m-explore-ros2（BSD；385★）。`map_merge.cpp`：**双 topic**（`<ns>/map` 全量 + `<ns>/map_updates` 增量 `OccupancyGridUpdate`），QoS `transient_local().reliable()`，`merging_rate` 4Hz，全量 topic 兜底刷新。`grid_compositor.cpp`：融合 = **OpenCV `cv::max` 逐格取 max**（最自信者胜），非 log-odds 加法——**max 是单调、幂等、可交换的，天然抗重复/乱序**，是"单调有界合并"的工程旁证。
- **Multi-Robot-Graph-SLAM** — https://github.com/aserbremen/Multi-Robot-Graph-SLAM（BSD-2；171★）：去中心化位姿图共享，实测推荐 rmw_zenoh。
- **QSP 工件** — https://github.com/phylib/QSPArtifacts：四叉树块版本+内部节点哈希差分的参考实现。
- **mCRDT** — https://github.com/njleonzhang/mCRDT：2D 地图标注协作 CRDT（拓扑保持，矢量）。
- **Terrain-aware mapping**（Team MARBLE 开源）：多语义栅格编码 + 车队分发带宽实测。

## 3. 对 4 个待决策点的建议

1. **增量周期**：建议 **5s 起步**，辅以 priority-accumulator 式自适应。依据：(a) 既有报告 §3.3 带宽模型：扫到就发≈1.1 万格/周期，2s 周期达 20~50KB/s/车，N=10 放大后逼近链路预算；(b) 先例（Intelligent Huffman 99% 压缩、Gaffer 优先级）都指向"先压缩+选优，再谈频率"；(c) mrgs/m-explore 的 4Hz 是本地全图刷新，非全网广播，不可直接类比。链路宽裕再降 2s，且先 RLE/LZ4 压帧。

2. **merged 截断上限**：**保持 ±8**。(a) OctoMap 默认仅 ±(2~3.5)，±8 已是保守阵营，±20 更僵（动态翻转更慢）且扩大阈值附近分歧；(b) i8 内两者均可，纯语义选择；(c) **clamp-阈值间距只有 2（±8 vs ±6）**：饱和格两次 −1 即翻转，是"饱和吸收+可更新"的甜点区，放宽到 ±20 把间距放大到 14，明显牺牲动态性；(d) 读时派生三态、写时 clamp ±8，加断言保护间距。

3. **msgid 复用 vs 新增**：**每次发布用新 msgid（默认 (source, seqno) 即可），不复用、不改内容哈希**。(a) gossipsub 默认 (source,seqno) 让同一次发布的中继副本天然去重（at-least-once 兜底），正是所需；(b) 内容哈希作 msgid 两坑：不同车广播相同字节被误判同一消息（多发送者大忌）、有意重发被网络层误删；(c) 应用层身份用 (发送者, epoch/seq) 主键，"复用"无先例支持。

4. **是否补 anti-entropy**：**有条件维持"不做"，但强烈建议把已在广播的 30s FULL 反熵化（≈0 成本）**。
   - 反方：gossipsub 无投递保证（#197 十节点即丢）；Montresor 经典结论"rumor-mongering 无送达保证，反熵负责收敛"；**+3/−1 双向非单调，没有"无全量对账仍收敛"的定理**（收敛定理全部依赖单调/幂等：max-consensus、stigmergic 单调计数器、m-explore 的 cv::max）。
   - 正方：Ditto 论证幂等合并下丢包无害（但那是 state-based 全量语义）；BrickSync 实测持续广播缓解大部分网络问题；≤10 车同 LAN 丢包低且 FULL 已每 30s 广播。
   - 裁决：FULL 从"老节点忽略"改为"**按发送者替换其贡献表**"（内存 N×64KB 可忽略），即从概率兜底升为数学收敛，同时保持"无独立 pull 协议"。若坚持零改动：把收敛契约书面写成"投递无丢失前提下的 SEC + 有界分歧（±clamp）"，阈值穿越格加"长期未收到对端更新"的降级标记。**结论：乱序/重复/丢包下仍收敛的简单模型只有单调幂等合并（max/单调计数/每发送者读时 clamp）；双向加法没有。**

## 4. 风险补充（既有报告未覆盖）

1. **gossipsub 小规模实证丢包**（#197）：10 节点实测，非理论——需加丢包统计/检测。
2. **msgid 内容哈希误用**：多发送者同字节误判 + 有意重发被去重（见 §3.3）。
3. **clamp-阈值间距是动态性关键旋钮**：±8 vs ±6=2 单位；过窄→阈值抖动，过宽→动态障碍"钉死"。按环境动态度调，不拍脑袋。
4. **全量 64KB 帧撞 gossipsub 64KB 上限**（既有报告 P0）：补充证据——Intelligent Huffman 实测 RLE+Huffman 降通信 99%、mrgs 用 LZ4；**压缩是上线先决条件**。
5. **重启车 own 表归零**：own 表仅 64KB，建议落盘，否则重启动态下 merged 被"自我清零"。
6. **RTK 瞬间失效 → 错误格永久污染**：per-sender 表天然具备"踢表"能力；观测写入前加 RTK 质量门限。
7. **动态障碍 vs 幂等性冲突**：任何时间衰减都破坏幂等；建议动态障碍不进共享图（本地局部图跟踪），共享图只承载静态结构。
8. **FULL 的 O(N) 放大**：N=10 时 30s 全网放大 ≈6.5MB/30s；建议 FULL 走 RLE 压缩或"瓦片哈希摘要+按需补块"（QSP 先例）。

## 5. 参考文献 / 链接列表

- mrgs：https://github.com/gondsm/mrgs ；论文：https://link.springer.com/chapter/10.1007/978-3-030-45956-7_3
- Multi-Robot-Graph-SLAM：https://github.com/aserbremen/Multi-Robot-Graph-SLAM
- m-explore-ros2 map_merge 源码：https://github.com/robo-friends/m-explore-ros2
- Terrain-aware semantic mapping：https://www.frontiersin.org/journals/robotics-and-ai/articles/10.3389/frobt.2023.1249586/full
- QSP 四叉树同步：https://www.sciencedirect.com/science/article/pii/S1389128620313177 ；工件 https://github.com/phylib/QSPArtifacts
- Geo-CRDT：https://doi.org/10.3390/ijgi15070302 ；Geometry-Aware CRDTs：https://www.mdpi.com/2220-9964/14/12/468 ；mCRDT：https://github.com/njleonzhang/mCRDT
- CRDT Game State Sync in P2P VR：https://arxiv.org/abs/2503.17826
- Gaffer On Games 状态同步：https://gafferongames.com/post/state_synchronization/
- Ditto Delta State CRDTs：https://www.ditto.com/blog/an-inside-look-at-dittos-delta-state-crdts
- Montresor, Gossip and Epidemic Protocols：http://disi.unitn.it/~montreso/ds/papers/montresor17.pdf
- go-libp2p-pubsub #197（丢包实证）：https://github.com/libp2p/go-libp2p-pubsub/issues/197
- Intelligent Huffman（Sensors 2024）：https://pmc.ncbi.nlm.nih.gov/articles/PMC11124910/
- Rogers et al., Swarm 栅格建图：https://www.mdpi.com/2218-6581/12/3/70
- SYNAOS 三层地图管理（VDA 5050）：https://www.synaos.com/en/post/fleetmanagementt-shopfloor-layout-map-management
- NODE.maps：https://node-robotics.com/solutions/node-fleet-autonomy-services/nodemaps
- Sodhi et al. 2019, 一致性栅格 clamp：https://www.cs.cmu.edu/~kaess/pub/Sodhi19iros.pdf
- gossipsub 规范与 msgid：https://github.com/libp2p/specs/blob/master/pubsub/gossipsub/gossipsub-v1.0.md

---

### 一句话结论
本轮新增强信号：(a) **mrgs 证明分布式整张栅格共享+增量融合+压缩工程可行**；**m-explore 实际用 cv::max（单调幂等）而非 log-odds 加法**——"单调有界合并"的现成工程旁证；(b) QSP/Geo-CRDT 表明空间/瓦片数据同步在游戏与 GIS 侧已成熟可借鉴，但**栅格 CRDT 仍无先例**；(c) "不做反熵"成立前提是单调幂等合并或零丢包，双向 +3/−1 下应把 30s FULL 反熵化（≈0 成本）；(d) 周期 5s 起步、merged 保持 ±8、msgid 用默认 (source,seqno) 每次新增、全量先压缩（RLE/LZ4）再提频。
