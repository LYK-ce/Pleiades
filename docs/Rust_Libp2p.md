#Presented by KeJi
#Date: 2026-01-14

# Rust libp2p 功能使用指南

## 概述

libp2p是一个模块化的网络协议栈，用于构建P2P应用。Rust实现：`libp2p` crate。

文档：https://docs.rs/libp2p/latest/libp2p/

## 依赖配置

```toml
[dependencies]
libp2p = { version = "0.54", features = [
    "tokio",
    "tcp",
    "quic",
    "noise",
    "yamux",
    "mdns",
    "kad",
    "request-response",
    "identify",
    "macros"
]}
tokio = { version = "1", features = ["full"] }
```

## 核心模块

### 1. 节点发现与组网

#### 1.1 mDNS (局域网发现)

```rust
use libp2p::mdns;

// 创建mDNS行为
let mdns = mdns::tokio::Behaviour::new(
    mdns::Config::default(),
    local_peer_id
)?;

// 事件处理
match event {
    mdns::Event::Discovered(peers) => {
        for (peer_id, addr) in peers {
            // 发现新节点，可加入Kademlia
            swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
        }
    }
    mdns::Event::Expired(peers) => {
        // 节点离开
    }
}
```

#### 1.2 Kademlia DHT (广域网发现)

```rust
use libp2p::kad::{self, store::MemoryStore};

// 创建Kademlia行为
let store = MemoryStore::new(local_peer_id);
let kademlia = kad::Behaviour::new(local_peer_id, store);

// 引导节点
kademlia.add_address(&bootstrap_peer_id, bootstrap_addr);
kademlia.bootstrap()?;

// 查找节点
kademlia.get_closest_peers(target_peer_id);
```

### 2. DHT功能

Kademlia DHT提供分布式存储：

```rust
// 存储键值对
let key = kad::RecordKey::new(&"my_key");
let record = kad::Record {
    key: key.clone(),
    value: b"my_value".to_vec(),
    publisher: Some(local_peer_id),
    expires: None,
};
kademlia.put_record(record, kad::Quorum::One)?;

// 查询键值对
kademlia.get_record(key);

// 事件处理
match event {
    kad::Event::OutboundQueryProgressed { result, .. } => {
        match result {
            kad::QueryResult::GetRecord(Ok(ok)) => {
                for peer_record in ok.records {
                    // 获取到记录
                }
            }
            kad::QueryResult::PutRecord(Ok(_)) => {
                // 存储成功
            }
            _ => {}
        }
    }
    _ => {}
}
```

### 3. 建立连接

#### 3.1 Transport层配置

```rust
use libp2p::{tcp, quic, noise, yamux, Transport};

// TCP + Noise加密 + Yamux多路复用
let tcp_transport = tcp::tokio::Transport::new(tcp::Config::default())
    .upgrade(libp2p::core::upgrade::Version::V1)
    .authenticate(noise::Config::new(&keypair)?)
    .multiplex(yamux::Config::default())
    .boxed();

// QUIC (内置加密和多路复用)
let quic_transport = quic::tokio::Transport::new(quic::Config::new(&keypair));

// 组合传输
let transport = tcp_transport.or_transport(quic_transport).boxed();
```

#### 3.2 Swarm创建

```rust
use libp2p::{Swarm, SwarmBuilder, identity};

let keypair = identity::Keypair::generate_ed25519();
let local_peer_id = keypair.public().to_peer_id();

let swarm = SwarmBuilder::with_existing_identity(keypair)
    .with_tokio()
    .with_tcp(
        tcp::Config::default(),
        noise::Config::new,
        yamux::Config::default,
    )?
    .with_quic()
    .with_behaviour(|key| MyBehaviour::new(key))?
    .build();

// 监听地址
swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?)?;

// 主动连接
swarm.dial(target_addr)?;
```

### 4. 传输命令 (Request-Response)

```rust
use libp2p::request_response::{self, ProtocolSupport, Codec};
use libp2p::StreamProtocol;

// 定义协议
#[derive(Debug, Clone)]
pub struct CommandCodec;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommandRequest {
    pub cmd: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommandResponse {
    pub success: bool,
    pub result: String,
}

// 实现Codec trait
impl Codec for CommandCodec {
    type Protocol = StreamProtocol;
    type Request = CommandRequest;
    type Response = CommandResponse;
    
    // read_request, read_response, write_request, write_response 实现...
}

// 创建行为
let protocols = [(StreamProtocol::new("/pleiades/cmd/1.0.0"), ProtocolSupport::Full)];
let cfg = request_response::Config::default();
let request_response = request_response::Behaviour::<CommandCodec>::new(protocols, cfg);

// 发送请求
let request = CommandRequest { cmd: "status".into(), args: vec![] };
let request_id = request_response.send_request(&peer_id, request);

// 响应处理
match event {
    request_response::Event::Message { peer, message } => {
        match message {
            request_response::Message::Request { request, channel, .. } => {
                // 处理请求
                let response = CommandResponse { success: true, result: "ok".into() };
                request_response.send_response(channel, response)?;
            }
            request_response::Message::Response { response, .. } => {
                // 处理响应
            }
        }
    }
    _ => {}
}
```

### 5. 传输文件

利用Request-Response传输分块文件：

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum FileRequest {
    Info { file_id: String },
    Chunk { file_id: String, offset: u64, length: u32 },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum FileResponse {
    Info { file_id: String, size: u64, hash: Vec<u8> },
    Chunk { offset: u64, data: Vec<u8> },
    Error { msg: String },
}

// 文件传输流程
// 1. 请求文件信息
let req = FileRequest::Info { file_id: file_id.clone() };
behaviour.send_request(&peer, req);

// 2. 分块请求 (根据info响应的size)
const CHUNK_SIZE: u32 = 65536; // 64KB
for offset in (0..file_size).step_by(CHUNK_SIZE as usize) {
    let req = FileRequest::Chunk { 
        file_id: file_id.clone(), 
        offset, 
        length: CHUNK_SIZE 
    };
    behaviour.send_request(&peer, req);
}

// 3. 接收端处理
match request {
    FileRequest::Chunk { file_id, offset, length } => {
        let mut file = File::open(&file_path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; length as usize];
        let n = file.read(&mut buf)?;
        buf.truncate(n);
        FileResponse::Chunk { offset, data: buf }
    }
    // ...
}
```

## 组合行为

```rust
use libp2p::swarm::NetworkBehaviour;

#[derive(NetworkBehaviour)]
pub struct PleiadesNetwork {
    pub mdns: mdns::tokio::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub identify: identify::Behaviour,
    pub request_response: request_response::Behaviour<CommandCodec>,
    pub file_transfer: request_response::Behaviour<FileCodec>,
}
```

## 主循环

```rust
loop {
    tokio::select! {
        event = swarm.select_next_some() => {
            match event {
                SwarmEvent::Behaviour(PleiadesNetworkEvent::Mdns(e)) => { /* ... */ }
                SwarmEvent::Behaviour(PleiadesNetworkEvent::Kademlia(e)) => { /* ... */ }
                SwarmEvent::Behaviour(PleiadesNetworkEvent::RequestResponse(e)) => { /* ... */ }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => { /* ... */ }
                SwarmEvent::ConnectionClosed { peer_id, .. } => { /* ... */ }
                _ => {}
            }
        }
    }
}
```

## 功能模块总结

| 需求 | libp2p模块 | 说明 |
|------|-----------|------|
| 局域网发现 | `mdns` | 自动发现局域网节点 |
| 广域网发现 | `kad` | DHT节点发现 |
| DHT存储 | `kad` | 分布式键值存储 |
| 加密传输 | `noise` | 端到端加密 |
| 多路复用 | `yamux` | 单连接多流 |
| 命令传输 | `request-response` | 请求-响应模式 |
| 文件传输 | `request-response` | 分块传输 |
| 节点标识 | `identify` | 交换节点信息 |
