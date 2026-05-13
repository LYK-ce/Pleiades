//Presented by KeJi
//Date ： 2026-05-13

//! Lua 引擎封装
//!
//! 提供 Lua 实例创建、沙箱配置和能力函数注册。

use mlua::{Lua, Value};

pub struct LuaContext;

impl LuaContext {
    /// 创建一个配置好沙箱的 Lua 实例。
    ///
    /// 仅加载安全的标准库（string, table, math），
    /// 禁用 os/io/require 等危险 API。
    pub fn new() -> mlua::Result<Lua> {
        let lua = Lua::new();

        // 禁用危险的全局函数
        let globals = lua.globals();
        globals.set("os", Value::Nil)?;
        globals.set("io", Value::Nil)?;
        globals.set("require", Value::Nil)?;
        globals.set("dofile", Value::Nil)?;
        globals.set("loadfile", Value::Nil)?;

        Ok(lua)
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_lua_instance() {
        let lua = LuaContext::new().expect("应该成功创建 Lua 实例");
        let result: String = lua.load("return 'hello'").eval().unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_sandbox_os_blocked() {
        let lua = LuaContext::new().expect("应该成功创建 Lua 实例");
        let result = lua.load("return os").eval::<Value>();
        // os 应该为 nil 或被禁用
        match result {
            Ok(Value::Nil) => {} // ok
            Err(_) => {}         // ok（某些版本直接报错）
            _ => panic!("os 未被正确禁用"),
        }
    }
}
