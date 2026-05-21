#Presented by KeJi
#Date: 2026-01-13

# Rust 环境配置指南

## 1. 安装 Rust

### Windows

```powershell
# 下载并运行安装程序
https://win.rustup.rs/

# 或使用 winget
winget install Rustlang.Rustup
```

安装过程中选择默认选项即可。

### Linux / macOS

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## 2. 环境变量

安装完成后，确保以下路径在 PATH 中：

```
Windows: %USERPROFILE%\.cargo\bin
Linux/macOS: ~/.cargo/bin
```

**重启终端**使环境变量生效。

## 3. 验证安装

```bash
rustc --version    # 编译器版本
cargo --version    # 包管理器版本
rustup --version   # 工具链管理器版本
```

## 4. 配置国内镜像 (可选)

创建/编辑 `~/.cargo/config.toml` (Windows: `%USERPROFILE%\.cargo\config.toml`):

```toml
[source.crates-io]
replace-with = 'ustc'

[source.ustc]
registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"
```

## 5. 安装常用组件

```bash
# 代码格式化
rustup component add rustfmt

# 代码检查
rustup component add clippy

# 语言服务器 (IDE支持)
rustup component add rust-analyzer
```

## 6. IDE 配置

### VS Code

安装扩展: `rust-analyzer`

### 其他 IDE

- IntelliJ IDEA: 安装 Rust 插件
- CLion: 内置 Rust 支持

## 7. 创建测试项目

```bash
cargo new hello_rust
cd hello_rust
cargo run
```

输出 `Hello, world!` 表示环境配置成功。

## 8. P2PLLM 所需依赖

项目将使用以下 crate，无需手动安装（Cargo 自动处理）：

| Crate | 用途 |
|-------|------|
| tokio | 异步运行时 |
| libp2p | P2P 网络 |
| candle-core | 张量计算 |
| candle-transformers | Transformer 模型 |
| safetensors | 模型加载 |
| serde | 序列化 |
