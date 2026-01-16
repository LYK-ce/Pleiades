#Presented by KeJi
#Date: 2026-01-14

# Rust 日志模块实现方案

## 一、日志库选型

| 库 | 特点 | 适用场景 |
|----|------|----------|
| `log` + `env_logger` | 简单门面模式 | 同步程序 |
| `tracing` + `tracing-subscriber` | 结构化日志+异步追踪 | **异步/P2P程序** ✓ |

**推荐**：`tracing` 生态，原因：
1. 原生支持异步span追踪
2. libp2p内部已使用tracing
3. 结构化日志便于分析

## 二、依赖配置

```toml
# Cargo.toml
[dependencies]
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

## 三、初始化

### 3.1 基础初始化（控制台输出）

```rust
use tracing_subscriber::{fmt, EnvFilter};

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env()  // RUST_LOG环境变量
            .add_directive("pleiades=debug".parse().unwrap())
            .add_directive("libp2p=info".parse().unwrap()))
        .init();
}
```

### 3.2 生产环境（JSON格式+文件）

```rust
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use std::fs::File;

fn init_logging_prod() {
    let file = File::create("pleiades.log").unwrap();
    
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt::layer().json().with_writer(file))
        .init();
}
```

## 四、使用方式

### 4.1 日志宏

```rust
use tracing::{info, warn, error, debug, trace, instrument};

// 基本日志
info!("节点启动");
warn!(peer_id = %peer, "连接断开");
error!(?err, "推理失败");

// 结构化字段
debug!(
    model_id = %model,
    layers = ?0..10,
    "加载模型"
);
```

### 4.2 异步Span追踪

```rust
#[instrument(skip(swarm), fields(peer_id = %peer_id))]
async fn handle_request(swarm: &mut Swarm<_>, peer_id: PeerId, req: Request) {
    info!("处理请求");
    // span自动关联所有内部日志
    let result = process(req).await;
    info!(status = ?result, "请求完成");
}
```

### 4.3 模块级日志控制

```bash
# 运行时控制
RUST_LOG=pleiades=debug,libp2p_kad=warn,ort=error cargo run
```

## 五、日志级别规范

| 级别 | 用途 |
|------|------|
| `error` | 严重错误，需立即处理 |
| `warn` | 警告，可能影响功能 |
| `info` | 重要状态变化 |
| `debug` | 调试信息 |
| `trace` | 详细追踪 |

## 六、项目集成

```rust
// src/main.rs
mod logging;

#[tokio::main]
async fn main() {
    logging::init();
    
    info!("Pleiades 启动");
    // ...
}
```

```rust
// src/logging.rs
use tracing_subscriber::{fmt, EnvFilter};

pub fn init() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("pleiades=info,libp2p=warn"))
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();
}
```

## 七、总结

| 项目 | 选择 |
|------|------|
| 核心库 | `tracing` + `tracing-subscriber` |
| 输出格式 | 开发：pretty | 生产：JSON |
| 级别控制 | `RUST_LOG` 环境变量 |
| 异步追踪 | `#[instrument]` 宏 |
