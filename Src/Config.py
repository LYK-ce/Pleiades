#Presented by KeJi
#Date : 2025-12-18

"""
配置组件
定义系统全局常量
"""

from typing import Final

# 日志记录标识符号常量
RUNTIME_SNAPSHOT: Final = 0b001
RUNTIME_LOAD: Final = 0b010
RUNTIME_EXECUTE: Final = 0b100
RUNTIME: Final = 0b111
