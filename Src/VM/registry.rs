//Presented by KeJi
//Date ： 2026-05-18

//! 脚本注册表
//!
//! 管理两部分脚本：
//! - `builtin`：`programs/builtin/` 下的系统内置脚本，启动时扫描一次
//! - `user`：`programs/user/` 下的用户自定义脚本，启动时扫描，支持运行时 reload
//!
//! 用户脚本优先级高于内置脚本（同命令名时 user 覆盖 builtin）。

use mlua::Lua;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const BUILTIN_DIR: &str = "programs/builtin";
const USER_DIR: &str = "programs/user";

/// 单个 Lua 程序的元数据
#[derive(Debug, Clone)]
pub struct ProgramEntry {
    pub path: PathBuf,
    pub command: String,
    pub description: String,
}

/// 命令注册表：命令名 → 程序条目
#[derive(Debug)]
pub struct ProgramRegistry {
    builtin: HashMap<String, ProgramEntry>,
    user: HashMap<String, ProgramEntry>,
}

impl ProgramRegistry {
    /// 初始化：扫描 builtin 和 user 两个目录。
    pub fn new() -> mlua::Result<Self> {
        let builtin = Self::scan_dir(Path::new(BUILTIN_DIR));
        let user = Self::scan_dir(Path::new(USER_DIR));
        Ok(Self { builtin, user })
    }

    /// 重新扫描 user 目录（热加载），保留 builtin 不变。
    pub fn reload_user(&mut self) {
        self.user = Self::scan_dir(Path::new(USER_DIR));
    }

    /// 返回所有命令名（user 优先，去重）。
    pub fn command_names(&self) -> Vec<&String> {
        let mut names: Vec<&String> = self.builtin.keys().collect();
        for k in self.user.keys() {
            if !names.contains(&k) {
                names.push(k);
            }
        }
        names
    }

    /// 获取指定命令的条目（user 优先）。
    pub fn get(&self, command: &str) -> Option<&ProgramEntry> {
        self.user.get(command).or_else(|| self.builtin.get(command))
    }

    /// 仅在 user 注册表中查找（供 exec 命令使用）。
    pub fn get_user(&self, command: &str) -> Option<&ProgramEntry> {
        self.user.get(command)
    }

    /// 返回所有内置 Lua 脚本条目。
    pub fn builtin_entries(&self) -> Vec<ProgramEntry> {
        self.builtin.values().cloned().collect()
    }

    /// 返回所有用户 Lua 脚本条目。
    pub fn user_entries(&self) -> Vec<ProgramEntry> {
        self.user.values().cloned().collect()
    }

    // ─── 内部方法 ─────────────────────────────────────────

    /// 递归扫描指定目录，收集所有 .lua 脚本的元数据。
    fn scan_dir(dir: &Path) -> HashMap<String, ProgramEntry> {
        let mut map = HashMap::new();
        Self::collect_scripts(dir, &mut map);
        map
    }

    fn collect_scripts(dir: &Path, map: &mut HashMap<String, ProgramEntry>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::collect_scripts(&path, map);
            } else if path.extension().map_or(false, |ext| ext == "lua") {
                if let Some(entry) = Self::parse_script(&path) {
                    map.insert(entry.command.clone(), entry);
                }
            }
        }
    }

    fn parse_script(path: &Path) -> Option<ProgramEntry> {
        let script = std::fs::read_to_string(path).ok()?;
        let lua = Lua::new();
        lua.load(&script).eval::<()>().ok()?;

        let globals = lua.globals();
        let command: String = globals.get("COMMAND").ok()?;
        let description: String = globals.get("DESCRIPTION").unwrap_or_default();

        Some(ProgramEntry {
            path: path.to_path_buf(),
            command,
            description,
        })
    }
}

impl Default for ProgramRegistry {
    fn default() -> Self {
        Self {
            builtin: HashMap::new(),
            user: HashMap::new(),
        }
    }
}

// ============================================================
// 测试
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_registry() {
        let reg = ProgramRegistry::default();
        assert!(reg.command_names().is_empty());
        assert!(reg.get("unknown").is_none());
    }

    #[test]
    fn test_user_overrides_builtin() {
        let mut reg = ProgramRegistry::default();
        reg.builtin.insert(
            "cmd".into(),
            ProgramEntry {
                path: PathBuf::from("programs/builtin/cmd.lua"),
                command: "cmd".into(),
                description: "builtin version".into(),
            },
        );
        reg.user.insert(
            "cmd".into(),
            ProgramEntry {
                path: PathBuf::from("programs/user/cmd.lua"),
                command: "cmd".into(),
                description: "user version".into(),
            },
        );
        let entry = reg.get("cmd").expect("entry exists");
        assert_eq!(entry.description, "user version");
        assert_eq!(reg.command_names().len(), 1);
    }

    #[test]
    fn test_get_user_only() {
        let mut reg = ProgramRegistry::default();
        reg.builtin.insert(
            "pipeline".into(),
            ProgramEntry {
                path: PathBuf::from("programs/builtin/pipeline.lua"),
                command: "pipeline".into(),
                description: "builtin".into(),
            },
        );
        // get 能找到 builtin
        assert!(reg.get("pipeline").is_some());
        // get_user 找不到 builtin
        assert!(reg.get_user("pipeline").is_none());
        // get_user 找不到不存在的命令
        assert!(reg.get_user("nope").is_none());
    }
}
