//Presented by KeJi
//Date ： 2026-05-18

//! StorageHandle — acquire_read / acquire_write 返回的 Lua UserData
//!
//! 持有路径和锁守卫。Lua 侧通过 `:path()` 获取磁盘路径；
//! Lua 变量释放时守卫自动 drop。

use crate::storage::{ReadGuard, WriteGuard};
use std::path::PathBuf;

pub struct StorageReadHandle {
    path: PathBuf,
    _guard: ReadGuard,
}

impl StorageReadHandle {
    pub fn new(path: PathBuf, guard: ReadGuard) -> Self {
        Self {
            path,
            _guard: guard,
        }
    }
}

impl mlua::UserData for StorageReadHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("path", |_, this, (): ()| {
            Ok(this.path.to_string_lossy().to_string())
        });
    }
}

pub struct StorageWriteHandle {
    path: PathBuf,
    _guard: WriteGuard,
}

impl StorageWriteHandle {
    pub fn new(path: PathBuf, guard: WriteGuard) -> Self {
        Self {
            path,
            _guard: guard,
        }
    }
}

impl mlua::UserData for StorageWriteHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("path", |_, this, (): ()| {
            Ok(this.path.to_string_lossy().to_string())
        });
    }
}
