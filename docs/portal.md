更简化的方案：Portal 退化为 Core 的内部方法层
不保留 Portal 结构体，也不保留 CoreAction 枚举。Core 自己持有路由所需的工具（Compiler、IoBroker），命令路由作为 Core 的私有方法存在。代码可以放在 portal.rs 里（通过 impl Core 块），但运行时没有 Portal 对象。
Core 结构体直接持有：
rust
复制
pub struct Core {
    // 内核状态
    registry: HashMap<JobId, JobHandle>,
    shutting_down: bool,
    
    // 路由工具（原 Portal 的职责）
    compiler: Arc<Compiler>,
    io_broker: Arc<IoBroker>,
    capabilities: Arc<Capabilities>,
    
    // 通道
    user_cmd_rx: mpsc::Receiver<UserCommand>,
    network_cmd_rx: mpsc::Receiver<NetworkCommand>,
    lifecycle_rx: mpsc::Receiver<LifecycleEvent>,
}
Core 主循环保持极简：
impl Core {
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                // 只有通道接收是异步的
                Some(cmd) = self.user_cmd_rx.recv(), if !self.shutting_down => {
                    self.route_user(cmd);  // 同步调用，无 await
                }
                Some(cmd) = self.network_cmd_rx.recv(), if !self.shutting_down => {
                    self.route_network(cmd);  // 同步调用，无 await
                }
                Some(event) = self.lifecycle_rx.recv() => {
                    self.handle_lifecycle_event(event);  // 同步调用
                }
            }
        }
    }

    // 同步方法！
    fn route_user(&mut self, cmd: UserCommand) {
        match cmd {
            UserCommand::Run { model_path } => {
                let program = self.compiler.compile_run(&model_path);
                let io = self.io_broker.allocate();
                self.spawn_job(JobKind::Run, program, io);
            }
            UserCommand::Cancel { job_id } => {
                self.cancel_job(job_id);
            }
            UserCommand::Quit => {
                self.shutdown();
            }
            UserCommand::DisplayPeer => {
                let peers = self.capabilities.network.list_peers();
                self.capabilities.ui.display(peers);
            }
            UserCommand::SetDevice { device } => {
                self.capabilities.compute.set_preference(device);
            }
        }
    }

    // 同步方法！
    fn spawn_job(&mut self, kind: JobKind, program: (), io: ()) {
        let job_id = JobId(generate_id());
        let cancel = CancellationToken::new();
        
        // 创建 Executor，但不 await 它！
        let executor = JobExecutor::new(job_id, kind, program, io, cancel.clone(), ...);
        tokio::spawn(executor.run());  // 同步 spawn，立即返回
        
        self.registry.insert(job_id, JobHandle { kind, cancel });
    }

    fn cancel_job(&mut self, job_id: JobId) {
        if let Some(handle) = self.registry.get(&job_id) {
            handle.cancel.cancel();  // 同步发信号
        }
    }

    fn shutdown(&mut self) {
        self.shutting_down = true;
        for handle in self.registry.values() {
            handle.cancel.cancel();  // 同步发信号
        }
    }

    fn handle_lifecycle_event(&mut self, event: LifecycleEvent) {
        match event {
            LifecycleEvent::Done { job_id, .. } => {
                self.registry.remove(&job_id);  // 同步 HashMap 操作
            }
        }
    }
}