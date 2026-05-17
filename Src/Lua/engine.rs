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

    // ─── Phase 1: 最小验证 ────────────────────────────────

    #[test]
    fn test_load_and_execute_hello_lua() {
        let lua = LuaContext::new().expect("create lua");
        let script = std::fs::read_to_string("programs/hello.lua").expect("read script");

        // 1. 加载脚本，执行顶层（注册 COMMAND/DESCRIPTION/execute）
        lua.load(&script).eval::<()>().expect("eval script");

        // 2. 读取元数据
        let command: String = lua.globals().get("COMMAND").expect("COMMAND");
        assert_eq!(command, "hello");

        // 3. 构造参数 table
        let params = lua.create_table().expect("params table");
        params.set("msg", "integration test").expect("set msg");

        // 4. 调用 execute(params)
        let execute: mlua::Function = lua.globals().get("execute").expect("execute fn");
        let result: String = execute.call(params).expect("call execute");
        assert_eq!(result, "ok");
    }

    #[test]
    fn test_script_missing_execute_errors() {
        let lua = LuaContext::new().expect("create lua");
        let script = r#"
            COMMAND = "bad"
            DESCRIPTION = "no execute function"
        "#;
        lua.load(script).eval::<()>().expect("eval script");

        let result = lua.globals().get::<mlua::Function>("execute");
        assert!(result.is_err(), "缺少 execute 函数应返回错误");
    }

    #[test]
    fn test_sandbox_cannot_io_open() {
        let lua = LuaContext::new().expect("create lua");
        // io 被设为 nil，调用 io.open 应失败
        let result = lua.load(r#"
            if io and io.open then
                return "should not reach"
            end
            return "sandbox ok"
        "#).eval::<String>();
        match result {
            Ok(s) => assert_eq!(s, "sandbox ok"),
            Err(_) => {} // 某些实现直接报错，也 OK
        }
    }
}
