//Presented by KeJi
//Date ： 2026-04-30

//! TUI 应用状态管理模块
//!
//! 定义 App 结构体及相关数据类型，管理 TUI 的所有显示状态。
//! App 是 TUI 渲染的唯一数据源，所有 panel 通过读取 &App 获取数据。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use crate::llm_io::IoFrontend;
use crate::orchestrator::job::JobId;
use ratatui::layout::Rect;

// ============================================================
// 视图模式
// ============================================================

/// TUI 视图模式
///
/// 控制布局层数：Idle=3层, Busy=3层(Job变化), BusyCoordinator=4层(弹出Command)
#[derive(Debug, Clone, PartialEq)]
pub enum View_Mode {
    /// 空闲状态：Log + Network | Job(空闲) | 输入栏
    Idle,
    /// 忙碌状态（工作节点）：Log + Network | Job(任务信息) | 输入栏
    Busy,
    /// 忙碌状态（协调者）：Log + Network | Job(任务信息) | Command(推理输出) | 输入栏
    Busy_Coordinator,
}

// ============================================================
// Job 状态
// ============================================================

/// 文件传输方向
#[derive(Debug, Clone, PartialEq)]
pub enum Transfer_Direction {
    /// 发送
    Send,
    /// 接收
    Receive,
}

/// Job 显示区的内容状态
#[derive(Debug, Clone)]
pub enum Job_State {
    /// 空闲
    Idle,
    /// 文件传输（显示进度条）
    File_Transfer {
        direction: Transfer_Direction,
        file_name: String,
        peer: String,
        sent: u64,
        total: u64,
    },
    /// 推理/加载（不显示进度条，只显示文字信息）
    Inference {
        model_name: String,
        device_count: usize,
        layer_range: String,
        phase: String,
    },
}

// ============================================================
// Command 输出
// ============================================================

/// Command 显示区的内容
#[derive(Debug, Clone)]
pub struct Command_Output {
    /// 推理输出文本（逐 token 累积）
    pub output_text: String,
    /// 已生成 token 数
    pub token_count: usize,
    /// 生成速度 (tokens/sec)
    pub tok_per_sec: f64,
    /// 总耗时 (秒)
    pub total_secs: f64,
    /// 是否已完成
    pub completed: bool,
}

impl Command_Output {
    /// 创建新的空 Command 输出
    pub fn New() -> Self {
        Self {
            output_text: String::new(),
            token_count: 0,
            tok_per_sec: 0.0,
            total_secs: 0.0,
            completed: false,
        }
    }
}

// ============================================================
// 输入焦点
// ============================================================

/// 输入焦点枚举
///
/// 双输入框设计，Tab 键在两者间切换。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFocus {
    /// 系统命令输入框（最底部）
    Command,
    /// Prompt 输入框（Command Output 下方）
    Prompt,
}

// ============================================================
// Peer 显示信息
// ============================================================

/// 网络节点显示信息
#[derive(Debug, Clone)]
pub struct Peer_Display {
    /// 节点 ID（截断显示）
    pub peer_id: String,
    /// 连接状态
    pub connected: bool,
}

// ============================================================
// App 主结构体
// ============================================================

/// TUI 应用状态
///
/// 包含所有面板的数据，是 TUI 渲染的唯一数据源。
pub struct App {
    /// 当前视图模式
    pub view_mode: View_Mode,

    // ===== 全局状态 =====
    /// 当前使用的设备 ("cpu" 或 "cuda")
    pub device: String,

    // ===== Log 面板数据 =====
    /// 日志列表（带时间戳的字符串）
    pub logs: Vec<String>,
    /// 日志滚动偏移
    pub log_scroll: usize,

    // ===== Network 面板数据 =====
    /// 已知节点列表
    pub peers: Vec<Peer_Display>,

    // ===== Job 面板数据 =====
    /// 当前 Job 状态
    pub job: Job_State,

    // ===== Command 面板数据 =====
    /// 命令输出（始终存在，Command 面板始终可见）
    pub command_output: Command_Output,
    /// 命令输出滚动偏移
    pub command_scroll: usize,

    // ===== 命令输入栏数据 =====
    /// 命令输入缓冲区
    pub input_buffer: String,
    /// 命令光标位置
    pub cursor_position: usize,

    // ===== Prompt 输入栏数据 =====
    /// Prompt 输入缓冲区
    pub prompt_buffer: String,
    /// Prompt 光标位置
    pub prompt_cursor: usize,

    // ===== 焦点管理 =====
    /// 当前输入焦点
    pub focus: InputFocus,

    // ===== 推理会话 =====
    /// 当前活跃的 IoFrontend（单 session）
    pub active_frontend: Option<IoFrontend>,
    /// 当前活跃的 job_id
    pub active_job_id: Option<JobId>,

    // ===== 控制标志 =====
    /// 是否应退出
    pub should_quit: bool,

    // ===== 面板区域（用于鼠标滚轮命中检测） =====
    /// Log 面板区域
    pub log_area: Rect,
    /// Command 面板区域
    pub command_area: Rect,
}

impl App {
    /// 创建新的 App 实例
    pub fn New() -> Self {
        Self {
            view_mode: View_Mode::Idle,
            device: "cpu".to_string(),
            logs: Vec::new(),
            log_scroll: 0,
            peers: Vec::new(),
            job: Job_State::Idle,
            command_output: Command_Output::New(),
            command_scroll: 0,
            input_buffer: String::new(),
            cursor_position: 0,
            prompt_buffer: String::new(),
            prompt_cursor: 0,
            focus: InputFocus::Command,
            active_frontend: None,
            active_job_id: None,
            should_quit: false,
            log_area: Rect::default(),
            command_area: Rect::default(),
        }
    }

    /// 添加一条日志
    pub fn Add_Log(&mut self, msg: String) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        self.logs.push(format!("[{}] {}", timestamp, msg));
        // 自动滚动到底部
        let max_visible = 50; // 大致可见行数
        if self.logs.len() > max_visible {
            self.log_scroll = self.logs.len() - max_visible;
        }
    }

    /// 添加或更新节点
    pub fn Update_Peer(&mut self, peer_id: String, connected: bool) {
        if let Some(peer) = self.peers.iter_mut().find(|p| p.peer_id == peer_id) {
            peer.connected = connected;
        } else {
            self.peers.push(Peer_Display { peer_id, connected });
        }
    }

    /// 移除节点
    pub fn Remove_Peer(&mut self, peer_id: &str) {
        self.peers.retain(|p| p.peer_id != peer_id);
    }

    /// 日志向上滚动
    pub fn Scroll_Up(&mut self) {
        if self.log_scroll > 0 {
            self.log_scroll -= 1;
        }
    }

    /// 日志向下滚动
    pub fn Scroll_Down(&mut self) {
        if self.log_scroll < self.logs.len().saturating_sub(1) {
            self.log_scroll += 1;
        }
    }

    /// 命令面板向上滚动
    pub fn Scroll_Command_Up(&mut self) {
        if self.command_scroll > 0 {
            self.command_scroll -= 1;
        }
    }

    /// 命令面板向下滚动
    ///
    /// 不在此处设置上界——由 command_panel::Render 根据视觉行数钳位。
    pub fn Scroll_Command_Down(&mut self) {
        self.command_scroll = self.command_scroll.saturating_add(1);
    }

    /// 输入字符
    pub fn Input_Char(&mut self, c: char) {
        self.input_buffer.insert(self.cursor_position, c);
        self.cursor_position += c.len_utf8();
    }

    /// 删除字符（Backspace）
    pub fn Delete_Char(&mut self) {
        if self.cursor_position > 0 {
            // 找到前一个字符的边界
            let prev = self.input_buffer[..self.cursor_position]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input_buffer.remove(prev);
            self.cursor_position = prev;
        }
    }

    /// 清空输入
    pub fn Clear_Input(&mut self) {
        self.input_buffer.clear();
        self.cursor_position = 0;
    }

    /// 提取输入内容并清空
    pub fn Take_Input(&mut self) -> String {
        let input = self.input_buffer.clone();
        self.Clear_Input();
        input
    }

    // ===== Prompt 输入栏方法 =====

    /// Prompt 输入字符
    pub fn Prompt_Input_Char(&mut self, c: char) {
        self.prompt_buffer.insert(self.prompt_cursor, c);
        self.prompt_cursor += c.len_utf8();
    }

    /// Prompt 删除字符（Backspace）
    pub fn Prompt_Delete_Char(&mut self) {
        if self.prompt_cursor > 0 {
            let prev = self.prompt_buffer[..self.prompt_cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.prompt_buffer.remove(prev);
            self.prompt_cursor = prev;
        }
    }

    /// Prompt 清空输入
    pub fn Prompt_Clear(&mut self) {
        self.prompt_buffer.clear();
        self.prompt_cursor = 0;
    }

    /// Prompt 提取输入内容并清空
    pub fn Take_Prompt(&mut self) -> String {
        let input = self.prompt_buffer.clone();
        self.Prompt_Clear();
        input
    }

    /// 切换输入焦点
    pub fn Toggle_Focus(&mut self) {
        self.focus = match self.focus {
            InputFocus::Command => InputFocus::Prompt,
            InputFocus::Prompt => InputFocus::Command,
        };
    }

    /// 检查是否有活跃推理会话
    pub fn Has_Active_Session(&self) -> bool {
        self.active_frontend.is_some()
    }
}
