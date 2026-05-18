//Presented by KeJi
//Date ： 2026-05-17

//! Lua 集成测试
//!
//! 验证 Lua 脚本通过 caps 函数表调用 Rust 能力函数的端到端通路。

use pleiades::lua::engine::LuaContext;
use pleiades::lua::capability_binding::register_caps;
use std::sync::Arc;

#[test]
fn test_hello_script_loads_and_executes() {
    let lua = LuaContext::new().expect("create lua");
    let script = std::fs::read_to_string("programs/user/hello.lua").expect("read script");

    lua.load(&script).eval::<()>().expect("eval script");

    // hello.lua 调用 caps.print，需注册 logging caps
    let bus = Arc::new(pleiades::event_bus::EventBus::New(4));
    pleiades::lua::capability_binding::register_logging_caps(&lua, bus)
        .expect("register logging caps");

    let command: String = lua.globals().get("COMMAND").expect("COMMAND");
    assert_eq!(command, "hello");

    let params = lua.create_table().expect("params");
    params.set("msg", "integration").expect("set msg");

    let execute: mlua::Function = lua.globals().get("execute").expect("execute");
    let result: String = execute.call(params).expect("call execute");
    assert_eq!(result, "ok");
}

#[test]
fn test_hello_script_with_caps() {
    let lua = LuaContext::new().expect("create lua");
    let script = std::fs::read_to_string("programs/user/hello.lua").expect("read script");

    lua.load(&script).eval::<()>().expect("eval script");
    register_caps(&lua).expect("register caps");

    let bus = Arc::new(pleiades::event_bus::EventBus::New(4));
    pleiades::lua::capability_binding::register_logging_caps(&lua, bus)
        .expect("register logging caps");

    let params = lua.create_table().expect("params");
    let execute: mlua::Function = lua.globals().get("execute").expect("execute");
    let result: String = execute.call(params).expect("call execute");
    assert_eq!(result, "ok");
}

#[test]
fn test_custom_script_calls_caps_functions() {
    let lua = LuaContext::new().expect("create lua");
    register_caps(&lua).expect("register caps");

    // 模拟业务脚本：调用多个 caps 函数
    let script = r#"
        COMMAND = "demo"
        DESCRIPTION = "演示脚本：测试多函数调用"

        function execute(params)
            local echo = caps.echo(params.msg)
            local sum = caps.add(params.a, params.b)
            return echo .. ":" .. tostring(sum)
        end
    "#;
    lua.load(script).eval::<()>().expect("eval script");

    let cmd: String = lua.globals().get("COMMAND").expect("COMMAND");
    assert_eq!(cmd, "demo");

    let params = lua.create_table().expect("params");
    params.set("msg", "test").expect("msg");
    params.set("a", 10).expect("a");
    params.set("b", 20).expect("b");

    let execute: mlua::Function = lua.globals().get("execute").expect("execute");
    let result: String = execute.call(params).expect("call execute");
    assert_eq!(result, "test:30.0");
}

#[tokio::test]
async fn test_async_caps_from_script() {
    let lua = LuaContext::new().expect("create lua");
    register_caps(&lua).expect("register caps");

    let script = r#"
        COMMAND = "ping_test"
        function execute(params)
            return caps.ping()
        end
    "#;
    lua.load(script).eval::<()>().expect("eval script");

    let execute: mlua::Function = lua.globals().get("execute").expect("execute");
    let result: String = execute.call_async(()).await.expect("call execute");
    assert_eq!(result, "pong");
}
