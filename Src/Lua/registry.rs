//Presented by KeJi
//Date ： 2026-05-13

//! 脚本注册表
//!
//! 启动时扫描 `programs/` 目录，从 Lua 脚本的顶层变量
//! `COMMAND` 和 `DESCRIPTION` 构建命令注册表。

use mlua::Lua;
use std::collections::HashMap;
use std::path::PathBuf;

/// 单个 Lua 程序的元数据
#[derive(Debug, Clone)]
pub struct ProgramEntry {
    pub path: PathBuf,
    pub command: String,
    pub description: String,
}

/// 命令注册表：命令名 → 程序条目
#[derive(Debug, Default)]
pub struct ProgramRegistry {
    programs: HashMap<String, ProgramEntry>,
}

impl ProgramRegistry {
    /// 扫描 `programs/` 目录，收集所有 .lua 脚本的元数据。
    pub fn scan() -> mlua::Result<Self> {
        let mut map = HashMap::new();
        let dir = PathBuf::from("programs");

        if !dir.exists() {
            return Ok(Self { programs: map });
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return Ok(Self { programs: map }),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |ext| ext != "lua") {
                continue;
            }

            let script = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let lua = Lua::new();
            if lua.load(&script).eval::<()>().is_err() {
                continue;
            }

            let globals = lua.globals();
            let command: String = match globals.get("COMMAND") {
                Ok(c) => c,
                Err(_) => continue,
            };
            let description: String = globals.get("DESCRIPTION").unwrap_or_default();

            map.insert(
                command.clone(),
                ProgramEntry {
                    path: path.clone(),
                    command,
                    description,
                },
            );
        }

        Ok(Self { programs: map })
    }

    /// 返回所有命令名
    pub fn command_names(&self) -> Vec<&String> {
        self.programs.keys().collect()
    }

    /// 获取指定命令的条目
    pub fn get(&self, command: &str) -> Option<&ProgramEntry> {
        self.programs.get(command)
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
        let reg = ProgramRegistry {
            programs: HashMap::new(),
        };
        assert!(reg.command_names().is_empty());
        assert!(reg.get("unknown").is_none());
    }
}
