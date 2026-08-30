//Presented by KeJi
//Date ： 2026-05-21

//! LuaTensor 的 mlua UserData 绑定
//!
//! 将 `crate::ml_engine::lua_tensor::LuaTensor` 的方法暴露给 Lua。
//! 注意：此文件仅负责 Lua 绑定层，LuaTensor 结构体定义及纯 Rust 逻辑
//! 位于 `crate::ml_engine::lua_tensor`。

use mlua::{UserData, UserDataMethods};
use crate::ml_engine::lua_tensor::LuaTensor;

impl UserData for LuaTensor {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("dims", |lua, this, (): ()| {
            let dims = this.dims();
            let t = lua.create_table()?;
            for (i, &d) in dims.iter().enumerate() {
                t.set(i + 1, d)?;
            }
            Ok(t)
        });

        methods.add_method("to_bytes", |lua, this, (): ()| {
            let bytes = this.to_bytes().map_err(|e| mlua::Error::runtime(e))?;
            lua.create_string(&bytes)
                .map_err(|e| mlua::Error::runtime(e.to_string()))
        });

        methods.add_method("to_device", |_, this, device_str: String| {
            this.to_device_str(&device_str)
                .map_err(|e| mlua::Error::runtime(e))
        });
    }
}
