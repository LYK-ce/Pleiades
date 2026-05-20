//Presented by KeJi
//Date ： 2026-05-18

//! StorageHandle — acquire_read / acquire_write 返回的 Lua UserData
//!
//! 持有路径和锁守卫。Lua 侧通过 `:path()` 获取磁盘路径，
//! 通过 `:release()` 提前释放锁（否则变量离开作用域时自动释放）。

use crate::storage::{ReadGuard, WriteGuard};
use std::path::PathBuf;

pub struct StorageReadHandle {
    path: PathBuf,
    guard: Option<ReadGuard>,
}

impl StorageReadHandle {
    pub fn new(path: PathBuf, guard: ReadGuard) -> Self {
        Self {
            path,
            guard: Some(guard),
        }
    }
}

impl mlua::UserData for StorageReadHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("path", |_, this, (): ()| {
            Ok(this.path.to_string_lossy().to_string())
        });
        methods.add_method_mut("release", |_, this, (): ()| {
            this.guard.take();
            Ok(())
        });
    }
}

pub struct StorageWriteHandle {
    path: PathBuf,
    guard: Option<WriteGuard>,
}

impl StorageWriteHandle {
    pub fn new(path: PathBuf, guard: WriteGuard) -> Self {
        Self {
            path,
            guard: Some(guard),
        }
    }
}

impl mlua::UserData for StorageWriteHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("path", |_, this, (): ()| {
            Ok(this.path.to_string_lossy().to_string())
        });
        methods.add_method_mut("release", |_, this, (): ()| {
            this.guard.take();
            Ok(())
        });
    }
}
