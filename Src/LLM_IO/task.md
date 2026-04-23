1. 根据llm_io_module_design.md，实现对应代码并加入到lib.rs当中
   有几个小问题需要修改：broker.rs 中 HashMap::insert 对同一 job_id 会直接静默覆盖旧条目，如果旧 Job 仍在运行，它的通道虽然物理上还存在（因为前端/ML 侧可能还持有句柄），但 Broker 的索引里已经查不到了。如果存在同一条目应该返回失败。InvalidJobId未使用，可以移除。如果某个线程在持有锁时 panic，锁会中毒（poisoned），后续所有 lock() 都会 panic。改用 tokio::sync::Mutex（虽然性能稍差，但无中毒问题）。当前 ChannelEntry 保存两份 Sender 克隆，目的是让 deallocate 时通过 drop 它们来加速通道关闭，如果前端和 ML 侧都还在运行，deallocate 只是从 Broker 索引移除，不会强制关闭它们之间的通道，在文档中明确说明："仅清理内部索引，通道的实际生命周期由两端句柄的 Drop 决定"
   完成

