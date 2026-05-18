//Presented by KeJi
//Date ： 2026-05-18

//! LuaTensor — candle_core::Tensor 的 mlua UserData 包装
//!
//! 使 Tensor 可以安全地跨 Lua 边界传递，Lua 脚本中表现为 userdata 对象。
//! 通过 Deref 自动解引用，Rust 侧使用时零开销。

use candle_core::Tensor;
use std::ops::Deref;

/// Tensor 的 Lua 包装。
///
/// 内存开销：Tensor 内部是 `Arc<Storage>`，LuaTensor 仅多一层 newtype，
/// 构造和传递均为指针复制，无数据拷贝。
pub struct LuaTensor(pub Tensor);

impl Deref for LuaTensor {
    type Target = Tensor;

    fn deref(&self) -> &Tensor {
        &self.0
    }
}

impl mlua::UserData for LuaTensor {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // 查询维度，Lua 侧用于调试
        methods.add_method("dims", |lua, this, (): ()| {
            let dims = this.dims();
            let t = lua.create_table()?;
            for (i, &d) in dims.iter().enumerate() {
                t.set(i + 1, d)?;
            }
            Ok(t)
        });
    }
}
